# 01 — Architecture

Canonical process model, crate layout, and data flow for Cartograph. Every
hypothesis spec assumes the interfaces defined here.

---

## 1. Process model

Cartograph is one binary (`carto`) with three roles, all sharing the same core
library and the same on-disk index:

```
              ┌─────────────────────────────────────────────┐
              │                  carto (one binary)          │
              ├───────────────┬───────────────┬──────────────┤
              │  carto index  │  carto serve  │  carto query │
              │  (one-shot    │  (MCP daemon, │  (one-shot   │
              │   build/CI)   │   stdio JSON- │   debug/CLI) │
              │               │   RPC + watch)│              │
              └───────┬───────┴───────┬───────┴──────┬───────┘
                      │               │              │
                      └───────────────┼──────────────┘
                                      ▼
                         ┌─────────────────────────┐
                         │     carto-core (lib)     │
                         │  indexer · graph · sync  │
                         │  planner · store access  │
                         └────────────┬────────────┘
                                      ▼
                         ┌─────────────────────────┐
                         │   SQLite index file      │
                         │   .carto/index.db        │
                         └─────────────────────────┘
```

- **`carto index`** — full or incremental build of the index for a repo. Used in
  CI/CD and for cold start. Exits when done.
- **`carto serve`** — long-lived **MCP server over stdio** (newline-delimited
  JSON-RPC 2.0). Owns the file watcher and git-HEAD watcher; keeps the index
  resynced and answers tool calls (see [03-mcp-tool-surface.md](./03-mcp-tool-surface.md)).
- **`carto query <tool> [args]`** — invoke any single tool from the shell for
  debugging, scripting, and the benchmark harness. Same code path as `serve`,
  no daemon.

All three resolve the index at `<repo>/.carto/index.db` (overridable). The index
is a build artifact: `.carto/` is `.gitignore`-d.

---

## 2. Crate layout (Rust workspace)

```
carto/
├── Cargo.toml                  # workspace
├── crates/
│   ├── carto-cli/              # bin: arg parsing, subcommands, wiring
│   ├── carto-core/             # lib: orchestration, planner (H5), public API
│   ├── carto-store/            # SQLite access, schema, migrations, query builders
│   ├── carto-parse/            # tree-sitter: grammar registry, incremental reparse,
│   │                           #   symbol/edge extraction via per-language queries
│   ├── carto-graph/            # symbol graph, PageRank (H2), traversal helpers
│   ├── carto-git/              # libgit2 wrapper: log walking, co-change (H3), HEAD watch
│   ├── carto-diff/             # AST structural diff (H4)
│   ├── carto-mcp/              # JSON-RPC 2.0 stdio framing, tool dispatch, schemas
│   └── carto-bench/            # benchmark harness (TRA/TC/SO), grep baseline
└── grammars/                   # vendored tree-sitter grammars + tags queries
```

Dependency direction (no cycles):

```
carto-cli ─▶ carto-core ─▶ { carto-store, carto-parse, carto-graph, carto-git, carto-diff }
carto-mcp ─▶ carto-core
carto-bench ─▶ carto-core
carto-parse ─▶ carto-store   (writes extracted rows)
carto-graph ─▶ carto-store   (reads edges, writes ranks)
carto-git  ─▶ carto-store   (writes co-change rows)
```

`carto-core` is the only crate the binary talks to. Hypotheses are modules/traits
inside core that read and write through `carto-store`.

---

## 3. The indexer interface (how H1–H4 compose)

Indexing is a pipeline of **producers** that each write into the shared store.
A producer is a trait; H1–H4 are implementations. H5 (the planner) is a
**consumer** that only reads.

```rust
/// A unit of indexing work. Producers are ordered by `stage()`.
pub trait Producer {
    fn name(&self) -> &'static str;
    fn stage(&self) -> Stage;              // see ordering below

    /// Full build over the whole repo snapshot.
    fn build(&self, ctx: &BuildCtx, store: &mut StoreTxn) -> Result<()>;

    /// Incremental update for a changeset (edited/added/removed files,
    /// or a HEAD move). Must be cheap; see 10-incremental-sync.md.
    fn apply(&self, delta: &Delta, store: &mut StoreTxn) -> Result<()>;
}

pub enum Stage {
    Parse   = 0,   // carto-parse: files → symbols, edges, skeletons   (feeds H1 + H2)
    Rank    = 1,   // carto-graph: edges → centrality                  (H2)
    History = 2,   // carto-git:  git log → co-change pairs            (H3)
    Diff    = 3,   // carto-diff: snapshot deltas → structural diffs   (H4)
}
```

