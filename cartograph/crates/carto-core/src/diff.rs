//! H4 — Semantic Diff Hydration (`08-h4-semantic-diff-hydration.md`).
//!
//! Structural diff at *symbol* granularity between the last indexed state
//! (symbols in the store) and a fresh parse of the working tree on disk. Answers
//! "what changed since I last indexed" in a handful of tokens instead of
//! resending file text. Each change is classified into exactly one
//! [`ChangeKind`]: Renamed / Moved / Signature / Added / Removed / Body.
//!
//! Matching is by `stable_key` (the cheap 90%): keys present on both sides whose
//! signature differs are `Signature` changes. The keys that *don't* line up are
//! resolved in a second pass into `Renamed` (same file, equal signature,
//! different name), `Moved` (cross-file, equal name+kind+signature), or bare
//! `Added`/`Removed`.
//!
//! v1 limitations:
//! - We do not store symbol body text, so a body-only edit (identical signatures,
//!   same symbol set) cannot be localized to a symbol. It is reported once per
//!   file as a file-granular `Body` change keyed on the file path, gated on the
//!   on-disk blake3 hash differing from the stored `content_hash`.
//! - Rename/Move detection only pairs exact-signature matches; near-identical
//!   bodies and shape-similarity heuristics (doc §2) are out of scope for v1.

use crate::reader::IndexReader;
use anyhow::Result;
use carto_model::{estimate_tokens, AstDiff, ChangeKind, DiffChange, SymbolKind};
use std::collections::{HashMap, HashSet};

/// Lightweight view of a symbol used for diffing (stored or freshly parsed).
#[derive(Clone)]
struct SymInfo {
    name: String,
    kind: SymbolKind,
    signature: Option<String>,
}

/// A candidate add/remove that survived `stable_key` matching, retained for the
/// rename (same-file) and move (cross-file) resolution passes.
struct Candidate {
    path: String,
    key: String,
    info: SymInfo,
}

