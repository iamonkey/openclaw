//! Integration tests for H3 `impact` (`06-h3-impact-radius.md`).
//!
//! These build a real index from a temp TypeScript project and exercise the
//! fusion in `carto_core::impact::compute` through `IndexReader::impact`.

use std::path::{Path, PathBuf};
use std::process::Command;

use carto_core::{build_index, ImpactOpts, IndexReader};
use carto_model::ImpactSource;

/// Unique scratch dir under the system temp dir (no extra dev-deps needed).
fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("carto-impact-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, rel: &str, src: &str) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(abs, src).unwrap();
}

/// Static-only path (no git repo): a symbol with a known caller must surface as
/// a Static impact item, and `cochange_available` must be false.
#[test]
fn impact_static_only_when_no_git() {
    let root = scratch("static");
    // `caller` calls `add`; the parser resolves that into a call edge, so `add`
    // has a known neighbor.
    write(
        &root,
        "src/math.ts",
        "export function add(a: number, b: number): number {\n  return a + b;\n}\n",
    );
    write(
        &root,
        "src/use.ts",
        "import { add } from './math';\nexport function caller(): number {\n  return add(1, 2);\n}\n",
    );

    let db = root.join(".carto").join("index.db");
    carto_core::ensure_db_dir(&db).expect("mk db dir");
    build_index(&root, &db).expect("build index");
    let reader = IndexReader::open(&db, &root).expect("open reader");

    let center = "src/math.ts#add:function";
    let set = reader
        .impact(center, &ImpactOpts::default())
        .expect("impact");

    assert!(
        !set.cochange_available,
        "no git repo → cochange must be unavailable"
    );
    assert!(
        !set.items.is_empty(),
        "expected at least one static neighbor for {center}"
    );
    assert!(
        set.items.iter().all(|i| i.source == ImpactSource::Static),
        "without history every item must be Static"
    );
    assert!(
        set.items
            .iter()
            .any(|i| i.key == "src/use.ts#caller:function"),
        "the known caller should appear in the blast radius; got: {:?}",
        set.items.iter().map(|i| &i.key).collect::<Vec<_>>()
    );
    // center is never returned as its own impact item.
    assert!(set.items.iter().all(|i| i.key != center));

    let _ = std::fs::remove_dir_all(&root);
}

/// Run a git command in `root`, returning whether it succeeded.
fn git(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Co-change path: two files that change together across multiple commits must
/// produce a Cochange impact item and flip `cochange_available` to true.
///
/// Gated on `git` being available; if `git init` fails the test is skipped (the
/// static test above is the required one).
#[test]
fn impact_surfaces_cochange_when_history_couples_files() {
    let root = scratch("cochange");
    if !git(&root, &["init", "-q"]) {
        eprintln!("git unavailable; skipping co-change test");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    // Two parsed files that we will edit together. `a.ts` is the center's file;
    // `b.ts` co-changes with it. MIN_SUPPORT is 2, so we make >=2 joint commits.
    write(
        &root,
        "a.ts",
        "export function alpha(): number {\n  return 1;\n}\n",
    );
    write(
        &root,
        "b.ts",
        "export function beta(): number {\n  return 2;\n}\n",
    );

    for n in 0..3 {
        // Touch both files in the same commit each round.
        write(
            &root,
            "a.ts",
            &format!("export function alpha(): number {{\n  return {n};\n}}\n"),
        );
        write(
            &root,
            "b.ts",
            &format!(
                "export function beta(): number {{\n  return {};\n}}\n",
                n + 10
            ),
        );
        assert!(git(&root, &["add", "-A"]));
        assert!(git(&root, &["commit", "-q", "-m", "co-change"]));
    }

    let db = root.join(".carto").join("index.db");
    carto_core::ensure_db_dir(&db).expect("mk db dir");
    build_index(&root, &db).expect("build index");
    let reader = IndexReader::open(&db, &root).expect("open reader");

    // min_lift 1.0 so a perfectly-coupled pair (always together) is included;
    // its lift = total_commits / freq is >= 1 by construction here.
    let opts = ImpactOpts {
        static_depth: 2,
        min_lift: 1.0,
        max_tokens: Some(2000),
    };
    let set = reader.impact("a.ts#alpha:function", &opts).expect("impact");

    assert!(
        set.cochange_available,
        "mined history should make cochange available"
    );
    assert!(
        set.items
            .iter()
            .any(|i| i.source == ImpactSource::Cochange && i.key == "b.ts#beta:function"),
        "beta (in the co-changing file) should appear as a Cochange item; got: {:?}",
        set.items
            .iter()
            .map(|i| (i.key.clone(), i.source))
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&root);
}
