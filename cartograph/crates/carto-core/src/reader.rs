//! `IndexReader` — the read-only facade backing the H1/H2 tools.
//! (`01-architecture.md` §3). Each method maps to one MCP tool
//! (`03-mcp-tool-surface.md`) plus the shared budget protocol (§4).

use anyhow::{anyhow, Result};
use carto_model::{
    estimate_tokens, EdgeKind, Outline, OutlineEntry, OutlineSymbol, SourceSpan, Symbol, Tier,
};
use carto_store::Store;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct IndexReader {
    store: Store,
    root: PathBuf,
}

impl IndexReader {
    /// Open the index at `db_path`; `root` is the repo root used to read live
    /// source bodies for `expand`.
    pub fn open(db_path: &Path, root: &Path) -> Result<IndexReader> {
        Ok(IndexReader {
            store: Store::open(db_path)?,
            root: root.to_path_buf(),
        })
    }

    pub fn generation(&self) -> Result<i64> {
        self.store.generation()
    }

    fn file_path_map(&self) -> Result<HashMap<i64, String>> {
        Ok(self
            .store
            .list_files()?
            .into_iter()
            .map(|f| (f.id, f.path))
            .collect())
    }

    /// H1 `outline`: tiered skeleton of a file or directory subtree.
    /// `path` = "" for repo root. Budget-truncates low-rank symbols first.
    pub fn outline(&self, path: &str, tier: Tier, max_tokens: Option<i64>) -> Result<Outline> {
        let generation = self.store.generation()?;
        let files = self.store.list_files()?;

        // file vs directory: exact file match, else prefix.
        let selected: Vec<_> = if files.iter().any(|f| f.path == path) {
            files.into_iter().filter(|f| f.path == path).collect()
        } else {
            let prefix = if path.is_empty() {
                String::new()
            } else {
                format!("{}/", path.trim_end_matches('/'))
            };
            files
                .into_iter()
                .filter(|f| prefix.is_empty() || f.path.starts_with(&prefix))
                .collect()
        };

        let mut entries = Vec::new();
        let mut spent: i64 = 0;
        let mut dropped: i64 = 0;
        let mut truncated = false;
        let budget = max_tokens.unwrap_or(i64::MAX);

        for f in &selected {
            // Tier-0 cost: the purpose line.
            let purpose_tokens = f
                .purpose
                .as_deref()
                .map(estimate_tokens)
                .unwrap_or(0)
                .max(1);
            if spent + purpose_tokens > budget {
                truncated = true;
                dropped += 1;
                continue;
            }
            spent += purpose_tokens;

            let mut syms = Vec::new();
            if tier >= Tier::Signature {
                let mut file_syms = self.store.symbols_in_file(f.id)?;
                // rank DESC so the architecturally heaviest survive truncation
                file_syms.sort_by(|a, b| b.rank.total_cmp(&a.rank));
                for s in file_syms {
                    let line = render_sig_line(&s);
                    let cost = estimate_tokens(&line);
                    if spent + cost > budget {
                        truncated = true;
                        dropped += 1;
                        continue;
                    }
                    spent += cost;
                    syms.push(OutlineSymbol {
                        key: s.stable_key,
                        signature: s.signature,
                        doc: s.doc,
                        kind: s.kind,
                        rank: s.rank,
                    });
                }
            }
            entries.push(OutlineEntry {
                path: f.path.clone(),
                purpose: f.purpose.clone(),
                symbols: syms,
            });
        }

        Ok(Outline {
            generation,
            token_est: spent,
            truncated,
            dropped,
            entries,
        })
    }

    /// H1 `expand`: full live source body of one symbol, read from disk.
    pub fn expand(&self, key: &str) -> Result<SourceSpan> {
        let generation = self.store.generation()?;
        let sym = self
            .store
            .symbol_by_key(key)?
            .ok_or_else(|| anyhow!("unknown symbol: {key}"))?;
        let paths = self.file_path_map()?;
        let rel = paths
            .get(&sym.file_id)
            .ok_or_else(|| anyhow!("file for symbol not found"))?;
        let abs = self.root.join(rel);
        let bytes = std::fs::read(&abs)?;
        let s = sym.start_byte.max(0) as usize;
        let e = (sym.end_byte.max(0) as usize).min(bytes.len());
        let slice = if s <= e { &bytes[s..e] } else { &bytes[0..0] };
        let source = String::from_utf8_lossy(slice).to_string();
        Ok(SourceSpan {
            generation,
            token_est: estimate_tokens(&source),
            symbol: sym.stable_key,
            lang: None,
            source,
            path: rel.clone(),
            start_row: sym.start_row,
            end_row: sym.end_row,
        })
    }

    /// H2 `who_calls`: reverse edges into a symbol.
    pub fn who_calls(&self, key: &str, kinds: &[EdgeKind]) -> Result<Vec<Symbol>> {
        let sym = self
            .store
            .symbol_by_key(key)?
            .ok_or_else(|| anyhow!("unknown symbol: {key}"))?;
        self.store.who_calls(sym.id, kinds)
    }

    /// H2 `neighborhood`: bounded rank-ordered subgraph around a symbol.
    pub fn neighborhood(&self, key: &str, depth: u8) -> Result<Vec<(Symbol, i64)>> {
        let sym = self
            .store
            .symbol_by_key(key)?
            .ok_or_else(|| anyhow!("unknown symbol: {key}"))?;
        self.store.neighborhood(sym.id, depth)
    }

    /// Entry-point fuzzy symbol search.
    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<Symbol>> {
        self.store.search_symbols(query, limit)
    }

    pub fn symbol_by_key(&self, key: &str) -> Result<Option<Symbol>> {
        self.store.symbol_by_key(key)
    }

    // ── Accessors for the hypothesis modules (impact/diff/plan) ───────────────

    /// Borrow the underlying store (read-only use by H3/H4/H5 modules).
    pub(crate) fn store(&self) -> &carto_store::Store {
        &self.store
    }

    /// Repo root (for reading live source in H4 working-tree diff).
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Read a file's current source from disk (repo-relative path).
    pub(crate) fn read_source(&self, rel: &str) -> Result<String> {
        Ok(std::fs::read_to_string(self.root.join(rel))?)
    }

    /// H3 `impact`: blast radius fusing static neighbors with git co-change.
    pub fn impact(&self, key: &str, opts: &crate::impact::ImpactOpts) -> Result<carto_model::ImpactSet> {
        crate::impact::compute(self, key, opts)
    }

    /// H4 `diff_context`: structural diff of the working tree vs the last index.
    pub fn diff_working(&self) -> Result<carto_model::AstDiff> {
        crate::diff::working_tree_diff(self)
    }

    /// H5 `plan_retrieval`: budget-constrained retrieval plan for a task.
    pub fn plan(&self, task: &str, budget: i64, dry_run: bool) -> Result<carto_model::RetrievalPlan> {
        crate::plan::plan(self, task, budget, dry_run)
    }
}

/// Render a one-line outline entry for a symbol: prefer the stored signature,
/// fall back to "kind name".
fn render_sig_line(s: &Symbol) -> String {
    match &s.signature {
        Some(sig) if !sig.is_empty() => sig.clone(),
        _ => format!("{} {}", s.kind.as_str(), s.name),
    }
}
