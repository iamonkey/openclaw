//! H5 — Adaptive Context Budgeter (`09-h5-adaptive-context-budgeter.md`).
//!
//! Classifies a task, picks a tool strategy across H1/H2/H3, and greedily fills
//! a token budget by value/cost ratio (rank- and weight-driven), returning both
//! an auditable plan and the hydrated, pre-truncated context.
//!
//! NOTE: body is a stub pending the H5 implementation.

use crate::reader::IndexReader;
use anyhow::Result;
use carto_model::{RetrievalPlan, TaskClass};

/// Plan and (unless `dry_run`) hydrate retrieval for `task` under `budget`.
/// STUB: returns an empty plan.
pub fn plan(reader: &IndexReader, _task: &str, budget: i64, _dry_run: bool) -> Result<RetrievalPlan> {
    Ok(RetrievalPlan {
        generation: reader.generation()?,
        task_class: TaskClass::Default,
        budget,
        spent_est: 0,
        steps: Vec::new(),
        context: Vec::new(),
    })
}