/// Diff the working tree on disk against the indexed snapshot, at symbol
/// granularity. Keeps the public signature stable (called by
/// `IndexReader::diff_working`).
pub fn working_tree_diff(reader: &IndexReader) -> Result<AstDiff> {
    let store = reader.store();

    let mut changes: Vec<DiffChange> = Vec::new();
    // Cross-file candidates collected across all files for move detection.
    let mut removed_candidates: Vec<Candidate> = Vec::new();
    let mut added_candidates: Vec<Candidate> = Vec::new();
    // Files that produced a structural change; used to suppress the file-level
    // Body fallback (a body change is only emitted when nothing structural was).
    let mut structural_files: HashSet<String> = HashSet::new();
    // Files whose on-disk bytes differ from the stored content_hash.
    let mut body_dirty_files: Vec<String> = Vec::new();

    for file in store.list_files()? {
        // Stored symbols for this file: stable_key -> info.
        let stored: HashMap<String, SymInfo> = store
            .symbols_in_file(file.id)?
            .into_iter()
            .map(|s| {
                (
                    s.stable_key,
                    SymInfo {
                        name: s.name,
                        kind: s.kind,
                        signature: s.signature,
                    },
                )
            })
            .collect();

        // Read the file fresh from disk. If unreadable (deleted / non-UTF-8),
        // skip the structural diff for this file so a transient read failure
        // doesn't spuriously report every stored symbol as Removed.
        let src = match reader.read_source(&file.path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Detect a body-level change for the file via blake3 of the on-disk
        // bytes vs the stored content_hash. blake3 is the same hash the index
        // build records, so equal bytes => equal hash => no body change.
        let disk_hash = blake3::hash(src.as_bytes());
        if disk_hash.as_bytes().as_slice() != file.content_hash.as_slice() {
            body_dirty_files.push(file.path.clone());
        }

        // Parse the current source. parse_file returns empty for unknown langs
        // (no symbols); for such files there is nothing structural to diff and
        // the body-hash check above still flags raw byte changes.
        let current: HashMap<String, SymInfo> = carto_parse::parse_file(&file.path, &src)
            .symbols
            .into_iter()
            .map(|rs| {
                (
                    rs.stable_key,
                    SymInfo {
                        name: rs.name,
                        kind: rs.kind,
                        signature: rs.signature,
                    },
                )
            })
            .collect();

        // Keys in both: compare signatures → Signature change when they differ.
        for (key, cur) in &current {
            if let Some(old) = stored.get(key) {
                if old.signature != cur.signature {
                    let detail = signature_detail(&old.signature, &cur.signature);
                    changes.push(DiffChange {
                        symbol: key.clone(),
                        change: ChangeKind::Signature,
                        token_est: estimate_tokens(&detail),
                        detail,
                    });
                    structural_files.insert(file.path.clone());
                }
            }
        }

        // Key only in stored → Removed candidate; only in current → Added.
        for (key, info) in &stored {
            if !current.contains_key(key) {
                removed_candidates.push(Candidate {
                    path: file.path.clone(),
                    key: key.clone(),
                    info: info.clone(),
                });
            }
        }
        for (key, info) in &current {
            if !stored.contains_key(key) {
                added_candidates.push(Candidate {
                    path: file.path.clone(),
                    key: key.clone(),
                    info: info.clone(),
                });
            }
        }
    }

    // ── Resolve candidates: Rename (same file) then Move (cross file) ─────────
    // `consumed_*` marks candidates already paired so they aren't double-counted
    // or emitted as bare Added/Removed.
    let mut removed_consumed = vec![false; removed_candidates.len()];
    let mut added_consumed = vec![false; added_candidates.len()];

    // Rename: within the same file, a Removed and an Added with equal signature
    // (and a non-empty signature) but different name → one Renamed change. This
    // is the case stable_key cannot represent (a rename breaks the key by design).
    for ri in 0..removed_candidates.len() {
        if removed_consumed[ri] {
            continue;
        }
        let matched = added_candidates.iter().enumerate().find(|(ai, add)| {
            !added_consumed[*ai]
                && add.path == removed_candidates[ri].path
                && add.info.name != removed_candidates[ri].info.name
                && rename_shape_match(&removed_candidates[ri].info, &add.info)
        });
        if let Some((ai, add)) = matched {
            let rem = &removed_candidates[ri];
            let detail = format!("{} -> {}", rem.info.name, add.info.name);
            changes.push(DiffChange {
                symbol: add.key.clone(),
                change: ChangeKind::Renamed,
                token_est: estimate_tokens(&detail),
                detail,
            });
            structural_files.insert(rem.path.clone());
            removed_consumed[ri] = true;
            added_consumed[ai] = true;
        }
    }

    // Move: a Removed name+kind+signature in file A and an identical Added in
    // file B → one Moved change keyed on the new location.
    for ri in 0..removed_candidates.len() {
        if removed_consumed[ri] {
            continue;
        }
        let matched = added_candidates.iter().enumerate().find(|(ai, add)| {
            !added_consumed[*ai]
                && add.path != removed_candidates[ri].path
                && add.info.name == removed_candidates[ri].info.name
                && add.info.kind == removed_candidates[ri].info.kind
                && signatures_match(&removed_candidates[ri].info, &add.info)
        });
        if let Some((ai, add)) = matched {
            let rem = &removed_candidates[ri];
            let detail = format!("{} -> {}", rem.path, add.path);
            changes.push(DiffChange {
                symbol: add.key.clone(),
                change: ChangeKind::Moved,
                token_est: estimate_tokens(&detail),
                detail,
            });
            structural_files.insert(rem.path.clone());
            structural_files.insert(add.path.clone());
            removed_consumed[ri] = true;
            added_consumed[ai] = true;
        }
    }

    // Leftover unpaired candidates → bare Added / Removed.
    for (ai, add) in added_candidates.iter().enumerate() {
        if added_consumed[ai] {
            continue;
        }
        let detail = added_detail(add);
        changes.push(DiffChange {
            symbol: add.key.clone(),
            change: ChangeKind::Added,
            token_est: estimate_tokens(&detail),
            detail,
        });
        structural_files.insert(add.path.clone());
    }
    for (ri, rem) in removed_candidates.iter().enumerate() {
        if removed_consumed[ri] {
            continue;
        }
        let detail = removed_detail(rem);
        changes.push(DiffChange {
            symbol: rem.key.clone(),
            change: ChangeKind::Removed,
            token_est: estimate_tokens(&detail),
            detail,
        });
        structural_files.insert(rem.path.clone());
    }

    // Body fallback: file bytes changed but no structural change was produced for
    // it. We can't localize to a symbol (no stored body text), so emit one
    // file-granular Body change keyed on the file path. (v1 limitation.)
    for path in body_dirty_files {
        if structural_files.contains(&path) {
            continue;
        }
        let detail = "body changed".to_string();
        changes.push(DiffChange {
            symbol: path,
            change: ChangeKind::Body,
            token_est: estimate_tokens(&detail),
            detail,
        });
    }

    // Order by significance: Renamed/Moved/Signature/Added/Removed before Body.
    changes.sort_by_key(|c| significance(c.change));

    let token_est = changes.iter().map(|c| c.token_est).sum();

    Ok(AstDiff {
        generation: reader.generation()?,
        from: "INDEX".to_string(),
        to: "WORKING".to_string(),
        token_est,
        changes,
    })
}

/// Significance ordering for truncation/display (doc §7): structural changes
/// before body churn. Lower sorts first.
fn significance(kind: ChangeKind) -> u8 {
    match kind {
        ChangeKind::Renamed => 0,
        ChangeKind::Moved => 1,
        ChangeKind::Signature => 2,
        ChangeKind::Added => 3,
        ChangeKind::Removed => 4,
        ChangeKind::Body => 5,
    }
}

/// Move-shape match: both carry a non-empty signature and the signatures are
/// equal. A move keeps the same name, so the full signature (which embeds the
/// name) is identical. We require a real signature on both sides so
/// signature-less symbols don't pair spuriously.
fn signatures_match(a: &SymInfo, b: &SymInfo) -> bool {
    match (&a.signature, &b.signature) {
        (Some(x), Some(y)) => !x.is_empty() && x == y,
        _ => false,
    }
}

/// Rename-shape match: the rendered signature embeds the symbol name, so a
/// rename necessarily changes the signature string. We instead pair on the
/// name-independent shape — same kind and an identical top-level parameter list
/// (per doc §2, "same kind + param-list"). Both must expose a parameter group;
/// signature-less or param-less symbols (e.g. a bare const) don't pair.
fn rename_shape_match(a: &SymInfo, b: &SymInfo) -> bool {
    if a.kind != b.kind {
        return false;
    }
    match (
        a.signature.as_deref().and_then(params_of),
        b.signature.as_deref().and_then(params_of),
    ) {
        (Some(pa), Some(pb)) => pa == pb,
        _ => false,
    }
}

/// Render a Signature change detail. Tries a cheap param-level diff (one
/// `+param`/`-param` fragment per added/removed parameter); falls back to the
/// faithful `old -> new` rendering when params can't be cleanly extracted or no
/// params changed.
fn signature_detail(old: &Option<String>, new: &Option<String>) -> String {
    let old_s = old.as_deref().unwrap_or("");
    let new_s = new.as_deref().unwrap_or("");
    if let (Some(old_params), Some(new_params)) = (params_of(old_s), params_of(new_s)) {
        let mut frags: Vec<String> = Vec::new();
        for p in &new_params {
            if !old_params.contains(p) {
                frags.push(format!("+param {p}"));
            }
        }
        for p in &old_params {
            if !new_params.contains(p) {
                // For a removed param show just the binding name when possible.
                let name = p.split(':').next().unwrap_or(p).trim();
                frags.push(format!("-param {name}"));
            }
        }
        if !frags.is_empty() {
            return frags.join(", ");
        }
    }
    // Fallback: faithful old -> new (e.g. return-type or other header changes).
    format!("{old_s} -> {new_s}")
}

/// Extract the comma-separated parameter fragments from a signature's first
/// parenthesized group, e.g. `parse(src: string, strict: boolean): T` →
/// `["src: string", "strict: boolean"]`. Returns `None` when there is no
/// parenthesized group (so callers fall back to old -> new). An empty param
/// list returns `Some(vec![])`.
fn params_of(sig: &str) -> Option<Vec<String>> {
    let open = sig.find('(')?;
    // Find the matching close paren accounting for nesting (generics/objects).
    let mut depth = 0i32;
    let mut close_rel = None;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close_rel = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close_rel? + open;
    let inner = sig[open + 1..close].trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    Some(split_top_level(inner))
}

/// Split a parameter list on top-level commas (ignoring commas nested inside
/// `<>`, `()`, `[]`, `{}`), trimming each fragment.
fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let frag = s[start..i].trim();
                if !frag.is_empty() {
                    out.push(frag.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let frag = s[start..].trim();
    if !frag.is_empty() {
        out.push(frag.to_string());
    }
    out
}

/// Detail for an added symbol: its full signature (so the agent learns the new
/// shape without a follow-up expand), prefixed with `+`. Falls back to
/// `+ <kind> <name>` when the symbol has no rendered signature.
fn added_detail(c: &Candidate) -> String {
    match &c.info.signature {
        Some(sig) if !sig.is_empty() => format!("+ {sig}"),
        _ => format!("+ {} {}", c.info.kind.as_str(), c.info.name),
    }
}

/// Detail for a removed symbol: its prior signature prefixed with `-`.
fn removed_detail(c: &Candidate) -> String {
    match &c.info.signature {
        Some(sig) if !sig.is_empty() => format!("- {sig}"),
        _ => format!("- {} {}", c.info.kind.as_str(), c.info.name),
    }
}
