# Cartograph

A language-agnostic tool that lets LLMs navigate large codebases with **maximum
accuracy and minimum token consumption**, via reactive *lazy context hydration*
— the agent pulls signatures first and full bodies only on demand, instead of
reading whole files.

This directory is the **working implementation** of the H1 + H2 vertical slice
specified in [`../docs/codebase-understanding/`](../docs/codebase-understanding/).
Rust workspace; SQLite index; tree-sitter extraction.

## Status

All five hypotheses implemented and tested (42 passing tests across the workspace):

| Capability | Spec | Status |
|---|---|---|
| Tiered repo map (file tree + purpose → signatures → bodies) | H1 | ✅ `outline`, `expand` |
| Symbol graph + PageRank centrality | H2 | ✅ `search`, `who_calls`, `neighborhood` |
| Impact radius (static graph + git co-change) | H3 | ✅ `impact` (`carto-git` mining) |
| Semantic diff hydration (working tree vs index) | H4 | ✅ `diff` |
| Adaptive context budgeter (task classifier + greedy budget fill) | H5 | ✅ `plan` |
| TypeScript extraction (symbols, signatures, docstrings, refs) | — | ✅ tree-sitter |
| SQLite index (WAL, recursive-CTE graph queries) | doc 02 | ✅ |
| CLI (`index`, `query`) | doc 01 | ✅ |
| Benchmark harness (full H1-H5, vs naive grep+full-file) | doc 11 | ✅ `carto-bench` |
| Incremental sync (file-watch, dirty reparse) | doc 10 | ⏳ full-build only in v1 |
| MCP stdio server (`serve`) | doc 03 | ⏳ CLI `query` exercises the same code path |

The CLI `query` subcommands return the exact JSON the MCP tools will serve, so
the daemon is a transport wrapper over an already-working tool surface. H4's
diff is working-tree-vs-index (the incremental `apply()` loop in doc 10 is the
remaining perf milestone); the symbol-level co-change and AST-rename refinements
noted in docs 06/08 are likewise deferred.

## Layout

```
cartograph/
├── crates/
│   ├── carto-model/   # shared type contract (mirrors docs/.../02-data-model.md)
│   ├── carto-store/   # SQLite schema + CRUD + edge resolution + neighborhood CTE + file_cochange
│   ├── carto-parse/   # tree-sitter TypeScript → symbols/refs/skeletons
│   ├── carto-git/     # git2 history mining → file co-change (H3)
│   ├── carto-core/    # index build + PageRank + IndexReader; impact/diff/plan modules (H3/H4/H5)
│   ├── carto-cli/     # `carto` binary: index / query
│   └── carto-bench/   # efficiency-frontier benchmark + report generator
├── bench/             # benchmark corpus (taskman API, 40 files) + tasks + report
└── docs/              # comparison vs prior art
```

## Build and run

```sh
cd cartograph
cargo build --release
cargo test                       # 42 tests

# index a repo (writes <root>/.carto/index.db)
cargo run -p carto-cli -- --root /path/to/repo index

# H1: tiered outline / full body
cargo run -p carto-cli -- --root /path/to/repo query outline "" --tier 1 --max-tokens 2000
cargo run -p carto-cli -- --root /path/to/repo query expand "src/util/paginate.ts#paginate:function"

# H2: who references a symbol / its neighborhood / fuzzy search
cargo run -p carto-cli -- --root /path/to/repo query who-calls "src/util/paginate.ts#paginate:function"
cargo run -p carto-cli -- --root /path/to/repo query neighborhood "src/http/router.ts#dispatch:function" --depth 2
cargo run -p carto-cli -- --root /path/to/repo query search paginate

# H3: blast radius (static neighbors + git co-change)
cargo run -p carto-cli -- --root /path/to/repo query impact "src/util/paginate.ts#paginate:function"

# H4: what changed since the last index (structural diff, working tree vs index)
cargo run -p carto-cli -- --root /path/to/repo query diff

# H5: budget-constrained retrieval plan for a task
cargo run -p carto-cli -- --root /path/to/repo query plan "where is pagination implemented" --budget 2000
```

## Benchmark results

Run on the bundled `bench/corpus` (40 TypeScript files, 2178 LOC, 211 symbols,
612 edges). The Cartograph path routes every task through the **H5 planner**
(which composes H1 outline/expand, H2 search/rank, and H3 impact under a 2000-
token budget) and measures whether the answer lands in the budgeted context.
Full report: [`bench/REPORT.md`](./bench/REPORT.md); raw data: `bench/results.json`.
Reproduce:

```sh
cargo run -p carto-bench -- bench/corpus bench/tasks.json bench
```

Headline (full H1-H5 vs naive grep + full-file baseline, 14 tasks):

| metric | result |
|---|---|
| **Overall token reduction** | **94.8%** (195,387 → 10,237 tokens) |
| Context precision | Cartograph reads **5.2%** of what naive reads |
| Query latency p50 / p95 | **1.8 ms / 3.5 ms** (interactive gate is 200 ms p95) |
| Full index build (2178 LOC) | ~250 ms |
| Symbol localization (answer in H5 context) | **12/14** |
| File localization | **14/14** (matches grep — but at ~19× fewer tokens) |

Both sides count tokens with the same `ceil(bytes/4)` estimator, so the ratios
are apples-to-apples. Routing through the H5 planner lifted symbol localization
from 8/14 (the earlier single-shot prefix search) to 12/14 and file localization
to 14/14, trading ~2 percentage points of token reduction (96.6% → 94.8%) for
reliably landing the answer in context. The two remaining symbol misses are
`bug-localize` tasks where the planner surfaces the right file but expands a
neighboring symbol — an iterative agent loop would resolve these.

See [`docs/comparison-vs-prior-art.md`](./docs/comparison-vs-prior-art.md) for
how this approach compares to cognee and the Codebase-Memory paper.

## Notes / limitations

- TypeScript only in this slice (the extractor is per-grammar; adding languages
  is a tree-sitter query + a few node-kind mappings).
- Reference resolution is name-based (no LSP): unique-name refs resolve cleanly;
  ambiguous names produce `resolved=0` edges that stay in the graph but can be
  discounted. This is the documented language-agnostic tradeoff (doc 05).
- Indexing is full-build; incremental `apply()` (doc 10) is the next perf
  milestone and is what the SO latency gate ultimately targets.
