//! `carto-parse` — tree-sitter extraction (symbols, skeletons, refs, purpose).
//! Stub; implemented in M1 (see crate work item).

use carto_model::ParsedFile;

/// Detect language by file extension. Returns the tree-sitter grammar id.
pub fn detect_lang(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "ts" | "tsx" | "mts" | "cts" => Some("typescript".to_string()),
        _ => None,
    }
}

/// Parse one file's source into symbols/refs/purpose. Stub returns empty.
pub fn parse_file(_path: &str, _src: &str) -> ParsedFile {
    ParsedFile::default()
}
