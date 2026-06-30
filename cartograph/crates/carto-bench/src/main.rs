//! `carto-bench` — efficiency-frontier benchmark for the H1/H2 vertical slice.
//!
//! Compares Cartograph lazy hydration against the "naive grep + full-file"
//! baseline (`11-benchmark-harness.md`) on a fixed corpus + task set:
//!   - Token Cost (TC): tokens an agent must read to locate+read the target.
//!   - Task Resolution / localization@k: does the right symbol/file surface.
//!   - Sync Overhead (SO): index build time + per-query latency p50/p95.
//!
//! v1 caveat: SO uses full-build time (incremental `apply()` is a later
//! milestone), reported honestly as such.

use anyhow::{Context, Result};
use carto_core::{build_index, IndexReader};
use carto_model::{estimate_tokens, Tier};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Task {
    id: String,
    class: String,
    query: String,
    #[serde(default)]
    answer_files: Vec<String>,
    #[serde(default)]
    answer_symbols: Vec<String>,
}

#[derive(Debug, Clone)]
struct TaskResult {
    id: String,
    class: String,
    baseline_tokens: i64,
    carto_tokens: i64,
    baseline_file_hit: bool,
    carto_symbol_hit: bool,
    carto_file_hit: bool,
    reduction_pct: f64,
}

const TOPK: usize = 10;
const STOPWORDS: &[&str] = &[
    "where",
    "what",
    "which",
    "when",
    "does",
    "done",
    "the",
    "this",
    "that",
    "implemented",
    "implementation",
    "breaks",
    "break",
    "change",
    "changed",
    "changes",
    "how",
    "are",
    "and",
    "for",
    "with",
    "into",
    "from",
    "result",
    "results",
    "function",
    "method",
    "class",
    "used",
    "use",
    "uses",
    "called",
    "call",
    "calls",
    "happen",
    "happens",
    "code",
    "logic",
];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let corpus = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "cartograph/bench/corpus".into()),
    );
    let tasks_path = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "cartograph/bench/tasks.json".into()),
    );
    let out_dir = PathBuf::from(args.next().unwrap_or_else(|| "cartograph/bench".into()));

    let corpus = corpus.canonicalize().context("corpus path")?;
    let tasks: Vec<Task> = serde_json::from_slice(
        &std::fs::read(&tasks_path).with_context(|| format!("read {}", tasks_path.display()))?,
    )
    .context("parse tasks.json")?;

    // ── Build the index (SO: full-build time) ───────────────────────────────
    let tmp = out_dir.join(".carto-bench");
    std::fs::create_dir_all(&tmp)?;
    let db = tmp.join("index.db");
    let _ = std::fs::remove_file(&db);
    let t0 = Instant::now();
    let stats = build_index(&corpus, &db).context("build index")?;
    let index_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let reader = IndexReader::open(&db, &corpus)?;

    // Corpus stats + baseline file texts.
    let files = read_corpus(&corpus);
    let corpus_loc: usize = files.iter().map(|(_, t)| t.lines().count()).sum();
    let corpus_tokens: i64 = files.iter().map(|(_, t)| estimate_tokens(t)).sum();

    // ── Per-task evaluation + query latency samples ─────────────────────────
    let mut results = Vec::new();
    let mut latencies_ms: Vec<f64> = Vec::new();
    for task in &tasks {
        let terms = derive_terms(&task.query);

        // Baseline: read every file containing any term, in full.
        let mut baseline_tokens = 0i64;
        let mut baseline_file_hit = false;
        let mut baseline_files: BTreeSet<String> = BTreeSet::new();
        for (path, text) in &files {
            let lc = text.to_lowercase();
            if terms.iter().any(|t| lc.contains(t)) {
                baseline_tokens += estimate_tokens(text);
                baseline_files.insert(path.clone());
            }
        }
        for af in &task.answer_files {
            if baseline_files.contains(af) {
                baseline_file_hit = true;
            }
        }

        // Carto: search symbols (locate), then read just the top candidate.
        let q0 = Instant::now();
        let mut matches: Vec<carto_model::Symbol> = Vec::new();
        let mut seen = BTreeSet::new();
        for term in &terms {
            for s in reader.search(term, TOPK as i64)? {
                if seen.insert(s.stable_key.clone()) {
                    matches.push(s);
                }
            }
        }
        matches.sort_by(|a, b| b.rank.total_cmp(&a.rank));
        matches.truncate(TOPK);

        // Tokens the agent reads to LOCATE: the rendered match list (key + sig).
        let locate_tokens: i64 = matches
            .iter()
            .map(|s| {
                estimate_tokens(&format!(
                    "{} {}",
                    s.stable_key,
                    s.signature.as_deref().unwrap_or("")
                ))
            })
            .sum();

        // Tokens to READ the chosen target: expand the best match (or the
        // ground-truth symbol's neighborhood for what-breaks).
        let mut read_tokens = 0i64;
        if task.class == "what-breaks" {
            // impact proxy: neighborhood signatures of the best match
            if let Some(best) = matches.first() {
                let nb = reader.neighborhood(&best.stable_key, 2)?;
                read_tokens = nb
                    .iter()
                    .map(|(s, _)| {
                        estimate_tokens(s.signature.as_deref().unwrap_or(s.name.as_str()))
                    })
                    .sum();
            }
        } else if let Some(best) = matches.first() {
            read_tokens = reader.expand(&best.stable_key)?.token_est;
        }
        latencies_ms.push(q0.elapsed().as_secs_f64() * 1000.0);

        let carto_tokens = locate_tokens + read_tokens;
        let match_keys: BTreeSet<&str> = matches.iter().map(|s| s.stable_key.as_str()).collect();
        let carto_symbol_hit = task
            .answer_symbols
            .iter()
            .any(|a| match_keys.contains(a.as_str()));
        let match_files: BTreeSet<String> = matches
            .iter()
            .map(|s| s.stable_key.split('#').next().unwrap_or("").to_string())
            .collect();
        let carto_file_hit = task.answer_files.iter().any(|a| match_files.contains(a));

        let reduction_pct = if baseline_tokens > 0 {
            100.0 * (1.0 - carto_tokens as f64 / baseline_tokens as f64)
        } else {
            0.0
        };

        results.push(TaskResult {
            id: task.id.clone(),
            class: task.class.clone(),
            baseline_tokens,
            carto_tokens,
            baseline_file_hit,
            carto_symbol_hit,
            carto_file_hit,
            reduction_pct,
        });
    }

    // Extra latency sampling on outline (the H1 hot path).
    for _ in 0..50 {
        let q = Instant::now();
        let _ = reader.outline("", Tier::Signature, Some(2000))?;
        latencies_ms.push(q.elapsed().as_secs_f64() * 1000.0);
    }

    let report = render_report(
        &corpus,
        files.len(),
        corpus_loc,
        corpus_tokens,
        &stats,
        index_ms,
        &results,
        &mut latencies_ms,
    );
    let report_path = out_dir.join("REPORT.md");
    std::fs::write(&report_path, &report)?;
    let results_json = out_dir.join("results.json");
    std::fs::write(
        &results_json,
        results_to_json(&results, index_ms, &latencies_ms),
    )?;

    println!("{report}");
    println!(
        "\nwrote {} and {}",
        report_path.display(),
        results_json.display()
    );
    Ok(())
}

