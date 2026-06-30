# Cartograph

A language-agnostic tool that lets LLMs navigate large codebases with **maximum
accuracy and minimum token consumption**, via reactive *lazy context hydration*
— the agent pulls signatures first and full bodies only on demand, instead of
reading whole files.

This directory is the **working implementation** of the H1 + H2 vertical slice
specified in [`../docs/codebase-understanding/`](../docs/codebase-understanding/).
Rust workspace; SQLite index; tree-sitter extraction.

## Status

Implemented and tested (29 passing tests across the workspace):

| Capability | Spec | Status |
|---|---|---|
| Tiered repo map (file tree + purpose → signatures → bodies) | H1 | ✅ `outline`, `expand` |
| Symbol graph + PageRank centrality | H2 | ✅ `get_symbol`/`search`, `who_calls`, `neighborhood` |
| TypeScript extraction (symbols, signatures, docstrings, refs) | — | ✅ tree-sitter |
| SQLite index (WAL, recursive-CTE graph queries) | doc 02 | ✅ |
| CLI (`index`, `query`) | doc 01 | ✅ |
| Benchmark harness vs naive grep+full-file | doc 11 | ✅ `carto-bench` |
| Incremental sync (file-watch, dirty reparse) | H4 / doc 10 | ⏳ full-build only in v1 |
| MCP stdio server (`serve`) | doc 03 | ⏳ CLI `query` exercises the same code path |
| Impact radius (git co-change) | H3 | ⏳ next milestone |
| Adaptive budgeter | H5 | ⏳ greedy budget protocol present per-tool |

The CLI `query` subcommands return the exact JSON the MCP tools will serve, so
the daemon is a transport wrapper over an already-working tool surface.

## Layout

```
cartograph/
├── crates/
│   ├── carto-model/   # shared type contract (mirrors docs/.../02-data-model.md)
│   ├── carto-store/   # SQLite schema + CRUD + edge resolution + neighborhood CTE
│   ├── carto-parse/   # tree-sitter TypeScript → symbols/refs/skeletons
│   ├── carto-core/    # index build orchestration + PageRank + IndexReader
│   ├── carto-cli/     # `carto` binary: index / query
│   └── carto-bench/   # efficiency-frontier benchmark + report generator
└── bench/             # benchmark corpus (taskman API, 40 files) + tasks + report
```

## Build and run

```sh
cd cartograph
cargo build --release
cargo test                       # 29 tests

# index a repo (writes <root>/.carto/index.db)
cargo run -p carto-cli -- --root /path/to/repo index

# H1: tiered outline of the whole repo (signatures, rank-ordered, budgeted)
cargo run -p carto-cli -- --root /path/to/repo query outline "" --tier 1 --max-tokens 2000

# H1: full body of one symbol
cargo run -p carto-cli -- --root /path/to/repo query expand "src/util/paginate.ts#paginate:function"

# H2: who references a symbol / its neighborhood / fuzzy search
cargo run -p carto-cli -- --root /path/to/repo query who-calls "src/util/paginate.ts#paginate:function"
cargo run -p carto-cli -- --root /path/to/repo query neighborhood "src/http/router.ts#dispatch:function" --depth 2
cargo run -p carto-cli -- --root /path/to/repo query search paginate
```

## Benchmark results

Run on the bundled `bench/corpus` (40 TypeScript files, 2178 LOC, 211 symbols,
612 edges). Full report: [`bench/REPORT.md`](./bench/REPORT.md); raw data:
`bench/results.json`. Reproduce:

```sh
cargo run -p carto-bench -- bench/corpus bench/tasks.json bench
```

Headline (Cartograph lazy hydration vs naive grep + full-file baseline, 14 tasks):

| metric | result |
|---|---|
| **Overall token reduction** | **96.6%** (195,387 → 6,566 tokens) |
| Context precision | Cartograph reads **3.4%** of what naive reads |
| Query latency p50 / p95 | **1.7 ms / 2.5 ms** (interactive gate is 200 ms p95) |
| Full index build (2178 LOC) | 288 ms |
| Symbol localization@10 | 8/14 |
| File localization@10 | 12/14 (vs grep 14/14 — but at ~30× the tokens) |

Both sides count tokens with the same `ceil(bytes/4)` estimator, so the ratios
are apples-to-apples. The localization gap is honest: the benchmark uses a
single-shot prefix-search term heuristic (no query planner yet); H5 and an
iterative agent loop are expected to close it. The token-efficiency and latency
results are the load-bearing finding — locating and reading the right code costs
~30× fewer tokens while staying far inside the interactive latency budget.

## Notes / limitations

- TypeScript only in this slice (the extractor is per-grammar; adding languages
  is a tree-sitter query + a few node-kind mappings).
- Reference resolution is name-based (no LSP): unique-name refs resolve cleanly;
  ambiguous names produce `resolved=0` edges that stay in the graph but can be
  discounted. This is the documented language-agnostic tradeoff (doc 05).
- Indexing is full-build; incremental `apply()` (doc 10) is the next perf
  milestone and is what the SO latency gate ultimately targets.
