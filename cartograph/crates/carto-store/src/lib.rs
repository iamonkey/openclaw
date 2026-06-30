//! `carto-store` — SQLite access for the Cartograph index.
//!
//! Owns the on-disk schema (see [`schema`]) and the [`Store`] type, which is the
//! single write/read gateway to the index. Opened in WAL mode; the schema is
//! applied idempotently on every open and `schema_version` is recorded in
//! `meta`. See `docs/codebase-understanding/02-data-model.md` for the canonical
//! table definitions.

mod schema;
mod store;

pub use schema::SCHEMA_VERSION;
pub use store::Store;

#[cfg(test)]
mod tests {
    use super::*;
    use carto_model::{EdgeKind, RawRef, RawSymbol, SymbolKind, Tier};
    use std::collections::HashMap;

    /// Build a top-level function `RawSymbol` with a rendered signature.
    fn func(path: &str, name: &str) -> RawSymbol {
        RawSymbol {
            stable_key: carto_model::stable_key(path, &[], name, SymbolKind::Function),
            name: name.to_string(),
            fqn: Some(format!("{path}::{name}")),
            kind: SymbolKind::Function,
            signature: Some(format!("fn {name}()")),
            doc: Some(format!("doc for {name}")),
            start_byte: 0,
            end_byte: 10,
            start_row: 0,
            end_row: 1,
            parent_idx: None,
        }
    }

    /// Build a method `RawSymbol` nested under symbol at `parent_idx`.
    fn method(path: &str, container: &str, name: &str, parent_idx: usize) -> RawSymbol {
        RawSymbol {
            stable_key: carto_model::stable_key(path, &[container], name, SymbolKind::Method),
            name: name.to_string(),
            fqn: Some(format!("{path}::{container}::{name}")),
            kind: SymbolKind::Method,
            signature: Some(format!("fn {name}(&self)")),
            doc: None,
            start_byte: 20,
            end_byte: 40,
            start_row: 2,
            end_row: 4,
            parent_idx: Some(parent_idx),
        }
    }

    #[test]
    fn open_applies_schema_and_seeds_meta() {
        let store = Store::open_in_memory().unwrap();
        // schema_version recorded, generation seeded to 0.
        assert_eq!(store.generation().unwrap(), 0);
        // Tables exist: a query against each should succeed (return empty).
        assert!(store.list_files().unwrap().is_empty());
        assert!(store.all_symbol_ids().unwrap().is_empty());
        assert!(store.all_call_edges().unwrap().is_empty());
    }

