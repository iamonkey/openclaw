# Cartograph — Phase 2 Specification

> **Working codename:** *Cartograph* (CLI binary: `carto`). The name is a
> placeholder; swap it project-wide before v1 if a better one lands.

A language-agnostic developer tool that lets LLMs navigate 100k+ LOC codebases
with **maximum accuracy and minimum token consumption**. It ships as a
high-performance **CLI** (indexing, CI/CD, background sync) that also exposes an
**MCP server over stdio (JSON-RPC)** for real-time IDE/chat-client use.

The core paradigm is **reactive, lazy context hydration**: instead of packing a
prompt with everything that might be relevant, the agent pulls only what it
needs, when it needs it, through dynamic tools backed by a fast-resyncing index.

---

## Locked decisions (Phase 2)

| Decision | Choice | Rationale |
|---|---|---|
| Implementation language | **Rust** | Single static binary, first-class tree-sitter bindings, best perf for a fast-mutating background daemon with incremental reparse. We accept the thinner MCP SDK and hand-roll the JSON-RPC/stdio layer. |
| Index storage | **SQLite** (recursive CTEs) | Single-file embedded, transactional, recursive CTEs cover graph traversal (`who_calls`, `neighborhood`); cheap append-only writes for H3 per-commit data and fast incremental writes for H4. Easiest path to a working vertical slice. |
| Hypothesis set | **H1–H5**, plus **Behavioral Slice** spec'd as a post-v1 extension | We spec both H3 (git co-change) and Behavioral Slice; v1 builds H3, Slice is sequenced after the frontier is validated. |

These are settled. Do not re-open them in Phase 2 without new evidence.

---

## The hypotheses

H1–H4 are retrieval **primitives** designed to compose. H5 is the **integration
spine** that plans across them under a token budget.

| # | Name | One-liner | Spec |
|---|---|---|---|
| **H1** | Recursive Repo Map | Tiered skeleton hydration: file tree + purpose → tree-sitter signatures → full bodies on demand. | [04-h1-recursive-repo-map.md](./04-h1-recursive-repo-map.md) |
| **H2** | Symbol-Rank Topology Graph | Cross-language def/ref/import graph + PageRank centrality. | [05-h2-symbol-rank-topology.md](./05-h2-symbol-rank-topology.md) |
| **H3** | Impact-Radius Mapping | Static call graph fused with git co-change signal → blast radius. | [06-h3-impact-radius.md](./06-h3-impact-radius.md) |
| **(R)** | Behavioral Slice Hydration | Program slicing over H2's graph → minimal affecting statement set. *(post-v1)* | [07-behavioral-slice.md](./07-behavioral-slice.md) |
| **H4** | Semantic Diff Hydration | AST-level structural diffs on edit / branch switch instead of resending text. | [08-h4-semantic-diff-hydration.md](./08-h4-semantic-diff-hydration.md) |
| **H5** | Adaptive Context Budgeter | Query planner compiling a token-budget-constrained retrieval plan across H1–H4. | [09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md) |

---

## Spec map

Read in this order. The **spine** (01–03) defines the canonical interfaces every
hypothesis spec builds on.

1. [01-architecture.md](./01-architecture.md) — process model, crate layout, indexer interface, data flow.
2. [02-data-model.md](./02-data-model.md) — canonical SQLite schema (symbols, edges, repo map, co-change, diffs).
3. [03-mcp-tool-surface.md](./03-mcp-tool-surface.md) — canonical tool contracts (`outline`, `expand`, `get_symbol`, `who_calls`, `neighborhood`, `impact`, …).
4. 04–09 — per-hypothesis specs (above).
5. [10-incremental-sync.md](./10-incremental-sync.md) — tree-sitter incremental reparse, branch-switch resync, staleness window.
6. [11-benchmark-harness.md](./11-benchmark-harness.md) — TRA / TC / SO methodology and the grep baseline.
7. [12-roadmap.md](./12-roadmap.md) — build order, vertical slice, milestones.

---

## Evaluation framework

Every concept is scored **1–10**. Higher is better **except IC** (cost).

| Metric | Meaning | Direction |
|---|---|---|
| **TRP** — Token Reduction Potential | How aggressively it cuts input noise | higher = better |
| **AF** — Architectural Fidelity | Preserves system-wide vs. local context | higher = better |
| **IC** — Implementation Complexity | Engineering lift | **higher = worse** |
| **II** — Index Integrity | Resilience to fast code mutation | higher = better |
| **US** — Update Speed | Incremental resync speed | higher = better |
| **Ceiling** | `[ER]` exponential return vs. `[LP]` linear plateau | — |

| # | Hypothesis | TRP | AF | IC | II | US | Ceiling |
|---|---|---|---|---|---|---|---|
| H1 | Recursive Repo Map | 9 | 7 | 5 | 8 | 9 | [ER] |
| H2 | Symbol-Rank Topology | 8 | 9 | 7 | 8 | 9 | [ER] |
| H3 | Impact-Radius Mapping | 7 | 9 | 6 | 8 | 8 | [ER] |
| (R) | Behavioral Slice | 7 | 8 | 7 | 7 | 7 | [ER] |
| H4 | Semantic Diff Hydration | 9 | 6 | 6 | 7 | 9 | [ER] |
| H5 | Adaptive Context Budgeter | 9 | 9 | 8 | 6 | 6 | [ER] |

---

## Hard operating constraints (shape every design)

1. **Interactive resync.** The index lives in a fast-mutating environment — it
   must resync as the developer types and switches git branches. Anything that
   can't re-compute incrementally at interactive speed is disqualified as a
   *core* mechanism. Target: **single-file edit p95 ≤ 200 ms**; branch switch
   bounded and reported (see [11-benchmark-harness.md](./11-benchmark-harness.md)).
2. **Language-agnostic.** No mechanism may require a per-language stateful server
   in the core path. Tree-sitter grammars are the common substrate. (LSP and
   embeddings are optional opt-in backends only — see *Rejected* below.)
3. **Lazy over eager.** Tools return the smallest useful unit and a handle to
   expand; never the whole file when a skeleton answers the question.
4. **Auditable fidelity.** Every token the agent sees is real source or a
   faithful structural derivation of it — no lossy invented shorthand.

---

## Rejected / deferred (do not re-litigate)

| Idea | Verdict | Why |
|---|---|---|
| LSP-bridged semantic index | Optional opt-in backend | One heavy stateful process per language breaks language-agnostic + lightweight-daemon mandate; brutal cold start, slow resync. Never the core index. |
| Embedding / vector-RAG chunks | Fuzzy NL→code fallback only | Retrieves what *looks* similar, not what's *connected*; no topology; re-embedding churns on every edit. |
| Custom compression DSL | Eliminated | Models read native code better than invented shorthand; in-context grammar burns the saved tokens; unauditable. |
| Execution-trace / hot-path index | Cut as core | Needs runnable code + workloads; dies on untested paths; detaches from source on first edit. |
| Coverage-weighted overlay | Cut | An enhancement, not an index; no standalone ceiling. |
