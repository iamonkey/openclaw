//! Incremental sync (H4 / `10-incremental-sync.md`).
//!
//! Turns the index from "full rebuild" into "reparse only what changed". The
//! single-file path (`sync_paths`) is the interactive hot path the SO latency
//! gate targets (edit p95 <= 200 ms): hash-gate, reparse the dirty file only,
//! incrementally re-resolve just the affected edges, bump the generation, and
//! mark rank dirty. PageRank is **not** recomputed in the hot path — it is
//! idle-amortized via `recompute_ranks` (doc 10 §5: "mark-dirty + periodic full
//! recompute on idle"). Co-change (H3) is history-derived and unchanged by a
//! working-tree edit, so it is left alone.

use anyhow::{Context, Result};
use carto_store::Store;
use ignore::WalkBuilder;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

const MAX_PARSE_BYTES: u64 = 1_500_000;

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub changed: usize,
    pub added: usize,
    pub deleted: usize,
    pub skipped_clean: usize,
    pub generation: i64,
    pub parse_ms: f64,
    pub resolve_ms: f64,
    pub total_ms: f64,
    pub rank_dirty: bool,
}

/// Incrementally sync a specific set of repo-relative paths (the watcher/edit
/// hot path). Unchanged files (hash match) are skipped; missing files are
/// deleted from the index. Bumps the generation once for the whole batch.
pub fn sync_paths(root: &Path, db: &Path, rels: &[String]) -> Result<SyncReport> {
    let mut store = Store::open(db)?;
    let generation = store.bump_generation()?;
    let mut report = SyncReport {
        generation,
        rank_dirty: true,
        ..Default::default()
    };
    let t0 = Instant::now();
    for rel in rels {
        sync_one(&mut store, root, rel, generation, &mut report)?;
    }
    store.set_meta("rank_dirty", "1")?;
    report.total_ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

/// Whole-repo incremental sync: scan the tree, reparse only files whose content
/// hash changed (or are new), delete files that vanished, skip the rest.
pub fn sync(root: &Path, db: &Path) -> Result<SyncReport> {
    // Discover on-disk files (same walk policy as the full build).
    let mut on_disk: BTreeSet<String> = BTreeSet::new();
    let walk = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .build();
    for dent in walk.flatten() {
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Ok(rel) = dent.path().strip_prefix(root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.starts_with(".carto/") || rel.contains("/target/") || rel.starts_with("target/")
            {
                continue;
            }
            on_disk.insert(rel);
        }
    }

    let mut store = Store::open(db)?;
    let indexed: BTreeSet<String> = store.list_files()?.into_iter().map(|f| f.path).collect();
    let generation = store.bump_generation()?;
    let mut report = SyncReport {
        generation,
        rank_dirty: true,
        ..Default::default()
    };
    let t0 = Instant::now();

    // Deletions: indexed but no longer on disk.
    for gone in indexed.difference(&on_disk) {
        if store.delete_file(gone)? {
            report.deleted += 1;
        }
    }
    // Adds + changes: every on-disk file (sync_one hash-gates the clean ones).
    for rel in &on_disk {
        sync_one(&mut store, root, rel, generation, &mut report)?;
    }

    store.set_meta("rank_dirty", "1")?;
    report.total_ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

/// Sync one path: hash-gate, reparse if dirty, upsert, incrementally resolve.
fn sync_one(
    store: &mut Store,
    root: &Path,
    rel: &str,
    generation: i64,
    report: &mut SyncReport,
) -> Result<()> {
    let abs = root.join(rel);
    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(_) => {
            // Vanished since discovery: treat as delete.
            if store.delete_file(rel)? {
                report.deleted += 1;
            }
            return Ok(());
        }
    };
    let hash = blake3::hash(&bytes);

    let existing = store.file_by_path(rel)?;
    if let Some(f) = &existing {
        if f.content_hash.as_slice() == hash.as_bytes().as_slice() {
            report.skipped_clean += 1;
            return Ok(());
        }
    }

    // Names this file defined *before* the edit (for incoming-edge rebuild).
    let old_names = match &existing {
        Some(f) => store.symbol_names_in_file(f.id)?,
        None => Vec::new(),
    };

    let size = bytes.len() as u64;
    let lang = carto_parse::detect_lang(rel);
    let t_parse = Instant::now();
    let parsed = match (&lang, size <= MAX_PARSE_BYTES, std::str::from_utf8(&bytes)) {
        (Some(_), true, Ok(src)) => carto_parse::parse_file(rel, src),
        _ => carto_model::ParsedFile::default(),
    };
    report.parse_ms += t_parse.elapsed().as_secs_f64() * 1000.0;

    let file_id = store
        .upsert_file(
            rel,
            lang.as_deref(),
            parsed.purpose.as_deref(),
            size as i64,
            hash.as_bytes(),
            &parsed.symbols,
            &parsed.refs,
            generation,
        )
        .with_context(|| format!("upsert {rel}"))?;

    // Re-resolve edges for the union of old + new names this file owns.
    let mut names: BTreeSet<String> = old_names.into_iter().collect();
    for s in &parsed.symbols {
        names.insert(s.name.clone());
    }
    let names: Vec<String> = names.into_iter().collect();
    let t_res = Instant::now();
    store.resolve_edges_incremental(file_id, &names)?;
    report.resolve_ms += t_res.elapsed().as_secs_f64() * 1000.0;

    if existing.is_some() {
        report.changed += 1;
    } else {
        report.added += 1;
    }
    Ok(())
}

/// Idle-time full PageRank recompute. Clears the `rank_dirty` marker. Kept out
/// of the edit hot path so a single-file sync stays inside the latency gate.
pub fn recompute_ranks(db: &Path) -> Result<f64> {
    let mut store = Store::open(db)?;
    let t = Instant::now();
    let nodes = store.all_symbol_ids()?;
    let edges = store.all_call_edges()?;
    let ranks = crate::pagerank::pagerank(&nodes, &edges, 0.85, 30);
    store.write_ranks(&ranks)?;
    store.set_meta("rank_dirty", "0")?;
    Ok(t.elapsed().as_secs_f64() * 1000.0)
}

/// Whether ranks are stale (a sync happened without a rank recompute).
pub fn rank_dirty(db: &Path) -> Result<bool> {
    let store = Store::open(db)?;
    Ok(store.get_meta("rank_dirty")?.as_deref() == Some("1"))
}
