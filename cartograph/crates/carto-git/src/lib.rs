//! `carto-git` — git history mining for H3 (`06-h3-impact-radius.md`).
//!
//! Walks `git log` and computes file-level co-change (association rules:
//! support / confidence / lift) into the store's `file_cochange` table. v1 is
//! file-granular (robust against historical rename/move); symbol-level
//! attribution is a later refinement.

use anyhow::Result;
use carto_model::{FileCochangeRow, FileId};
use carto_store::Store;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Most recent commits to mine. Bounds cold-start cost and biases toward
/// *current* coupling (`06-h3-impact-radius.md` §2.3).
const COCHANGE_WINDOW: usize = 5000;

/// Commits touching more indexed files than this are skipped: a formatting
/// sweep or mass rename would otherwise manufacture O(n^2) spurious pairs that
/// all look mutually coupled (`06-h3-impact-radius.md` §3.4).
const MAX_COMMIT_FILES: usize = 50;

/// Minimum number of commits a pair must co-occur in to be written. Two files
/// that changed together once are coincidence, not coupling (§2.3).
const MIN_SUPPORT: i64 = 2;

/// Mine file co-change from the git repo at `root` and write `file_cochange`.
/// Returns the number of co-change rows written. If `root` is not a git repo
/// (or has no history), writes nothing and returns 0 — `impact` then degrades
/// to static-only (architecture §7).
pub fn mine_cochange(root: &Path, store: &mut Store) -> Result<usize> {
    // Not a repo / bare / corrupt → degrade silently to static-only.
    let repo = match git2::Repository::open(root) {
        Ok(r) => r,
        Err(_) => return Ok(0),
    };

    // No commits yet (unborn HEAD) → nothing to mine.
    let mut revwalk = match repo.revwalk() {
        Ok(w) => w,
        Err(_) => return Ok(0),
    };
    if revwalk.push_head().is_err() {
        return Ok(0);
    }
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    // Per-file change frequency, pairwise co-occurrence (unordered key with
    // a < b), and the count of commits that touched >=1 indexed file.
    let mut freq: HashMap<FileId, i64> = HashMap::new();
    let mut co: HashMap<(FileId, FileId), i64> = HashMap::new();
    let mut total_commits: i64 = 0;

    for (i, oid) in revwalk.enumerate() {
        if i >= COCHANGE_WINDOW {
            break;
        }
        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        // Skip merges: a merge's diff against one parent is the other branch's
        // entire delta, not a coherent unit of intent (§3.5).
        if commit.parent_count() > 1 {
            continue;
        }

        // Diff against the first parent (or the empty tree for the root commit).
        let commit_tree = commit.tree()?;
        let parent_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree()?),
            Err(_) => None, // root commit
        };
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None)?;

        // Collect changed file paths (repo-relative, forward slashes), mapped to
        // the file ids currently in the index. Paths not in the index are
        // ignored. A set dedups add+delete of the same path within one commit.
        let mut touched: HashSet<FileId> = HashSet::new();
        for delta in diff.deltas() {
            for path in [delta.old_file().path(), delta.new_file().path()]
                .into_iter()
                .flatten()
            {
                let rel = path.to_string_lossy().replace('\\', "/");
                if let Some(rec) = store.file_by_path(&rel)? {
                    touched.insert(rec.id);
                }
            }
        }

        if touched.is_empty() {
            continue;
        }
        // Mega-commits add noise and are quadratic in pair count: skip them (§3.4).
        if touched.len() > MAX_COMMIT_FILES {
            continue;
        }

        total_commits += 1;
        let files: Vec<FileId> = touched.into_iter().collect();
        for &f in &files {
            *freq.entry(f).or_insert(0) += 1;
        }
        // Every unordered pair (a < b) co-occurs in this commit.
        for (ai, &a) in files.iter().enumerate() {
            for &b in &files[ai + 1..] {
                let key = if a < b { (a, b) } else { (b, a) };
                *co.entry(key).or_insert(0) += 1;
            }
        }
    }

    if total_commits == 0 {
        store.replace_file_cochange(&[])?;
        return Ok(0);
    }

    // Emit a directional row for BOTH (a,b) and (b,a). support and lift are
    // symmetric; confidence is directional.
    //   support(a,b)    = co
    //   confidence(a->b)= co / freq[a]
    //   lift(a,b)       = confidence(a->b) / (freq[b] / total_commits)
    let total = total_commits as f64;
    let mut rows: Vec<FileCochangeRow> = Vec::with_capacity(co.len() * 2);
    for ((a, b), c) in co {
        if c < MIN_SUPPORT {
            continue;
        }
        let fa = *freq.get(&a).unwrap_or(&0) as f64;
        let fb = *freq.get(&b).unwrap_or(&0) as f64;
        if fa == 0.0 || fb == 0.0 {
            continue;
        }
        let cf = c as f64;
        let lift = (cf / fa) / (fb / total);
        rows.push(FileCochangeRow {
            a_file: a,
            b_file: b,
            support: c,
            confidence: cf / fa,
            lift,
        });
        rows.push(FileCochangeRow {
            a_file: b,
            b_file: a,
            support: c,
            confidence: cf / fb,
            lift,
        });
    }

    store.replace_file_cochange(&rows)?;
    Ok(rows.len())
}
