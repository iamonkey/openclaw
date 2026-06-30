//! Integration tests for H4 working-tree semantic diff (`carto_core::diff`).
//!
//! Build an index over a temp TS dir, mutate files on disk, then call
//! `IndexReader::diff_working()` and assert the expected structural changes
//! surface. The store holds the *indexed* snapshot; the diff reparses the
//! mutated working tree and reports the symbol-granular delta.

use carto_core::{build_index, default_db_path, IndexReader};
use carto_model::ChangeKind;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A self-cleaning temp directory. We avoid pulling in the `tempfile` crate as a
/// dev-dependency: a process-unique counter + pid keeps concurrent test runs
/// from colliding, and `Drop` removes the tree.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "carto-diff-test-{}-{}-{}",
            std::process::id(),
            n,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Write `contents` to `<root>/<rel>`, creating parent dirs.
fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, contents).unwrap();
}

/// Build (or rebuild) the index and open a reader over the temp root.
fn index_and_open(root: &Path) -> IndexReader {
    let db = default_db_path(root);
    carto_core::ensure_db_dir(&db).unwrap();
    build_index(root, &db).unwrap();
    IndexReader::open(&db, root).unwrap()
}

/// Find the single change matching a kind whose detail contains `needle`.
fn find<'a>(
    changes: &'a [carto_model::DiffChange],
    kind: ChangeKind,
    needle: &str,
) -> Option<&'a carto_model::DiffChange> {
    changes
        .iter()
        .find(|c| c.change == kind && c.detail.contains(needle))
}

#[test]
fn no_disk_change_yields_zero_changes() {
    let tmp = TempDir::new();
    let root = tmp.path();
    write(
        root,
        "src/a.ts",
        "export function add(a: number, b: number): number {\n  return a + b;\n}\n",
    );

    let reader = index_and_open(root);
    let diff = reader.diff_working().unwrap();

    assert_eq!(diff.from, "INDEX");
    assert_eq!(diff.to, "WORKING");
    assert!(
        diff.changes.is_empty(),
        "expected no changes when disk matches index, got: {:?}",
        diff.changes
    );
    assert_eq!(diff.token_est, 0);
}

#[test]
fn signature_change_is_detected() {
    let tmp = TempDir::new();
    let root = tmp.path();
    write(
        root,
        "src/a.ts",
        "export function parse(src: string): Config {\n  return load(src);\n}\n",
    );
    let reader = index_and_open(root);

    // Add a parameter to `parse` on disk.
    write(
        root,
        "src/a.ts",
        "export function parse(src: string, strict: boolean): Config {\n  return load(src);\n}\n",
    );

    let diff = reader.diff_working().unwrap();
    let sig = find(&diff.changes, ChangeKind::Signature, "strict")
        .unwrap_or_else(|| panic!("expected a signature change, got: {:?}", diff.changes));
    assert_eq!(sig.symbol, "src/a.ts#parse:function");
    // Param-level rendering should surface the added parameter.
    assert!(
        sig.detail.contains("+param strict: boolean"),
        "detail should itemize the new param: {}",
        sig.detail
    );
    assert!(sig.token_est > 0);
}

#[test]
fn added_and_removed_symbols_are_detected() {
    let tmp = TempDir::new();
    let root = tmp.path();
    write(
        root,
        "src/a.ts",
        "export function keep(): void {}\nexport function gone(x: number): void {}\n",
    );
    let reader = index_and_open(root);

    // Delete `gone`, add `fresh`. `keep` is untouched.
    write(
        root,
        "src/a.ts",
        "export function keep(): void {}\nexport function fresh(y: string): void {}\n",
    );

    let diff = reader.diff_working().unwrap();

    let added = find(&diff.changes, ChangeKind::Added, "fresh")
        .unwrap_or_else(|| panic!("expected `fresh` added, got: {:?}", diff.changes));
    assert_eq!(added.symbol, "src/a.ts#fresh:function");
    assert!(added.detail.starts_with('+'));

    let removed = find(&diff.changes, ChangeKind::Removed, "gone")
        .unwrap_or_else(|| panic!("expected `gone` removed, got: {:?}", diff.changes));
    assert_eq!(removed.symbol, "src/a.ts#gone:function");
    assert!(removed.detail.starts_with('-'));

    // `keep` was untouched: it must not appear as any change.
    assert!(
        !diff.changes.iter().any(|c| c.symbol.contains("keep")),
        "untouched symbol leaked into diff: {:?}",
        diff.changes
    );
}

