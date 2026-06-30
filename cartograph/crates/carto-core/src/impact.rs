//! H3 — Impact-Radius fusion (`06-h3-impact-radius.md`).
//!
//! `impact(symbol)` = static neighbors (H2) ∪ git co-change neighbors (H3),
//! merged into one weight-ranked, budget-truncated list. The co-change signal
//! is mined into `file_cochange` by the `carto-git` crate; this module fuses it
//! with the static graph at query time.
//!
//! NOTE: body is a stub pending the H3 implementation.

use crate::reader::IndexReader;
use anyhow::{anyhow, Result};
use carto_model::{estimate_tokens, ImpactItem, ImpactSet, ImpactSource};
use std::collections::HashMap;

/// Options for `impact`.
#[derive(Debug, Clone)]
pub struct ImpactOpts {
    /// Static neighborhood radius (H2).
    pub static_depth: u8,
    /// Minimum co-change lift to include a file's symbols.
    pub min_lift: f64,
    /// Token budget for the returned set (None = unbounded).
    pub max_tokens: Option<i64>,
}

impl Default for ImpactOpts {
    fn default() -> Self {
        ImpactOpts {
            static_depth: 2,
            min_lift: 1.5,
            max_tokens: Some(2000),
        }
    }
}

/// How many top-ranked symbols of a co-changing file to surface as impact items.
const COCHANGE_SYMBOLS_PER_FILE: usize = 3;
/// How many co-changing files to pull from the store before fan-out to symbols.
const COCHANGE_FILE_LIMIT: i64 = 20;

/// Compute the blast radius for `key`: H2 static neighbors fused with H3 git
/// co-change, merged into one weight-ranked, budget-truncated list
/// (`06-h3-impact-radius.md` §4).
pub fn compute(reader: &IndexReader, key: &str, opts: &ImpactOpts) -> Result<ImpactSet> {
    let store = reader.store();
    let center = reader
        .symbol_by_key(key)?
        .ok_or_else(|| anyhow!("unknown symbol: {key}"))?;

    let mut items: Vec<ImpactItem> = Vec::new();

    // ── Static side: H2 bounded neighborhood (§4.1). Weight decays with hop
    // distance so closer neighbors rank higher.
    for (sym, depth) in reader.neighborhood(key, opts.static_depth)? {
        items.push(ImpactItem {
            key: sym.stable_key,
            source: ImpactSource::Static,
            reason: format!("static depth {depth}"),
            weight: 1.0 / (depth as f64 + 1.0),
        });
    }

    // ── Co-change side: files that historically changed with the center's file
    // (§4.2), then that file's top-ranked symbols. Weight squashes lift into
    // [0,1) so a churny lift~1 pair stays modest.
    for (b_file, lift) in
        store.cochanging_files(center.file_id, opts.min_lift, COCHANGE_FILE_LIMIT)?
    {
        let mut syms = store.symbols_in_files(&[b_file])?; // already rank DESC
        syms.truncate(COCHANGE_SYMBOLS_PER_FILE);
        let weight = (lift / (1.0 + lift)).min(1.0);
        for sym in syms {
            items.push(ImpactItem {
                key: sym.stable_key,
                source: ImpactSource::Cochange,
                reason: format!("changed together (lift {lift:.1})"),
                weight,
            });
        }
    }

    // ── Merge: drop the center, dedup by key keeping the MAX weight (Static
    // reason wins ties since it is inserted first), then sort by weight DESC.
    let mut best: HashMap<String, ImpactItem> = HashMap::new();
    for item in items {
        if item.key == center.stable_key {
            continue;
        }
        match best.get(&item.key) {
            Some(existing) if existing.weight >= item.weight => {}
            _ => {
                best.insert(item.key.clone(), item);
            }
        }
    }
    let mut merged: Vec<ImpactItem> = best.into_values().collect();
    // Sort by weight DESC; tie-break on key for deterministic output.
    merged.sort_by(|a, b| {
        b.weight
            .total_cmp(&a.weight)
            .then_with(|| a.key.cmp(&b.key))
    });

    // ── Budget truncation (shared protocol): fill until the next item would
    // exceed max_tokens, then mark the rest dropped.
    let budget = opts.max_tokens.unwrap_or(i64::MAX);
    let mut kept: Vec<ImpactItem> = Vec::new();
    let mut spent: i64 = 0;
    let mut dropped: i64 = 0;
    let mut truncated = false;
    for item in merged {
        // Cost = token estimate of the item symbol's signature (fall back to its
        // name when it has no rendered signature).
        let cost = match reader.symbol_by_key(&item.key)? {
            Some(sym) => estimate_tokens(sym.signature.as_deref().unwrap_or(&sym.name)),
            None => estimate_tokens(&item.key),
        }
        .max(1);
        if spent + cost > budget {
            truncated = true;
            dropped += 1;
            continue;
        }
        spent += cost;
        kept.push(item);
    }

    Ok(ImpactSet {
        generation: reader.generation()?,
        cochange_available: store.file_cochange_count()? > 0,
        items: kept,
        truncated,
        dropped,
    })
}
