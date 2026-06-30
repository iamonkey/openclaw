//! H4 — Semantic Diff Hydration (`08-h4-semantic-diff-hydration.md`).
//!
//! Structural diff at *symbol* granularity between the last indexed state
//! (symbols in the store) and a fresh parse of the working tree. Answers "what
//! changed since I last indexed" in a handful of tokens instead of resending
//! file text. Classifies each change as added/removed/renamed/signature/moved.
//!
//! NOTE: body is a stub pending the H4 implementation.

use crate::reader::IndexReader;
use anyhow::Result;
use carto_model::AstDiff;

/// Diff the working tree against the indexed snapshot. STUB: returns no changes.
pub fn working_tree_diff(reader: &IndexReader) -> Result<AstDiff> {
    Ok(AstDiff {
        generation: reader.generation()?,
        from: "INDEX".to_string(),
        to: "WORKING".to_string(),
        token_est: 0,
        changes: Vec::new(),
    })
}
