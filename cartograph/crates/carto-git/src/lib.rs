//! `carto-git` — git history mining for H3 (`06-h3-impact-radius.md`).
//!
//! Walks `git log` and computes file-level co-change (association rules:
//! support / confidence / lift) into the store's `file_cochange` table. v1 is
//! file-granular (robust against historical rename/move); symbol-level
//! attribution is a later refinement.
//!
//! NOTE: body is a stub pending the H3 implementation.

use anyhow::Result;
use carto_store::Store;
use std::path::Path;

/// Mine file co-change from the git repo at `root` and write `file_cochange`.
/// Returns the number of co-change pairs written. If `root` is not a git repo
/// (or has no history), writes nothing and returns 0 — `impact` then degrades
/// to static-only (architecture §7).
///
/// STUB: no-op until the H3 implementation lands.
pub fn mine_cochange(_root: &Path, _store: &mut Store) -> Result<usize> {
    Ok(0)
}
