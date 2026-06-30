//! Minimal PageRank over the call/reference graph (H2).
//!
//! Computes centrality so the retrieval tools can rank symbols by
//! "architectural gravity" and the budget protocol can truncate low-rank
//! frontier first. This is the simple iterative formulation; the spec's
//! incremental/localized re-rank (`10-incremental-sync.md`) is a later
//! optimization — a full pass is fine at vertical-slice corpus sizes.

use carto_model::SymbolId;
use std::collections::HashMap;

/// Run PageRank. `nodes` is every symbol id; `edges` are directed (src -> dst)
/// reference edges. Returns a normalized rank per node (sums to ~1.0).
///
/// Edge direction: `src` references `dst`, so rank flows toward referenced
/// (depended-upon) symbols — a heavily-called util accrues high rank.
pub fn pagerank(
    nodes: &[SymbolId],
    edges: &[(SymbolId, SymbolId)],
    damping: f64,
    iterations: usize,
) -> HashMap<SymbolId, f64> {
    let n = nodes.len();
    if n == 0 {
        return HashMap::new();
    }
    let base = 1.0 / n as f64;
    let mut rank: HashMap<SymbolId, f64> = nodes.iter().map(|&id| (id, base)).collect();

    // out-degree and adjacency (src -> [dst])
    let mut out: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
    for &(s, d) in edges {
        // ignore edges to/from unknown nodes (defensive)
        if rank.contains_key(&s) && rank.contains_key(&d) {
            out.entry(s).or_default().push(d);
        }
    }

    let teleport = (1.0 - damping) / n as f64;
    for _ in 0..iterations {
        let mut next: HashMap<SymbolId, f64> = nodes.iter().map(|&id| (id, teleport)).collect();
        let mut dangling = 0.0;
        for &id in nodes {
            let r = rank[&id];
            match out.get(&id) {
                Some(dsts) if !dsts.is_empty() => {
                    let share = damping * r / dsts.len() as f64;
                    for &d in dsts {
                        *next.get_mut(&d).unwrap() += share;
                    }
                }
                // dangling node: redistribute its rank uniformly
                _ => dangling += damping * r,
            }
        }
        if dangling > 0.0 {
            let spread = dangling / n as f64;
            for v in next.values_mut() {
                *v += spread;
            }
        }
        rank = next;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_outranks_leaf() {
        // 1,2,3 all reference 4 (a hub). 4 should rank highest.
        let nodes = vec![1, 2, 3, 4];
        let edges = vec![(1, 4), (2, 4), (3, 4)];
        let r = pagerank(&nodes, &edges, 0.85, 50);
        let hub = r[&4];
        for leaf in [1, 2, 3] {
            assert!(hub > r[&leaf], "hub {hub} should beat leaf {}", r[&leaf]);
        }
        let sum: f64 = r.values().sum();
        assert!((sum - 1.0).abs() < 1e-6, "ranks should sum to 1, got {sum}");
    }

    #[test]
    fn empty_graph_is_safe() {
        assert!(pagerank(&[], &[], 0.85, 10).is_empty());
    }
}