    #[test]
    fn generation_bumps() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(store.generation().unwrap(), 0);
        assert_eq!(store.bump_generation().unwrap(), 1);
        assert_eq!(store.bump_generation().unwrap(), 2);
        assert_eq!(store.generation().unwrap(), 2);
    }

    #[test]
    fn upsert_and_read_back_symbols() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        let syms = vec![func(path, "alpha"), method(path, "alpha", "beta", 0)];
        let file_id = store
            .upsert_file(
                path,
                Some("rust"),
                Some("module a"),
                42,
                b"hash",
                &syms,
                &[],
                1,
            )
            .unwrap();

        let frec = store.file_by_path(path).unwrap().unwrap();
        assert_eq!(frec.id, file_id);
        assert_eq!(frec.lang.as_deref(), Some("rust"));
        assert_eq!(frec.purpose.as_deref(), Some("module a"));
        assert_eq!(frec.size_bytes, 42);
        assert_eq!(frec.content_hash, b"hash");
        assert_eq!(frec.generation, 1);

        let read = store.symbols_in_file(file_id).unwrap();
        assert_eq!(read.len(), 2);
        let alpha = &read[0];
        let beta = &read[1];
        assert_eq!(alpha.name, "alpha");
        assert_eq!(alpha.kind, SymbolKind::Function);
        assert_eq!(alpha.parent_id, None);
        assert_eq!(alpha.generation, 1);
        assert_eq!(beta.name, "beta");
        assert_eq!(beta.kind, SymbolKind::Method);
        // parent_idx 0 -> alpha's db id.
        assert_eq!(beta.parent_id, Some(alpha.id));

        // symbol_by_key round-trips.
        let by_key = store.symbol_by_key(&alpha.stable_key).unwrap().unwrap();
        assert_eq!(by_key.id, alpha.id);
    }

    #[test]
    fn upsert_is_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        let syms = vec![func(path, "alpha"), method(path, "alpha", "beta", 0)];

        let id1 = store
            .upsert_file(path, Some("rust"), None, 1, b"h1", &syms, &[], 1)
            .unwrap();
        let id2 = store
            .upsert_file(path, Some("rust"), None, 1, b"h2", &syms, &[], 2)
            .unwrap();

        // Re-upsert deletes prior rows; exactly one file and two symbols remain.
        assert_eq!(store.list_files().unwrap().len(), 1);
        // The first id was freed by the delete; SQLite may reuse that rowid, so
        // id1 == id2 is expected here. The idempotency guarantee is "no dup
        // rows", asserted below, not a fresh id.
        let _ = id1;
        let syms_now = store.symbols_in_file(id2).unwrap();
        assert_eq!(syms_now.len(), 2);
        // No orphan symbols anywhere.
        assert_eq!(store.all_symbol_ids().unwrap().len(), 2);
        // Updated metadata reflects second upsert.
        let frec = store.file_by_path(path).unwrap().unwrap();
        assert_eq!(frec.content_hash, b"h2");
        assert_eq!(frec.generation, 2);
    }

    #[test]
    fn skeleton_retrieval() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        let syms = vec![func(path, "alpha")];
        let file_id = store
            .upsert_file(path, None, None, 1, b"h", &syms, &[], 1)
            .unwrap();
        let alpha = &store.symbols_in_file(file_id).unwrap()[0];

        let skel = store.skeleton(alpha.id, Tier::Signature).unwrap().unwrap();
        assert_eq!(skel.tier, Tier::Signature);
        // text = signature + "\n" + doc.
        assert_eq!(skel.text, "fn alpha()\ndoc for alpha");
        assert_eq!(skel.token_est, carto_model::estimate_tokens(&skel.text));
        assert!(skel.token_est > 0);

        // No tier-0 skeleton was written for this slice.
        assert!(store.skeleton(alpha.id, Tier::Purpose).unwrap().is_none());
    }

    #[test]
    fn search_matches_name_and_fqn_case_insensitive() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        let syms = vec![func(path, "parseConfig"), func(path, "loadFile")];
        store
            .upsert_file(path, None, None, 1, b"h", &syms, &[], 1)
            .unwrap();

        // Case-insensitive substring on name.
        let hits = store.search_symbols("config", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "parseConfig");

        // Substring on fqn (path is in fqn).
        let hits = store.search_symbols("a.rs::load", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "loadFile");

        // Limit is honored.
        let hits = store.search_symbols("a.rs", 1).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn resolve_edges_unique_and_ambiguous() {
        let mut store = Store::open_in_memory().unwrap();

        // File 1: caller `main` calls `helper` (unique) and `dup` (ambiguous).
        let p1 = "src/main.rs";
        let f1 = vec![func(p1, "main"), func(p1, "dup")];
        let refs1 = vec![
            RawRef {
                src_idx: 0,
                target_name: "helper".into(),
                kind: EdgeKind::Call,
            },
            RawRef {
                src_idx: 0,
                target_name: "dup".into(),
                kind: EdgeKind::Call,
            },
        ];
        store
            .upsert_file(p1, None, None, 1, b"h1", &f1, &refs1, 1)
            .unwrap();

        // File 2: defines `helper` (unique target) and another `dup` (collision).
        let p2 = "src/util.rs";
        let f2 = vec![func(p2, "helper"), func(p2, "dup")];
        store
            .upsert_file(p2, None, None, 1, b"h2", &f2, &[], 1)
            .unwrap();

        let resolved = store.resolve_edges().unwrap();
        // `helper` resolves uniquely -> 1 resolved edge.
        assert_eq!(resolved, 1);

        // Total edges: 1 (helper) + 2 (dup -> both dup defs, ambiguous).
        let edges = store.all_call_edges().unwrap();
        assert_eq!(edges.len(), 3);

        // Idempotent: re-running clears + rebuilds to the same counts.
        let resolved2 = store.resolve_edges().unwrap();
        assert_eq!(resolved2, 1);
        assert_eq!(store.all_call_edges().unwrap().len(), 3);
    }

    #[test]
    fn who_calls_reverse_edges() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        // a -> b (call). who_calls(b) should return a.
        let syms = vec![func(path, "a"), func(path, "b")];
        let refs = vec![RawRef {
            src_idx: 0,
            target_name: "b".into(),
            kind: EdgeKind::Call,
        }];
        let fid = store
            .upsert_file(path, None, None, 1, b"h", &syms, &refs, 1)
            .unwrap();
        store.resolve_edges().unwrap();

        let read = store.symbols_in_file(fid).unwrap();
        let b = read.iter().find(|s| s.name == "b").unwrap();
        let a = read.iter().find(|s| s.name == "a").unwrap();

        let callers = store.who_calls(b.id, &[EdgeKind::Call]).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].id, a.id);

        // Empty kinds slice = any kind.
        let callers_any = store.who_calls(b.id, &[]).unwrap();
        assert_eq!(callers_any.len(), 1);

        // A kind that has no edges returns nothing.
        let none = store.who_calls(b.id, &[EdgeKind::Import]).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn neighborhood_depth_1_and_2() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        // Chain: a -> b -> c.
        let syms = vec![func(path, "a"), func(path, "b"), func(path, "c")];
        let refs = vec![
            RawRef {
                src_idx: 0,
                target_name: "b".into(),
                kind: EdgeKind::Call,
            },
            RawRef {
                src_idx: 1,
                target_name: "c".into(),
                kind: EdgeKind::Call,
            },
        ];
        let fid = store
            .upsert_file(path, None, None, 1, b"h", &syms, &refs, 1)
            .unwrap();
        store.resolve_edges().unwrap();
        let read = store.symbols_in_file(fid).unwrap();
        let a = read.iter().find(|s| s.name == "a").unwrap();
        let b = read.iter().find(|s| s.name == "b").unwrap();
        let c = read.iter().find(|s| s.name == "c").unwrap();

        // From b at depth 1: reaches a (incoming) and c (outgoing), both depth 1.
        let nb1 = store.neighborhood(b.id, 1).unwrap();
        let ids1: Vec<_> = nb1.iter().map(|(s, _)| s.id).collect();
        assert_eq!(nb1.len(), 2);
        assert!(ids1.contains(&a.id));
        assert!(ids1.contains(&c.id));
        for (_, d) in &nb1 {
            assert_eq!(*d, 1);
        }
        // Center excluded.
        assert!(!ids1.contains(&b.id));

        // From a at depth 2: reaches b (depth 1) and c (depth 2).
        let nb2 = store.neighborhood(a.id, 2).unwrap();
        let mut by_id: HashMap<i64, i64> = HashMap::new();
        for (s, d) in &nb2 {
            by_id.insert(s.id, *d);
        }
        assert_eq!(nb2.len(), 2);
        assert_eq!(by_id.get(&b.id), Some(&1));
        assert_eq!(by_id.get(&c.id), Some(&2));

        // From a at depth 1: only b.
        let nb_a1 = store.neighborhood(a.id, 1).unwrap();
        assert_eq!(nb_a1.len(), 1);
        assert_eq!(nb_a1[0].0.id, b.id);
    }

    #[test]
    fn write_ranks_then_search_ordering() {
        let mut store = Store::open_in_memory().unwrap();
        let path = "src/a.rs";
        // Two symbols both matching "fn".
        let syms = vec![func(path, "fnLow"), func(path, "fnHigh")];
        let fid = store
            .upsert_file(path, None, None, 1, b"h", &syms, &[], 1)
            .unwrap();
        let read = store.symbols_in_file(fid).unwrap();
        let low = read.iter().find(|s| s.name == "fnLow").unwrap();
        let high = read.iter().find(|s| s.name == "fnHigh").unwrap();

        let mut ranks = HashMap::new();
        ranks.insert(low.id, 0.1);
        ranks.insert(high.id, 0.9);
        store.write_ranks(&ranks).unwrap();

        // search orders by rank DESC -> fnHigh first.
        let hits = store.search_symbols("fn", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "fnHigh");
        assert_eq!(hits[0].rank, 0.9);
        assert_eq!(hits[1].name, "fnLow");
        assert_eq!(hits[1].rank, 0.1);
    }
}