#[test]
fn rename_pairs_add_and_remove() {
    let tmp = TempDir::new();
    let root = tmp.path();
    write(
        root,
        "src/a.ts",
        "export function validate(src: string): boolean {\n  return helper(src);\n}\n",
    );
    let reader = index_and_open(root);

    // Rename `validate` -> `validateStrict`, keeping the same body/signature shape.
    write(
        root,
        "src/a.ts",
        "export function validateStrict(src: string): boolean {\n  return helper(src);\n}\n",
    );

    let diff = reader.diff_working().unwrap();

    let renamed = find(
        &diff.changes,
        ChangeKind::Renamed,
        "validate -> validateStrict",
    )
    .unwrap_or_else(|| panic!("expected a rename, got: {:?}", diff.changes));
    assert_eq!(renamed.symbol, "src/a.ts#validateStrict:function");

    // The rename must consume both candidates: no bare Added/Removed for them.
    assert!(
        !diff
            .changes
            .iter()
            .any(|c| matches!(c.change, ChangeKind::Added | ChangeKind::Removed)),
        "rename should consume add+remove, got: {:?}",
        diff.changes
    );
}

#[test]
fn move_across_files_is_detected() {
    let tmp = TempDir::new();
    let root = tmp.path();
    let body = "export function shared(a: number): number {\n  return a + 1;\n}\n";
    write(root, "src/a.ts", body);
    write(root, "src/b.ts", "export function other(): void {}\n");
    let reader = index_and_open(root);

    // Move `shared` from a.ts to b.ts verbatim.
    write(root, "src/a.ts", "export const PLACEHOLDER = 1;\n");
    write(
        root,
        "src/b.ts",
        &format!("export function other(): void {{}}\n{body}"),
    );

    let diff = reader.diff_working().unwrap();

    let moved = find(&diff.changes, ChangeKind::Moved, "src/a.ts -> src/b.ts")
        .unwrap_or_else(|| panic!("expected a move, got: {:?}", diff.changes));
    assert_eq!(moved.symbol, "src/b.ts#shared:function");

    // The moved symbol must not also appear as bare Added/Removed.
    assert!(
        !diff.changes.iter().any(
            |c| matches!(c.change, ChangeKind::Added | ChangeKind::Removed)
                && c.detail.contains("shared")
        ),
        "move should consume the add+remove for `shared`, got: {:?}",
        diff.changes
    );
}

#[test]
fn body_only_change_is_file_granular() {
    let tmp = TempDir::new();
    let root = tmp.path();
    write(
        root,
        "src/a.ts",
        "export function run(): number {\n  return 1;\n}\n",
    );
    let reader = index_and_open(root);

    // Change only the body; the signature (`run(): number`) is unchanged.
    write(
        root,
        "src/a.ts",
        "export function run(): number {\n  const x = 41;\n  return x + 1;\n}\n",
    );

    let diff = reader.diff_working().unwrap();

    // No structural change for this file, so a single file-granular Body change.
    let body = find(&diff.changes, ChangeKind::Body, "body changed")
        .unwrap_or_else(|| panic!("expected a body change, got: {:?}", diff.changes));
    assert_eq!(body.symbol, "src/a.ts", "body change is keyed on file path");
    assert!(
        !diff
            .changes
            .iter()
            .any(|c| !matches!(c.change, ChangeKind::Body)),
        "expected only a body change, got: {:?}",
        diff.changes
    );
}

#[test]
fn changes_are_ordered_by_significance() {
    let tmp = TempDir::new();
    let root = tmp.path();
    // Two files so we can produce a structural change in one and a body-only
    // change in the other in a single diff.
    write(
        root,
        "src/a.ts",
        "export function f(a: number): number {\n  return a;\n}\n",
    );
    write(
        root,
        "src/b.ts",
        "export function g(): number {\n  return 1;\n}\n",
    );
    let reader = index_and_open(root);

    // a.ts: signature change. b.ts: body-only change.
    write(
        root,
        "src/a.ts",
        "export function f(a: number, b: number): number {\n  return a;\n}\n",
    );
    write(
        root,
        "src/b.ts",
        "export function g(): number {\n  return 1 + 1;\n}\n",
    );

    let diff = reader.diff_working().unwrap();
    assert!(diff.changes.len() >= 2, "got: {:?}", diff.changes);

    // Body changes must sort after all structural changes.
    let first_body = diff
        .changes
        .iter()
        .position(|c| c.change == ChangeKind::Body);
    let last_structural = diff
        .changes
        .iter()
        .rposition(|c| c.change != ChangeKind::Body);
    if let (Some(b), Some(s)) = (first_body, last_structural) {
        assert!(b > s, "body change should sort last: {:?}", diff.changes);
    }

    // token_est is the sum of per-change estimates.
    let sum: i64 = diff.changes.iter().map(|c| c.token_est).sum();
    assert_eq!(diff.token_est, sum);
}