fn read_corpus(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dent in walkdir::WalkDir::new(root).into_iter().flatten() {
        if !dent.file_type().is_file() {
            continue;
        }
        let path = dent.path();
        let is_ts = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "ts" | "tsx" | "mts" | "cts"))
            .unwrap_or(false);
        if !is_ts {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, text));
        }
    }
    out.sort();
    out
}

/// Derive symmetric search/grep terms from a query: content words (len>=4, not
/// stopwords), each clipped to a <=6 char prefix so both engines get the same
/// recall surface (e.g. "pagination" -> "pagina", matching symbol "paginate").
fn derive_terms(query: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for raw in query.split(|c: char| !c.is_alphanumeric()) {
        let w = raw.to_lowercase();
        if w.len() < 4 || STOPWORDS.contains(&w.as_str()) {
            continue;
        }
        let prefix: String = w.chars().take(6).collect();
        terms.insert(prefix);
    }
    if terms.is_empty() {
        // fall back to any word >=3 chars
        for raw in query.split(|c: char| !c.is_alphanumeric()) {
            if raw.len() >= 3 {
                terms.insert(raw.to_lowercase());
            }
        }
    }
    terms.into_iter().collect()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[allow(clippy::too_many_arguments)]
fn render_report(
    corpus: &Path,
    file_count: usize,
    loc: usize,
    corpus_tokens: i64,
    stats: &carto_core::BuildStats,
    index_ms: f64,
    results: &[TaskResult],
    latencies: &mut Vec<f64>,
) -> String {
    latencies.sort_by(|a, b| a.total_cmp(b));
    let p50 = percentile(latencies, 50.0);
    let p95 = percentile(latencies, 95.0);

    let n = results.len().max(1) as f64;
    let sum_base: i64 = results.iter().map(|r| r.baseline_tokens).sum();
    let sum_carto: i64 = results.iter().map(|r| r.carto_tokens).sum();
    let mean_reduction = results.iter().map(|r| r.reduction_pct).sum::<f64>() / n;
    let overall_reduction = if sum_base > 0 {
        100.0 * (1.0 - sum_carto as f64 / sum_base as f64)
    } else {
        0.0
    };
    let carto_sym_hits = results.iter().filter(|r| r.carto_symbol_hit).count();
    let carto_file_hits = results.iter().filter(|r| r.carto_file_hit).count();
    let base_file_hits = results.iter().filter(|r| r.baseline_file_hit).count();
    let total = results.len();

    let mut s = String::new();
    s.push_str("# Cartograph Benchmark Report (H1+H2 vertical slice)\n\n");
    s.push_str(&format!("Corpus: `{}`\n\n", corpus.display()));
    s.push_str("## Corpus and index\n\n");
    s.push_str("| metric | value |\n|---|---|\n");
    s.push_str(&format!("| files (TypeScript) | {file_count} |\n"));
    s.push_str(&format!("| lines of code | {loc} |\n"));
    s.push_str(&format!("| full-corpus tokens (est) | {corpus_tokens} |\n"));
    s.push_str(&format!("| symbols indexed | {} |\n", stats.symbols));
    s.push_str(&format!("| edges resolved | {} |\n", stats.edges_resolved));
    s.push_str(&format!(
        "| index build time | {index_ms:.1} ms (full build) |\n\n"
    ));

    s.push_str("## Headline: token cost vs naive grep + full-file\n\n");
    s.push_str(&format!(
        "- **Overall token reduction: {overall_reduction:.1}%** ({sum_base} baseline -> {sum_carto} Cartograph tokens across {total} tasks)\n"
    ));
    s.push_str(&format!(
        "- Mean per-task reduction: {mean_reduction:.1}%\n"
    ));
    s.push_str(&format!(
        "- Context precision: Cartograph reads {} of what naive reads\n\n",
        if sum_base > 0 {
            format!("{:.1}%", 100.0 * sum_carto as f64 / sum_base as f64)
        } else {
            "n/a".into()
        }
    ));

    s.push_str("## Task resolution (localization)\n\n");
    s.push_str(&format!(
        "- Cartograph symbol-localization@{TOPK}: {carto_sym_hits}/{total}\n"
    ));
    s.push_str(&format!(
        "- Cartograph file-localization@{TOPK}: {carto_file_hits}/{total}\n"
    ));
    s.push_str(&format!(
        "- Baseline file-localization (grep): {base_file_hits}/{total}\n\n"
    ));

    s.push_str("## Sync overhead (SO)\n\n");
    s.push_str("| metric | value | gate |\n|---|---|---|\n");
    s.push_str(&format!("| query latency p50 | {p50:.2} ms | - |\n"));
    s.push_str(&format!(
        "| query latency p95 | {p95:.2} ms | informational |\n"
    ));
    s.push_str(&format!(
        "| full index build | {index_ms:.1} ms | (incremental apply is a later milestone) |\n\n"
    ));

    s.push_str("## Per-task detail\n\n");
    s.push_str("| task | class | baseline tok | carto tok | reduction | sym hit | file hit |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for r in results {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {:.1}% | {} | {} |\n",
            r.id,
            r.class,
            r.baseline_tokens,
            r.carto_tokens,
            r.reduction_pct,
            if r.carto_symbol_hit { "yes" } else { "no" },
            if r.carto_file_hit { "yes" } else { "no" },
        ));
    }
    s.push('\n');
    s.push_str("> Token estimates use ceil(bytes/4), identical on both sides, so ratios are apples-to-apples.\n");
    s
}

fn results_to_json(results: &[TaskResult], index_ms: f64, latencies: &[f64]) -> String {
    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let arr: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id, "class": r.class,
                "baseline_tokens": r.baseline_tokens,
                "carto_tokens": r.carto_tokens,
                "reduction_pct": r.reduction_pct,
                "carto_symbol_hit": r.carto_symbol_hit,
                "carto_file_hit": r.carto_file_hit,
                "baseline_file_hit": r.baseline_file_hit,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "index_build_ms": index_ms,
        "query_latency_p50_ms": percentile(&sorted, 50.0),
        "query_latency_p95_ms": percentile(&sorted, 95.0),
        "tasks": arr,
    }))
    .unwrap_or_else(|_| "{}".into())
}