**Ordering rationale:** Parse must precede Rank (ranks read edges) and History
(co-change keys to symbols, not just files). Diff runs last because it compares
the new parse against the prior snapshot. The planner (H5) never registers as a
producer — it composes the *outputs* at query time.

```rust
/// Read-only facade the planner and tools use. Backed by carto-store.
pub trait IndexReader {
    fn outline(&self, path: &Path, tier: Tier) -> Result<Outline>;          // H1
    fn expand(&self, symbol: SymbolId) -> Result<SourceSpan>;               // H1
    fn get_symbol(&self, q: &SymbolQuery) -> Result<Vec<Symbol>>;           // H2
    fn neighbors(&self, s: SymbolId, dir: Dir, depth: u8) -> Result<SubGraph>; // H2
    fn impact(&self, s: SymbolId, opts: ImpactOpts) -> Result<ImpactSet>;   // H3
    fn diff(&self, from: &Rev, to: &Rev, scope: Scope) -> Result<AstDiff>;  // H4
    fn rank(&self, s: SymbolId) -> Result<f32>;                             // H2
}
```

Every MCP tool ([03](./03-mcp-tool-surface.md)) is a thin adapter over one
`IndexReader` method plus budget accounting. The planner (H5) is the only
component that calls multiple methods to satisfy one agent request.

---

## 4. Data flow

### Cold start (`carto index`, or first `serve`)

```
walk repo (respect .gitignore)
  └─▶ for each file: detect lang → tree-sitter parse → extract symbols + edges + skeleton
        └─▶ Parse stage writes: files, symbols, edges, skeletons
  └─▶ Rank stage: load edge table → PageRank → write symbol.rank
  └─▶ History stage: git log walk → co-change pairs → write cochange
  (Diff stage no-op on cold start; baseline snapshot recorded)
```

### Steady state (`carto serve`)

```
fs watcher ──┐
             ├─▶ debounce (≈50ms) ─▶ Delta{changed_files} ─▶ pipeline.apply(delta)
git HEAD ────┘                                                  │
watcher                                                         ├─ Parse.apply:  reparse dirty subtrees only
                                                               ├─ Rank.apply:   recompute affected rank region (10-)
                                                               ├─ History.apply: append new commits only (branch switch = walk delta)
                                                               └─ Diff.apply:    structural diff old↔new for H4
```

Tool calls read a **consistent snapshot** of the store. Writes happen in a single
SQLite write transaction per delta; readers use WAL-mode snapshots so a resync in
flight never returns a torn view. The **staleness window** is the gap between a
file changing on disk and its delta committing — measured and gated in
[11-benchmark-harness.md](./11-benchmark-harness.md).

---

## 5. Concurrency & consistency

- **SQLite in WAL mode.** One writer (the sync loop), many readers (tool calls).
  Readers never block the writer and vice versa.
- **Single sync loop.** All `apply()` calls are serialized through one task so
  producer ordering holds and the write transaction is coherent. fs events
  coalesce into one `Delta` per debounce tick.
- **Snapshot reads.** Each tool call opens a read transaction at the latest
  committed point. Long planner calls (H5) pin one snapshot for the whole plan so
  multi-step retrieval is internally consistent.
- **Generation counter.** Every committed delta bumps a monotonic `generation`.
  Tool responses include the generation they were computed against so a client
  can detect staleness and the benchmark can measure the staleness window.

---

## 6. Configuration

`.carto/config.toml` at repo root (all keys optional; sane defaults):

```toml
[index]
languages = ["auto"]          # or explicit list; "auto" = detect by extension
exclude   = ["dist", "vendor", "*.min.js"]   # extends .gitignore
max_file_bytes = 1_500_000    # skip larger files (generated/minified)

[sync]
debounce_ms = 50
watch_git_head = true

[graph]
pagerank_iterations = 30
pagerank_damping = 0.85

[history]
cochange_window_commits = 5000   # how far back to mine git log
cochange_min_support = 3         # prune rare pairs

[budget]
default_token_budget = 6000      # H5 default plan ceiling
```

See each hypothesis spec for the keys it consumes.

---

## 7. Error & degradation policy

- **Unknown language / parse failure** → file is recorded with a Tier-0 entry
  (path + size + best-effort one-line purpose from comments/README heuristics) and
  skipped for symbol extraction. The repo map ([H1](./04-h1-recursive-repo-map.md))
  still lists it; the graph just has no symbols for it.
- **Git absent / shallow clone** → History stage degrades gracefully: H3 falls
  back to pure static call-graph impact with a `cochange: unavailable` flag.
- **Index missing/corrupt on `serve`** → auto-trigger a cold build, surfacing
  progress over MCP logging notifications; tools return `index_building` until
  ready rather than erroring.
