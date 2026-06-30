//! `carto-model` — the shared type contract for Cartograph.
//!
//! Plain data types mirroring the canonical SQLite schema
//! (`docs/codebase-understanding/02-data-model.md`). This crate has no logic
//! and no heavy deps so every other crate can depend on it without coupling to
//! storage or parsing. Stable by design: changing it is changing the contract.

use serde::{Deserialize, Serialize};

/// Internal integer id for a file row (`files.id`).
pub type FileId = i64;
/// Internal integer id for a symbol row (`symbols.id`).
pub type SymbolId = i64;

/// Tiered hydration level (H1). Tier-2 (full body) is read live, never stored.
/// Ordered so `tier >= Tier::Signature` reads naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// One-line purpose per file (`files.purpose`).
    Purpose = 0,
    /// Signatures + docstrings (`symbols.signature`, `skeletons` tier 1).
    Signature = 1,
    /// Full source body, read live from disk via `expand`.
    Body = 2,
}

impl Tier {
    pub fn as_i64(self) -> i64 {
        self as i64
    }
    pub fn from_i64(v: i64) -> Option<Tier> {
        match v {
            0 => Some(Tier::Purpose),
            1 => Some(Tier::Signature),
            2 => Some(Tier::Body),
            _ => None,
        }
    }
}

/// Kind of a symbol (`symbols.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Type,
    Interface,
    Const,
    Module,
    Field,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Type => "type",
            SymbolKind::Interface => "interface",
            SymbolKind::Const => "const",
            SymbolKind::Module => "module",
            SymbolKind::Field => "field",
        }
    }
    pub fn from_str(s: &str) -> Option<SymbolKind> {
        Some(match s {
            "function" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "class" => SymbolKind::Class,
            "type" => SymbolKind::Type,
            "interface" => SymbolKind::Interface,
            "const" => SymbolKind::Const,
            "module" => SymbolKind::Module,
            "field" => SymbolKind::Field,
            _ => return None,
        })
    }
}

/// Kind of a directed edge between symbols (`edges.kind`, H2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Call,
    Import,
    Inherit,
    Implement,
    TypeRef,
    Read,
    Write,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Call => "call",
            EdgeKind::Import => "import",
            EdgeKind::Inherit => "inherit",
            EdgeKind::Implement => "implement",
            EdgeKind::TypeRef => "typeref",
            EdgeKind::Read => "read",
            EdgeKind::Write => "write",
        }
    }
    pub fn from_str(s: &str) -> Option<EdgeKind> {
        Some(match s {
            "call" => EdgeKind::Call,
            "import" => EdgeKind::Import,
            "inherit" => EdgeKind::Inherit,
            "implement" => EdgeKind::Implement,
            "typeref" => EdgeKind::TypeRef,
            "read" => EdgeKind::Read,
            "write" => EdgeKind::Write,
            _ => return None,
        })
    }
}

/// A file row (`files`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRec {
    pub id: FileId,
    /// Repo-root-relative, forward slashes.
    pub path: String,
    /// tree-sitter grammar id; `None` if unknown.
    pub lang: Option<String>,
    /// Tier-0 one-line purpose (H1).
    pub purpose: Option<String>,
    pub size_bytes: i64,
    /// blake3 of file bytes; drives dirty detection.
    pub content_hash: Vec<u8>,
    pub generation: i64,
}

/// A symbol row (`symbols`). Bodies are not stored here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub file_id: FileId,
    /// Content-independent external handle (`02-data-model.md` §3).
    pub stable_key: String,
    pub name: String,
    pub fqn: Option<String>,
    pub kind: SymbolKind,
    /// Rendered signature (Tier-1), no body.
    pub signature: Option<String>,
    /// Leading docstring/comment, trimmed.
    pub doc: Option<String>,
    pub start_byte: i64,
    pub end_byte: i64,
    pub start_row: i64,
    pub end_row: i64,
    pub parent_id: Option<SymbolId>,
    /// Denormalized PageRank (H2); 0.0 until ranked.
    pub rank: f64,
    pub generation: i64,
}

