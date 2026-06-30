//! Incremental sync correctness (H4 / doc 10).

use carto_core::{build_index, sync, sync_paths, IndexReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Minimal self-cleaning temp dir (avoids a tempfile dev-dependency).
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> TempDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "carto-sync-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, src).unwrap();
}

#[test]
fn incremental_sync_updates_symbols_and_cross_file_edges() {
    let tmp = TempDir::new();
    let root = tmp.path();
    let db = root.join(".carto").join("index.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();

    write(root, "src/util.ts", "export function slugify(s: string): string { return s; }\n");
    write(
        root,
        "src/app.ts",
        "import { slugify } from \"./util\";\nexport function run(): void { slugify(\"x\"); }\n",
    );

    let stats = build_index(root, &db).unwrap();
    assert!(stats.symbols >= 2, "expected slugify + run");

    // Baseline: run() calls slugify().
    {
        let reader = IndexReader::open(&db, root).unwrap();
        let callers = reader.who_calls("src/util.ts#slugify:function", &[]).unwrap();
        assert!(
            callers.iter().any(|c| c.name == "run"),
            "run should call slugify before edit"
        );
    }

    // Edit app.ts: add a brand-new function `extra` that also calls slugify.
    write(
        root,
        "src/app.ts",
        "import { slugify } from \"./util\";\n\
         export function run(): void { slugify(\"x\"); }\n\
         export function extra(): string { return slugify(\"y\"); }\n",
    );
    let report = sync_paths(root, &db, &["src/app.ts".to_string()]).unwrap();
    assert_eq!(report.changed, 1, "app.ts should be reparsed");
    assert!(report.generation >= 2);

    let reader = IndexReader::open(&db, root).unwrap();
    // The new symbol exists.
    assert!(
        reader.symbol_by_key("src/app.ts#extra:function").unwrap().is_some(),
        "new symbol `extra` should be indexed after sync"
    );
    // Incremental edge re-resolution wired the new caller.
    let callers = reader.who_calls("src/util.ts#slugify:function", &[]).unwrap();
    let names: Vec<&str> = callers.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"run"), "run still calls slugify");
    assert!(names.contains(&"extra"), "extra should now call slugify after incremental sync");
}

#[test]
fn clean_file_is_skipped_and_deletion_is_handled() {
    let tmp = TempDir::new();
    let root = tmp.path();
    let db = root.join(".carto").join("index.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();

    write(root, "src/a.ts", "export function a(): void {}\n");
    write(root, "src/b.ts", "export function b(): void {}\n");
    build_index(root, &db).unwrap();

    // No edits → a whole-tree sync skips everything as clean.
    let report = sync(root, &db).unwrap();
    assert_eq!(report.changed, 0);
    assert_eq!(report.added, 0);
    assert!(report.skipped_clean >= 2, "both files clean");

    // Delete b.ts on disk and sync → it is removed from the index.
    std::fs::remove_file(root.join("src/b.ts")).unwrap();
    let report = sync(root, &db).unwrap();
    assert_eq!(report.deleted, 1, "b.ts should be deleted from the index");

    let reader = IndexReader::open(&db, root).unwrap();
    assert!(reader.symbol_by_key("src/b.ts#b:function").unwrap().is_none());
    assert!(reader.symbol_by_key("src/a.ts#a:function").unwrap().is_some());
}
