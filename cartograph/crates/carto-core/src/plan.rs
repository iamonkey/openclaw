//! H5 — Adaptive Context Budgeter (`09-h5-adaptive-context-budgeter.md`).
//!
//! Classifies a task, picks a tool strategy across H1/H2/H3, and greedily fills
//! a token budget by value/cost ratio (rank- and weight-driven), returning both
//! an auditable plan and the hydrated, pre-truncated context.

use crate::impact::ImpactOpts;
use crate::reader::IndexReader;
use anyhow::Result;
use carto_model::{
    estimate_tokens, ContextChunk, PlanStep, RetrievalPlan, Symbol, TaskClass, Tier,
};

/// Plan and (unless `dry_run`) hydrate retrieval for `task` under `budget`.
pub fn plan(reader: &IndexReader, task: &str, budget: i64, dry_run: bool) -> Result<RetrievalPlan> {
    let task_class = classify(task);
    let terms = search_terms(task);

    let mut steps: Vec<PlanStep> = Vec::new();
    // Candidate chunks are produced in descending value order; the greedy fill
    // below adds them while the budget allows, skipping a chunk that would
    // overflow and trying the next (smaller / lower-value) candidate.
    let mut candidates: Vec<ContextChunk> = Vec::new();

    match task_class {
        TaskClass::WhereIs | TaskClass::BugLocalize => {
            let matches = locate(reader, &terms, &mut steps);
            push_locate_and_bodies(reader, &matches, &mut steps, &mut candidates)?;
        }
        TaskClass::WhatBreaks => {
            let matches = locate(reader, &terms, &mut steps);
            // Add the cheap locate chunk first (high value / low cost).
            push_locate_chunk(&matches, &mut candidates);
            if let Some(best) = matches.first() {
                let best_key = best.stable_key.clone();
                steps.push(PlanStep {
                    tool: "impact".into(),
                    args: format!("impact key={best_key} (defaults)"),
                    why: "blast radius of the located symbol drives what could break".into(),
                });
                let set = reader.impact(&best_key, &ImpactOpts::default())?;
                // Impact items are already weight-ranked; preserve that order.
                for item in set.items {
                    let text = format!("{}\n  {}", item.key, item.reason);
                    candidates.push(ContextChunk {
                        source_tool: "impact".into(),
                        key: Some(item.key),
                        token_est: estimate_tokens(&text),
                        text,
                    });
                }
            }
        }
        TaskClass::Default => {
            steps.push(PlanStep {
                tool: "outline".into(),
                args: format!("outline path=\"\" tier=signature max_tokens={budget}"),
                why: "no specific target; survey the repo signature skeleton".into(),
            });
            let outline = reader.outline("", Tier::Signature, Some(budget))?;
            let text = render_outline(&outline);
            candidates.push(ContextChunk {
                source_tool: "outline".into(),
                key: None,
                token_est: estimate_tokens(&text),
                text,
            });
        }
    }

    // Greedy budget fill: take candidates in their (descending-value) order,
    // skipping any that would overflow so a later, cheaper candidate can fit.
    let mut context: Vec<ContextChunk> = Vec::new();
    let mut spent_est: i64 = 0;
    if !dry_run {
        for chunk in candidates {
            if spent_est + chunk.token_est > budget {
                continue;
            }
            spent_est += chunk.token_est;
            context.push(chunk);
        }
    }

    Ok(RetrievalPlan {
        generation: reader.generation()?,
        task_class,
        budget,
        spent_est,
        steps,
        context,
    })
}

/// Keyword heuristic → `TaskClass`. Ordered so bug/impact signals win over a
/// generic "where", per the H5 design.
fn classify(task: &str) -> TaskClass {
    let t = task.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));

    // Bug localization first: an "off-by-one bug in pagination" mentions neither
    // break nor where but must route to BugLocalize.
    if has(&[
        "bug",
        "fix",
        "error",
        "wrong",
        "off-by",
        "off by",
        "crash",
        "fail",
        "incorrect",
        "broken",
    ]) {
        return TaskClass::BugLocalize;
    }
    // Then impact/ripple questions.
    if has(&[
        "break",
        "impact",
        "affect",
        "depend",
        "ripple",
        "change",
        "consequence",
        "downstream",
    ]) {
        return TaskClass::WhatBreaks;
    }
    // Then plain location questions.
    if has(&["where", "find", "locate", "implement", "defined", "lives"]) {
        return TaskClass::WhereIs;
    }
    TaskClass::Default
}

