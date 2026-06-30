//! `carto` CLI — index / query subcommands over the Cartograph index.
//! (`serve` MCP daemon is a later milestone.)

use anyhow::{Context, Result};
use carto_core::{build_index, default_db_path, ensure_db_dir, IndexReader};
use carto_model::Tier;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "carto", version, about = "Token-efficient codebase index")]
struct Cli {
    /// Repo root to index / query against.
    #[arg(long, default_value = ".", global = true)]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the index for the repo root.
    Index,
    /// Incrementally resync the index (reparse only changed files).
    /// With paths, syncs just those (the edit hot path); else scans the tree.
    Sync {
        /// Repo-relative paths to sync; empty = whole-tree dirty scan.
        paths: Vec<String>,
        /// Also recompute PageRank now (otherwise ranks are marked dirty).
        #[arg(long)]
        rerank: bool,
    },
    /// Run a single retrieval tool and print JSON.
    Query {
        #[command(subcommand)]
        tool: Tool,
    },
}

#[derive(Subcommand)]
enum Tool {
    /// H1: tiered skeleton of a path ("" = repo root).
    Outline {
        #[arg(default_value = "")]
        path: String,
        #[arg(long, default_value_t = 1)]
        tier: i64,
        #[arg(long)]
        max_tokens: Option<i64>,
    },
    /// H1: full body of one symbol (by stable_key).
    Expand { key: String },
    /// H2: reverse callers/referencers of a symbol.
    WhoCalls { key: String },
    /// H2: bounded subgraph around a symbol.
    Neighborhood {
        key: String,
        #[arg(long, default_value_t = 2)]
        depth: u8,
    },
    /// Fuzzy symbol search.
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// H3: blast radius (static neighbors + git co-change).
    Impact {
        key: String,
        #[arg(long, default_value_t = 2)]
        static_depth: u8,
        #[arg(long, default_value_t = 1.5)]
        min_lift: f64,
        #[arg(long)]
        max_tokens: Option<i64>,
    },
    /// H4: structural diff of the working tree vs the last index.
    Diff,
    /// H5: budget-constrained retrieval plan for a task.
    Plan {
        task: String,
        #[arg(long, default_value_t = 6000)]
        budget: i64,
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.root.canonicalize().unwrap_or(cli.root.clone());
    let db = default_db_path(&root);

    match cli.cmd {
        Cmd::Index => {
            ensure_db_dir(&db)?;
            let t = std::time::Instant::now();
            let stats = build_index(&root, &db).context("build index")?;
            let ms = t.elapsed().as_millis();
            println!(
                "indexed {} files ({} parsed), {} symbols, {} edges, {} co-change pairs in {ms} ms (generation {})",
                stats.files_indexed,
                stats.files_parsed,
                stats.symbols,
                stats.edges_resolved,
                stats.cochange_pairs,
                stats.generation
            );
        }
        Cmd::Sync { paths, rerank } => {
            let report = if paths.is_empty() {
                carto_core::sync(&root, &db)?
            } else {
                carto_core::sync_paths(&root, &db, &paths)?
            };
            let rerank_ms = if rerank {
                Some(carto_core::recompute_ranks(&db)?)
            } else {
                None
            };
            println!(
                "synced: {} changed, {} added, {} deleted, {} clean-skipped \
                 (parse {:.2} ms, resolve {:.2} ms, total {:.2} ms, generation {}{})",
                report.changed,
                report.added,
                report.deleted,
                report.skipped_clean,
                report.parse_ms,
                report.resolve_ms,
                report.total_ms,
                report.generation,
                match rerank_ms {
                    Some(ms) => format!(", rerank {ms:.1} ms"),
                    None => ", rank dirty".to_string(),
                },
            );
        }
        Cmd::Query { tool } => {
            let reader =
                IndexReader::open(&db, &root).context("open index (did you run `carto index`?)")?;
            let json = run_tool(&reader, tool)?;
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }
    Ok(())
}

fn run_tool(reader: &IndexReader, tool: Tool) -> Result<serde_json::Value> {
    use serde_json::json;
    Ok(match tool {
        Tool::Outline {
            path,
            tier,
            max_tokens,
        } => {
            let tier = Tier::from_i64(tier).unwrap_or(Tier::Signature);
            serde_json::to_value(reader.outline(&path, tier, max_tokens)?)?
        }
        Tool::Expand { key } => serde_json::to_value(reader.expand(&key)?)?,
        Tool::WhoCalls { key } => {
            let callers = reader.who_calls(&key, &[])?;
            json!({
                "generation": reader.generation()?,
                "callers": callers.iter().map(symbol_brief).collect::<Vec<_>>(),
            })
        }
        Tool::Neighborhood { key, depth } => {
            let nodes = reader.neighborhood(&key, depth)?;
            json!({
                "generation": reader.generation()?,
                "center": key,
                "nodes": nodes.iter().map(|(s, d)| {
                    let mut b = symbol_brief(s);
                    b["depth"] = json!(d);
                    b
                }).collect::<Vec<_>>(),
            })
        }
        Tool::Search { query, limit } => {
            let matches = reader.search(&query, limit)?;
            json!({
                "generation": reader.generation()?,
                "matches": matches.iter().map(symbol_brief).collect::<Vec<_>>(),
            })
        }
        Tool::Impact {
            key,
            static_depth,
            min_lift,
            max_tokens,
        } => {
            let opts = carto_core::ImpactOpts {
                static_depth,
                min_lift,
                max_tokens,
            };
            serde_json::to_value(reader.impact(&key, &opts)?)?
        }
        Tool::Diff => serde_json::to_value(reader.diff_working()?)?,
        Tool::Plan {
            task,
            budget,
            dry_run,
        } => serde_json::to_value(reader.plan(&task, budget, dry_run)?)?,
    })
}

fn symbol_brief(s: &carto_model::Symbol) -> serde_json::Value {
    serde_json::json!({
        "key": s.stable_key,
        "name": s.name,
        "kind": s.kind.as_str(),
        "signature": s.signature,
        "rank": s.rank,
    })
}
