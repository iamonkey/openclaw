//! Integration tests for H5 (`plan::plan`): task classification + greedy,
//! budget-filled retrieval plan over a tiny built index.

use carto_core::{build_index, IndexReader};
use carto_model::TaskClass;
use std::path::PathBuf;

/// Create a unique temp directory under the system temp dir, seed it with a TS
/// file containing an obvious `paginate` target, build the index, and return an
/// open `IndexReader` plus the temp root (kept alive for cleanup).
struct Fixture {
    reader: IndexReader,
    _root: TempDir,
}

/// Minimal self-cleaning temp dir (avoids adding a `tempfile` dev-dependency).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "carto-plan-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> Fixture {
    let root = TempDir::new("repo");
    let src = root.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    // An obvious target symbol: `paginate`.
    std::fs::write(
        src.join("pagination.ts"),
        r#"
export function paginate(items: number[], page: number, size: number): number[] {
  const start = page * size;
  return items.slice(start, start + size);
}

export function totalPages(count: number, size: number): number {
  return Math.ceil(count / size);
}
"#,
    )
    .unwrap();
    std::fs::write(
        src.join("util.ts"),
        "export function clamp(n: number, lo: number, hi: number): number { return Math.max(lo, Math.min(hi, n)); }\n",
    )
    .unwrap();

    let db = root.path().join(".carto").join("index.db");
    carto_core::ensure_db_dir(&db).unwrap();
    build_index(root.path(), &db).unwrap();
    let reader = IndexReader::open(&db, root.path()).unwrap();
    Fixture {
        reader,
        _root: root,
    }
}

#[test]
fn classifies_where_is() {
    let fx = fixture();
    let plan = fx
        .reader
        .plan("Where is pagination implemented?", 2000, true)
        .unwrap();
    assert_eq!(plan.task_class, TaskClass::WhereIs);
}

#[test]
fn classifies_bug_localize_over_where() {
    let fx = fixture();
    // Mentions neither "break" nor "where" but a bug — must route to BugLocalize.
    let plan = fx
        .reader
        .plan("fix the off-by-one bug in pagination", 2000, true)
        .unwrap();
    assert_eq!(plan.task_class, TaskClass::BugLocalize);
}

#[test]
fn classifies_add_as_where_is_over_incidental_keywords() {
    let fx = fixture();
    // "Add a method ..." is an implementation task: the "depend"-style impact
    // keyword (here "change") must not hijack it into WhatBreaks. It routes to
    // WhereIs so the pattern to mirror gets located, not a blast-radius survey.
    let plan = fx
        .reader
        .plan(
            "Add a new paginate variant; this should not change downstream impact",
            2000,
            true,
        )
        .unwrap();
    assert_eq!(plan.task_class, TaskClass::WhereIs);
}

#[test]
fn default_task_locates_instead_of_outline() {
    let fx = fixture();
    // A task with searchable terms but no add/where/fix/impact signal still
    // falls to Default — which must now locate real targets (search + expand)
    // rather than emit only a whole-repo outline survey.
    let plan = fx
        .reader
        .plan("paginate items per page", 2000, false)
        .unwrap();
    assert_eq!(plan.task_class, TaskClass::Default);
    // The located target body must be hydrated, not just an outline skeleton.
    let mentions_paginate = plan
        .context
        .iter()
        .any(|c| c.text.contains("paginate") || c.key.as_deref() == Some("src/pagination.ts#paginate:function"));
    assert!(
        mentions_paginate,
        "Default plan should locate the paginate target, not fall back to outline"
    );
    // And it should record at least one search step (the locate path ran).
    assert!(
        plan.steps.iter().any(|s| s.tool == "search"),
        "Default plan should run search before any outline fallback"
    );
}

#[test]
fn where_is_plan_hydrates_target_within_budget() {
    let fx = fixture();
    let budget = 2000;
    let plan = fx
        .reader
        .plan("Where is pagination implemented?", budget, false)
        .unwrap();

    assert!(!plan.context.is_empty(), "context should be hydrated");
    assert!(
        plan.spent_est <= budget,
        "spent_est {} must not exceed budget {budget}",
        plan.spent_est
    );
    // spent_est is the sum of returned chunk token estimates.
    let sum: i64 = plan.context.iter().map(|c| c.token_est).sum();
    assert_eq!(plan.spent_est, sum);

    // The target symbol's key must appear somewhere in the hydrated context
    // (either the cheap locate list or an expanded body chunk).
    let mentions_paginate = plan.context.iter().any(|c| {
        c.key
            .as_deref()
            .map(|k| k.contains("paginate"))
            .unwrap_or(false)
            || c.text.contains("paginate")
    });
    assert!(
        mentions_paginate,
        "expected the paginate target in the hydrated context"
    );
}

#[test]
fn dry_run_returns_steps_but_no_context() {
    let fx = fixture();
    let plan = fx
        .reader
        .plan("Where is pagination implemented?", 2000, true)
        .unwrap();
    assert!(!plan.steps.is_empty(), "dry_run should still produce steps");
    assert!(plan.context.is_empty(), "dry_run context must be empty");
    assert_eq!(plan.spent_est, 0, "dry_run spends nothing");
}
