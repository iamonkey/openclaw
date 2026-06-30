//! `carto-core` — orchestration spine.
//!
//! Composes `carto-parse` (extraction) and `carto-store` (SQLite) into the
//! index build (`index::build_index`) and the read-only tool facade
//! (`reader::IndexReader`). The CLI and benchmark crates depend only on this.

pub mod index;
pub mod pagerank;
pub mod reader;

pub use index::{build_index, BuildStats};
pub use reader::IndexReader;

/// Default index location relative to a repo root: `<root>/.carto/index.db`.
pub fn default_db_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".carto").join("index.db")
}

/// Ensure the parent dir of the db path exists.
pub fn ensure_db_dir(db_path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
