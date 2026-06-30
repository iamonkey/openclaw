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
                "indexed {} files ({} parsed), {} symbols, {} edges in {ms} ms (generation {})",
                stats.files_indexed,
                stats.files_parsed,
                stats.symbols,
                stats.edges_resolved,
                stats.generation
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
