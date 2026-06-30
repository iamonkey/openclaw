//! H3 — Impact-Radius fusion (`06-h3-impact-radius.md`).
//!
//! `impact(symbol)` = static neighbors (H2) ∪ git co-change neighbors (H3),
//! merged into one weight-ranked, budget-truncated list. The co-change signal
//! is mined into `file_cochange` by the `carto-git` crate; this module fuses it
//! with the static graph at query time.
//!
//! NOTE: body is a stub pending the H3 implementation.

use crate::reader::IndexReader;
use anyhow::Result;
use carto_model::ImpactSet;

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

/// Compute the blast radius for `key`. STUB: returns an empty set whose
/// `cochange_available` reflects whether any history was mined.
pub fn compute(reader: &IndexReader, _key: &str, _opts: &ImpactOpts) -> Result<ImpactSet> {
    Ok(ImpactSet {
        generation: reader.generation()?,
        cochange_available: reader.store().file_cochange_count()? > 0,
        items: Vec::new(),
        truncated: false,
        dropped: 0,
    })
}
