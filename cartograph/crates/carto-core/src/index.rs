//! Index build orchestration (M1 full build).
//!
//! Walk the repo, parse each file, write symbols/skeletons/refs, resolve edges,
//! then PageRank. This is the `carto index` code path. Incremental `apply()`
//! (`10-incremental-sync.md`) is a later milestone; v1 does full builds.

use crate::pagerank;
use anyhow::{Context, Result};
use carto_store::Store;
use ignore::WalkBuilder;
use std::path::Path;

/// Files larger than this are recorded at Tier-0 only (skip symbol extraction).
const MAX_PARSE_BYTES: u64 = 1_500_000;

#[derive(Debug, Default, Clone)]
pub struct BuildStats {
    pub files_indexed: usize,
    pub files_parsed: usize,
    pub symbols: usize,
    pub edges_resolved: usize,
    pub cochange_pairs: usize,
    pub generation: i64,
}

/// Full build of `root` into the SQLite index at `db_path`.
pub fn build_index(root: &Path, db_path: &Path) -> Result<BuildStats> {
    let mut store = Store::open(db_path).context("open index store")?;
    let generation = store.bump_generation().context("bump generation")?;
    let mut stats = BuildStats {
        generation,
        ..Default::default()
    };

    let walk = WalkBuilder::new(root)
        .hidden(false) // index dotfiles too, but .gitignore still applies
        .git_ignore(true)
        .git_global(false)
        .build();

    for dent in walk {
        let dent = match dent {
            Ok(d) => d,
            Err(_) => continue,
        };
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let abs = dent.path();
        // skip the index itself / target dirs defensively
        let rel = match abs.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if rel.starts_with(".carto/") || rel.contains("/target/") || rel.starts_with("target/") {
            continue;
        }

        let meta = match std::fs::metadata(abs) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();

        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let hash = blake3::hash(&bytes);
        let lang = carto_parse::detect_lang(&rel);

        // Parse only known languages within the size budget and valid UTF-8.
        let parsed = match (&lang, size <= MAX_PARSE_BYTES, std::str::from_utf8(&bytes)) {
            (Some(_), true, Ok(src)) => {
                let p = carto_parse::parse_file(&rel, src);
                stats.files_parsed += 1;
                p
            }
            _ => carto_model::ParsedFile::default(),
        };

        store
            .upsert_file(
                &rel,
                lang.as_deref(),
                parsed.purpose.as_deref(),
                size as i64,
                hash.as_bytes(),
                &parsed.symbols,
                &parsed.refs,
                generation,
            )
            .with_context(|| format!("upsert {rel}"))?;
        stats.files_indexed += 1;
        stats.symbols += parsed.symbols.len();
    }

    // H2: resolve references into edges, then rank.
    stats.edges_resolved = store.resolve_edges().context("resolve edges")?;
    let nodes = store.all_symbol_ids()?;
    let edges = store.all_call_edges()?;
    let ranks = pagerank::pagerank(&nodes, &edges, 0.85, 30);
    store.write_ranks(&ranks).context("write ranks")?;

    // H3: mine git co-change into file_cochange (no-op if not a git repo).
    stats.cochange_pairs = carto_git::mine_cochange(root, &mut store).unwrap_or(0);

    Ok(stats)
}