/// A directed edge row (`edges`, H2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub src_id: SymbolId,
    pub dst_id: SymbolId,
    pub kind: EdgeKind,
    /// 1 = resolved to a def, 0 = unresolved/heuristic.
    pub resolved: bool,
    pub generation: i64,
}

/// A pre-rendered skeleton row (`skeletons`, H1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    pub symbol_id: SymbolId,
    pub tier: Tier,
    pub text: String,
    /// Cached token estimate for budgeting (H5).
    pub token_est: i64,
}

// ── Extraction output (carto-parse → carto-store) ───────────────────────────

/// A symbol as produced by the parser, before it has a db id assigned.
/// `parent_idx` references another `RawSymbol` within the same file by vec index.
#[derive(Debug, Clone)]
pub struct RawSymbol {
    pub stable_key: String,
    pub name: String,
    pub fqn: Option<String>,
    pub kind: SymbolKind,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub start_byte: i64,
    pub end_byte: i64,
    pub start_row: i64,
    pub end_row: i64,
    pub parent_idx: Option<usize>,
}

/// A reference produced by the parser, keyed by name. Resolution to a concrete
/// `dst` symbol happens later (H2); v1 may leave many edges unresolved.
#[derive(Debug, Clone)]
pub struct RawRef {
    /// Index into the file's `RawSymbol` vec that owns this reference.
    pub src_idx: usize,
    /// The referenced name (e.g. callee, imported symbol, type name).
    pub target_name: String,
    pub kind: EdgeKind,
}

/// Everything `carto-parse` extracts from one file.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    pub lang: Option<String>,
    pub purpose: Option<String>,
    pub symbols: Vec<RawSymbol>,
    pub refs: Vec<RawRef>,
}

// ── Tool result types (carto-core IndexReader → tools) ──────────────────────

/// One symbol entry inside an `outline` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineSymbol {
    pub key: String,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub kind: SymbolKind,
    pub rank: f64,
}

/// One file entry inside an `outline` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineEntry {
    pub path: String,
    pub purpose: Option<String>,
    pub symbols: Vec<OutlineSymbol>,
}

/// Result of `outline` (H1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outline {
    pub generation: i64,
    pub token_est: i64,
    pub truncated: bool,
    pub dropped: i64,
    pub entries: Vec<OutlineEntry>,
}

/// Result of `expand` (H1): the full live body of one symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpan {
    pub generation: i64,
    pub token_est: i64,
    pub symbol: String,
    pub lang: Option<String>,
    pub source: String,
    pub path: String,
    pub start_row: i64,
    pub end_row: i64,
}

/// Compute the canonical `stable_key` for a symbol.
///
/// `stable_key = "<repo-rel-path>#<container-path>/<name>:<kind>"`
/// where `container_path` is the slash-joined names of enclosing symbols
/// (empty for top-level). See `02-data-model.md` §3.
pub fn stable_key(path: &str, container_path: &[&str], name: &str, kind: SymbolKind) -> String {
    if container_path.is_empty() {
        format!("{path}#{name}:{}", kind.as_str())
    } else {
        format!("{path}#{}/{name}:{}", container_path.join("/"), kind.as_str())
    }
}

/// Cheap, deterministic token estimate (bytes/4 heuristic). Computed once at
/// write time and cached; good enough for budgeting (`02-data-model.md` §6).
pub fn estimate_tokens(text: &str) -> i64 {
    // ceil(bytes / 4), min 1 for non-empty text.
    if text.is_empty() {
        0
    } else {
        ((text.len() as i64) + 3) / 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_key_toplevel_and_nested() {
        assert_eq!(
            stable_key("src/a.ts", &[], "run", SymbolKind::Function),
            "src/a.ts#run:function"
        );
        assert_eq!(
            stable_key("src/a.ts", &["ConfigLoader"], "parse", SymbolKind::Method),
            "src/a.ts#ConfigLoader/parse:method"
        );
    }

    #[test]
    fn token_estimate_is_ceil_div4() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn kind_roundtrips() {
        for k in [
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Type,
            SymbolKind::Interface,
            SymbolKind::Const,
            SymbolKind::Module,
            SymbolKind::Field,
        ] {
            assert_eq!(SymbolKind::from_str(k.as_str()), Some(k));
        }
    }
}