/// Derive search terms: lowercase, split on non-alphanumeric, drop short words
/// and stopwords, clip each to a <=6-char prefix so "pagination" matches a
/// "paginate" symbol via substring search.
fn search_terms(task: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "where",
        "what",
        "which",
        "when",
        "does",
        "find",
        "have",
        "this",
        "that",
        "with",
        "from",
        "into",
        "code",
        "function",
        "implemented",
        "implement",
        "happen",
        "happens",
        "would",
        "could",
        "should",
        "about",
        "there",
    ];
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for word in task
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(w))
    {
        // Clip to a <=6-char prefix (chars, not bytes, to stay UTF-8 safe).
        let prefix: String = word.chars().take(6).collect();
        if seen.insert(prefix.clone()) {
            out.push(prefix);
        }
    }
    out
}

/// `search` each term (limit 10), union, dedup by key, sort by rank DESC.
/// Records one `PlanStep` per term searched.
fn locate(reader: &IndexReader, terms: &[String], steps: &mut Vec<PlanStep>) -> Vec<Symbol> {
    let mut by_key: std::collections::HashMap<String, Symbol> = std::collections::HashMap::new();
    for term in terms {
        steps.push(PlanStep {
            tool: "search".into(),
            args: format!("search query=\"{term}\" limit=10"),
            why: "locate symbols matching a task keyword".into(),
        });
        if let Ok(hits) = reader.search(term, 10) {
            for s in hits {
                by_key.entry(s.stable_key.clone()).or_insert(s);
            }
        }
    }
    let mut matches: Vec<Symbol> = by_key.into_values().collect();
    // rank DESC; stable tiebreak by key so output is deterministic.
    matches.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then_with(|| a.stable_key.cmp(&b.stable_key))
    });
    matches
}

/// The cheap "locate" chunk: rendered `key  signature` lines for the ranked
/// match list. High value / low cost, so it is the first candidate.
fn push_locate_chunk(matches: &[Symbol], candidates: &mut Vec<ContextChunk>) {
    if matches.is_empty() {
        return;
    }
    let mut text = String::new();
    for s in matches {
        let sig = s.signature.as_deref().unwrap_or("");
        text.push_str(&s.stable_key);
        if !sig.is_empty() {
            text.push_str("  ");
            text.push_str(sig);
        }
        text.push('\n');
    }
    candidates.push(ContextChunk {
        source_tool: "search".into(),
        key: matches.first().map(|s| s.stable_key.clone()),
        token_est: estimate_tokens(&text),
        text,
    });
}

/// Locate chunk, then `expand` the top matches (bodies) in rank order as
/// further candidates. Records an `expand` `PlanStep` per body fetched.
fn push_locate_and_bodies(
    reader: &IndexReader,
    matches: &[Symbol],
    steps: &mut Vec<PlanStep>,
    candidates: &mut Vec<ContextChunk>,
) -> Result<()> {
    push_locate_chunk(matches, candidates);
    // Expand up to the top 3 matches; the greedy fill decides which actually fit.
    for s in matches.iter().take(3) {
        steps.push(PlanStep {
            tool: "expand".into(),
            args: format!("expand key=\"{}\"", s.stable_key),
            why: "read the full body of a top-ranked match".into(),
        });
        if let Ok(span) = reader.expand(&s.stable_key) {
            candidates.push(ContextChunk {
                source_tool: "expand".into(),
                key: Some(s.stable_key.clone()),
                token_est: estimate_tokens(&span.source),
                text: span.source,
            });
        }
    }
    Ok(())
}

/// Render an `Outline` into a flat signature listing for the Default strategy.
fn render_outline(outline: &carto_model::Outline) -> String {
    let mut text = String::new();
    for entry in &outline.entries {
        if let Some(p) = &entry.purpose {
            text.push_str(&format!("// {}: {}\n", entry.path, p));
        } else {
            text.push_str(&format!("// {}\n", entry.path));
        }
        for sym in &entry.symbols {
            let sig = sym.signature.as_deref().unwrap_or("");
            text.push_str("  ");
            text.push_str(&sym.key);
            if !sig.is_empty() {
                text.push_str("  ");
                text.push_str(sig);
            }
            text.push('\n');
        }
    }
    text
}
