# 12 — Roadmap and Build Order

The execution plan that turns the specs into a shippable tool. It sequences the
spine ([01](./01-architecture.md)–[03](./03-mcp-tool-surface.md)) and the
hypothesis specs ([04](./04-h1-recursive-repo-map.md)–09, plus
[10](./10-incremental-sync.md) and [11](./11-benchmark-harness.md)) into
milestones with measurable exit gates.

This doc owns *order and gates*; it does not re-specify mechanisms. Each milestone
links the spec it implements.

---

## 1. Guiding principles

**Vertical slice before breadth.** Ship H1 end-to-end on **one** language before
adding a second grammar or the graph. H1 is the lowest-IC `[ER]` primitive
([04](./04-h1-recursive-repo-map.md): IC 5) and exercises the full
`outline → expand` descent every other hypothesis composes against. One language,
all the way through a real MCP client, validates the lazy-hydration thesis before
we spend on topology, history, or multi-language. If H1 cuts tokens at the
projected ~10–20× ([04](./04-h1-recursive-repo-map.md) §1), the thesis holds and
breadth is justified; if it doesn't, we learn it in M1, not M5.

**Benchmark harness stood up early.** `carto-bench`
([01](./01-architecture.md) §2, [11](./11-benchmark-harness.md)) lands right after
the first slice (M2), not at the end. Every milestone from M2 on reports a
benchmark datapoint against the **grep baseline**. We measure TRA / TC / SO from
day one so the frontier is a tracked curve, never a retrofitted claim. A
hypothesis that doesn't move the frontier on the corpus does not merge.

**Foundation-first dependency discipline.** H1 + H2 are the foundation; everything
composes on them. The Parse stage feeds both H1 and H2
([01](./01-architecture.md) §3). H2's rank powers H1 truncation
([04](./04-h1-recursive-repo-map.md) §7). H3 needs H2's call graph
([README](./README.md)). H4 is the interactive-loop refresh layer. H5 plans
*across* H1–H4 and is therefore built **last**. We do not start a hypothesis
before its inputs exist and are benchmark-validated.

**One writer, WAL snapshots, incremental from the start.** The
single-sync-loop / WAL-snapshot model ([01](./01-architecture.md) §5) and
`stable_key` identity ([02](./02-data-model.md) §3) are established in M0/M1, not
bolted on. Building incremental-aware from M1 avoids a later rewrite when the
`<=200 ms` p95 single-edit gate ([README](./README.md) §"Hard operating
constraints") lands in M4.

---

## 2. Milestone plan

Each milestone states a **goal**, **key deliverables**, the **specs it
implements**, and a single measurable **exit criterion**. Later milestones must
not regress an earlier gate (the benchmark harness guards this from M2 on).

### Milestone table

| M | Name | Implements | Exit gate (measurable) |
|---|---|---|---|
| **M0** | Scaffold | [01](./01-architecture.md), [02](./02-data-model.md), [03](./03-mcp-tool-surface.md) | `carto index/serve/query` wire up; empty index builds; MCP `initialize` + `tools/list` round-trip in a real client; migrations apply clean. |
| **M1** | H1 vertical slice (1 lang) | [04](./04-h1-recursive-repo-map.md) | `outline → expand` works end-to-end on a TS corpus in an MCP client; descent answers a real task. |
| **M2** | Benchmark harness v0 | [11](./11-benchmark-harness.md) | `carto-bench` runs grep baseline + H1 on the corpus; first frontier datapoint (H1 TC reduction vs grep) recorded. |
| **M3** | H2 graph + rank | 05-h2 | `who_calls`/`neighborhood`/`get_symbol`/`search_symbols` answer; rank powers H1 truncation; H1+H2 beat grep on TRA. |
| **M4** | Incremental sync hardened | [10](./10-incremental-sync.md) | single-file edit **p95 ≤ 200 ms** SO gate met on the corpus; branch-switch bounded + reported. |
| **M5** | Multi-language | [04](./04-h1-recursive-repo-map.md), 05-h2 | 4–5 grammars; cross-language graph validated on a polyglot corpus; language-agnostic claims hold. |
| **M6** | H3 impact | 06-h3 | `impact` fuses static + co-change; impact beats grep+static-only on TRA for change-coupled tasks. |
| **M7** | H4 semantic diff | 08-h4 | `diff_context` returns structural deltas; loop refresh re-hydrates an edit in fewer tokens than re-`expand`. |
| **M8** | H5 budgeter | 09-h5 | `plan_retrieval` fills a budget greedily; full ablation shows H5 ≥ best single primitive at fixed budget. |
| **M9** | Post-v1 | [07](./07-behavioral-slice.md), 09-h5 (learned), opt backends | Behavioral Slice spec built; learned value model + optional LSP/embedding backends behind flags. |

---

### M0 — Scaffold

**Goal.** A skeleton that compiles, indexes nothing usefully yet, and speaks MCP.
Everything downstream slots into these seams.

**Key deliverables.**
- Rust workspace + crates exactly per [01](./01-architecture.md) §2:
  `carto-cli`, `carto-core`, `carto-store`, `carto-parse`, `carto-graph`,
  `carto-git`, `carto-diff`, `carto-mcp`, `carto-bench`; `grammars/` vendor dir.
- CLI subcommands `carto index` / `carto serve` / `carto query`
  ([01](./01-architecture.md) §1), all resolving `<repo>/.carto/index.db`.
- SQLite schema + migration runner in `carto-store` for the full DDL in
  [02](./02-data-model.md) §2 (WAL mode, all indexes, `meta` seed keys
  `schema_version`/`generation`/`head_rev`).
- MCP stdio JSON-RPC 2.0 skeleton in `carto-mcp` ([03](./03-mcp-tool-surface.md)
  §3): newline framing, `initialize`, `tools/list`, stderr-only logging, error
  envelope.
- The two core traits as real seams: `Producer { build, apply, stage }` and
  `IndexReader` ([01](./01-architecture.md) §3) — methods may be `todo!()`, the
  shapes are fixed so M1+ implement against stable signatures.
- `Stage` ordering enum (Parse/Rank/History/Diff) and the single-sync-loop +
  WAL-snapshot scaffolding ([01](./01-architecture.md) §5).

**Exit criterion.** `carto index` on an empty dir creates a valid WAL `index.db`
at the current `schema_version`; `carto serve` completes an MCP `initialize` +
`tools/list` handshake in a real client and returns the catalog skeleton; a
migration round-trips clean on a fresh and an existing DB.

---

### M1 — H1 vertical slice (one language: TypeScript)

**Goal.** Prove lazy hydration end-to-end on one language. This is the thesis
test.

**Key deliverables.**
- One tree-sitter grammar vendored (TypeScript) with a `tags.scm`
  ([04](./04-h1-recursive-repo-map.md) §4) mapping captures → `symbols.kind`.
- Parse-stage `Producer`: walk repo (respect `.gitignore`), detect lang, parse,
  extract `files` + `symbols` + `skeletons` (Tier-0 purpose heuristics §3,
  Tier-1 signature rendering §4), write content-hash for dirty detection
  ([02](./02-data-model.md) §4).
- `IndexReader::outline` (Tier-0 + Tier-1) and `expand` (Tier-2 live disk read).
- MCP `outline` + `expand` tools wired to those reads, with the shared
  token-budget protocol ([03](./03-mcp-tool-surface.md) §4,
  [04](./04-h1-recursive-repo-map.md) §7) — `max_tokens`, `truncated`, `dropped`,
  `token_est`, `generation` on every response.
- `stable_key` generation ([02](./02-data-model.md) §3) so `expand` handles
  survive a resync.
- `carto query outline/expand` for shell-driven debugging (same code path).

**Specs implemented.** [04](./04-h1-recursive-repo-map.md) (H1), backed by
[01](./01-architecture.md) §3 Parse stage and [02](./02-data-model.md)
`files`/`symbols`/`skeletons`.

**Exit criterion.** In a real MCP client against a mid-size TS repo, the worked
descent ([04](./04-h1-recursive-repo-map.md) §5) — root Tier-0 → one dir Tier-1 →
`expand` — returns a correct, source-grounded answer to "where is X validated?"
in **≤ 3k tokens**, with no full-file read. (Rank not required yet; Tier-1 may
sort lexically until M3.)

---

### M2 — Benchmark harness v0 (`carto-bench`)

**Goal.** Make the frontier measurable before we add primitives, so every later
milestone is judged, not asserted.

**Key deliverables.**
- Grep baseline runner: a scripted "naive agent" retrieval over the corpus
  (grep/ripgrep + full-file reads) producing a token cost per task
  ([11](./11-benchmark-harness.md)).
- Corpus selection: a frozen task set with ground-truth answer spans (start with
  the M1 TS repo; add tasks spanning lookup, who-calls, and change-coupling for
  later milestones).
- TRA / TC / SO instrumentation ([README](./README.md) §"Evaluation",
  [11](./11-benchmark-harness.md)): Task Retrieval Accuracy, Token Cost, and the
  Staleness/Sync window plumbing (SO measured for real once M4 lands).
- First frontier datapoint: **H1 vs grep** TC reduction on the corpus, checked
  into the harness output so regressions are visible.

**Specs implemented.** [11](./11-benchmark-harness.md).

**Exit criterion.** `carto-bench` runs both arms (grep baseline, H1) on the
corpus unattended and emits a TRA/TC table; H1 shows a measured TC reduction over
grep at equal-or-better TRA. The number is recorded as the v0 frontier point.

---

### M3 — H2 graph: edges, rank, topology tools

**Goal.** Add the topology layer. Rank becomes the truncation key for every
budgeted tool.

**Key deliverables.**
- Edge extraction in the Parse stage: `call`/`import`/`inherit`/`implement`/
  `typeref`/`read`/`write` into `edges` ([02](./02-data-model.md) §2).
- Reference resolution: bind call/typeref sites to def symbols within the TS
  corpus; mark unresolved edges `resolved=0` ([02](./02-data-model.md) §2) rather
  than dropping them.
- Rank stage (`carto-graph`): PageRank over `edges` → `symbol_rank` and
  denormalized `symbols.rank` ([01](./01-architecture.md) §3,
  [02](./02-data-model.md) §2; config `[graph]`).
- `IndexReader::get_symbol`/`neighbors`/`rank` and tools `get_symbol`,
  `who_calls`, `neighborhood`, `search_symbols` ([03](./03-mcp-tool-surface.md)),
  using the recursive-CTE traversal patterns ([02](./02-data-model.md) §5).
- **Rank now powers H1 truncation**: `outline` Tier-0/Tier-1 order by
  `symbols.rank DESC` ([04](./04-h1-recursive-repo-map.md) §7), closing the M1
  lexical-sort placeholder.

**Specs implemented.** 05-h2, plus the rank-truncation hooks in
[04](./04-h1-recursive-repo-map.md) §7 and the CTE queries in
[02](./02-data-model.md) §5.

**Exit criterion.** On the corpus, `who_calls`/`neighborhood` answer reference
tasks with TRA ≥ grep at lower TC; rank-ordered `outline` truncation keeps the
ground-truth symbol in-budget on tasks where flat ordering drops it. H1+H2
combined advance the recorded frontier over the M2 H1-only point.

---

### M4 — Incremental sync hardened

**Goal.** Make the index live at interactive speed — the hard constraint that
disqualifies any non-incremental mechanism ([README](./README.md) §"Hard
operating constraints").

**Key deliverables.**
- fs watcher + git-HEAD watcher in `carto serve`; debounce (~50 ms) coalescing
  events into one `Delta` ([01](./01-architecture.md) §4, config `[sync]`).
- `content_hash` dirty gate: no-op saves dropped before parse
  ([02](./02-data-model.md) §4, [04](./04-h1-recursive-repo-map.md) §6).
- Incremental reparse: tree-sitter reparse of dirty subtrees only; delete-and-
  reinsert `symbols`/`edges`/`skeletons` scoped by `file_id` in one write txn;
  `generation` bump ([10](./10-incremental-sync.md), [04](./04-h1-recursive-repo-map.md) §6).
- Localized re-rank: recompute only the affected rank region on edit rather than
  full PageRank ([10](./10-incremental-sync.md)).
- Branch-switch resync: walk the HEAD delta rather than a cold rebuild;
  bound-and-report the cost.
- SO instrumentation wired through `meta.generation` so the harness measures the
  real staleness window ([01](./01-architecture.md) §5,
  [11](./11-benchmark-harness.md)).

**Specs implemented.** [10](./10-incremental-sync.md); SO half of
[11](./11-benchmark-harness.md).

**Exit criterion.** Single-file edit **p95 ≤ 200 ms** SO gate met on the corpus
in `carto-bench`; branch switch completes within a reported bound; no torn reads
under concurrent tool calls (WAL snapshot invariant holds).

---

### M5 — Multi-language

**Goal.** Cash in the language-agnostic mandate ([README](./README.md)
§"Hard operating constraints" #2). Until now everything is TS-only.

**Key deliverables.**
- 3–4 more grammars vendored with `tags.scm` (e.g. Python, Go, Rust, Java) under
  `grammars/` ([01](./01-architecture.md) §2).
- Per-language signature-boundary handling and the override hook for exotic
  syntax flagged in [04](./04-h1-recursive-repo-map.md) §9.
- Cross-language graph: imports/typerefs that cross file-and-language boundaries
  resolve (or degrade to `resolved=0`) so `neighborhood` spans languages.
- Polyglot corpus added to `carto-bench` with cross-language tasks.

**Specs implemented.** [04](./04-h1-recursive-repo-map.md) §4 (per-language tags)
and 05-h2 (cross-language edges), validated on the polyglot corpus.

**Exit criterion.** All registered languages produce Tier-0/Tier-1 outlines and
edges on the polyglot corpus; cross-language `neighborhood` returns correct
spanning nodes; the H1+H2 frontier holds (no per-language regression) and the
language-agnostic claim is demonstrated, not asserted.

---

### M6 — H3 impact

**Goal.** Blast radius: static call graph fused with git co-change signal.

**Key deliverables.**
- History stage (`carto-git`): git-log walk → symbol-level `cochange`
  (support/confidence/lift) and `file_cochange` fallback
  ([02](./02-data-model.md) §2, config `[history]`); graceful degrade when git is
  absent/shallow ([01](./01-architecture.md) §7).
- `IndexReader::impact` + the `impact` tool fusing static neighbors (H2) with
  co-change, ranked, with `cochange: available|unavailable`
  ([03](./03-mcp-tool-surface.md), [02](./02-data-model.md) §5).
- Incremental History `apply`: append new commits only; branch switch walks the
  delta ([01](./01-architecture.md) §4).

**Specs implemented.** 06-h3.

**Exit criterion.** On change-coupling tasks in the corpus, `impact` (static +
co-change) achieves higher TRA at equal-or-lower TC than both grep and
static-only neighborhood; degrades cleanly to static-only on a git-less corpus.

---

### M7 — H4 semantic diff

**Goal.** Refresh the agent's context on edit/branch-switch with structural
deltas instead of resending text.

**Key deliverables.**
- Diff stage (`carto-diff`): `snapshots` + `ast_diffs` (added/removed/renamed/
  signature/body/moved) keyed by `stable_key` with cached `token_est`
  ([02](./02-data-model.md) §2).
- `IndexReader::diff` + `diff_context` tool over revisions / working tree
  ([03](./03-mcp-tool-surface.md)).
- Rename detection emitting the key-remap row so handles and co-change history
  follow renames ([02](./02-data-model.md) §3).
- Loop-refresh path: after an edit, re-hydrate via `diff_context` rather than a
  fresh `expand`.

**Specs implemented.** 08-h4.

**Exit criterion.** `diff_context` returns correct structural changes for a known
edit set; refreshing context after an edit costs materially fewer tokens than
re-`expand`-ing the changed symbols, measured in `carto-bench`; rename remap keeps
a pre-edit handle valid.

---

### M8 — H5 adaptive budgeter

**Goal.** The integration spine: plan retrieval across H1–H4 under a token budget.

**Key deliverables.**
- Heuristic task classifier (lookup / who-calls / impact / diff intent) selecting
  which primitives to invoke ([09-h5], config `[budget]`).
- Greedy budget fill across primitives using cached `token_est` and rank ordering
  ([02](./02-data-model.md) §6, [03](./03-mcp-tool-surface.md) §4).
- `plan_retrieval` tool returning both the auditable plan and the
  budget-truncated hydrated context, with `dry_run`
  ([03](./03-mcp-tool-surface.md)).
- Snapshot pinning so a multi-step plan reads one consistent generation
  ([01](./01-architecture.md) §5).
- Full ablation in `carto-bench`: H5 vs each primitive and vs grep at fixed
  budgets.

**Specs implemented.** 09-h5 (heuristic planner).

**Exit criterion.** At a fixed token budget, `plan_retrieval` matches or beats the
best single primitive on TRA across the task mix (the ablation table), and beats
the grep baseline by the stated v1 margin (§5). This is the v1 frontier point.

---

### M9 — Post-v1

**Goal.** Extend past the validated frontier.

**Key deliverables.**
- Behavioral Slice ([07](./07-behavioral-slice.md)): program slicing over H2's
  graph → minimal affecting statement set; `slice` tool
  ([03](./03-mcp-tool-surface.md)).
- Learned H5 value model replacing the heuristic classifier (trained on harness
  signals).
- Optional opt-in backends behind flags only ([README](./README.md) §"Rejected /
  deferred"): LSP-bridged resolution, embedding NL→code fallback.

**Specs implemented.** [07](./07-behavioral-slice.md); learned 09-h5; optional
backends.

**Exit criterion.** Slice ships behind the same budget protocol and is benchmarked
as an addition to the frontier; opt-in backends never sit on the core sync path
(constraint #2 preserved). No fixed date — sequenced after v1 ships.

---

## 3. Milestone dependency graph

```
                         ┌──────────────┐
                         │  M0 Scaffold │   crates · CLI · schema · MCP skeleton · traits
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M1 H1 slice  │   one language, outline→expand end-to-end
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M2 bench v0  │   grep baseline · TRA/TC · first frontier point
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M3 H2 graph  │   edges · PageRank · rank powers H1 truncation
                         └──┬────────┬──┘
                            │        │
                  ┌─────────▼──┐  ┌──▼───────────┐
                  │ M4 incr.   │  │ M5 multi-lang│   (M4 ∥ M5; both need H1+H2)
                  │ sync (SO)  │  │ + xlang graph│
                  └─────┬──────┘  └──────┬───────┘
                        └────────┬───────┘
                                 ▼
                         ┌──────────────┐
                         │ M6 H3 impact │   needs H2 call graph + git history
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M7 H4 diff   │   structural deltas · loop refresh
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M8 H5 plan   │   composes H1–H4 under budget  ← v1 line
                         └──────┬───────┘
                                ▼
                         ┌──────────────┐
                         │ M9 post-v1   │   slice · learned H5 · opt backends
                         └──────────────┘
```

Hard blocks: M0→M1→M2→M3 are strictly serial (each needs the prior's output). M4
and M5 both depend only on M3 (H1+H2) and may run in parallel. M6 needs H2's
resolved call graph (M3) and benefits from M4's incremental History `apply`. M8
needs all of H1–H4 to plan across them. M9 needs H2 (slice traverses the graph).

---

## 4. Risk register

| # | Risk | Surfaces at | Mitigation |
|---|---|---|---|
| R1 | **Reference resolution accuracy without LSP.** Binding call/typeref sites to defs from tree-sitter alone is heuristic; mis-resolution corrupts the graph and every tool downstream of it. | M3 (worsens M5 cross-language) | Mark unresolved edges `resolved=0` instead of dropping ([02](./02-data-model.md) §2) so PageRank degrades gracefully and the harness can quantify resolution error; keep LSP a strictly-optional opt-in backend (M9) for high-accuracy modes, never the core path. |
| R2 | **Re-rank latency.** Full PageRank on every edit blows the 200 ms p95 SO gate. | M4 | Localized re-rank over the affected region only ([10](./10-incremental-sync.md)); bound iterations (`[graph].pagerank_iterations`); measure SO continuously in `carto-bench` from M4 so a regression fails the gate, not review. |
| R3 | **Co-change attribution across renames.** A rename breaks `stable_key` ([02](./02-data-model.md) §3); if history doesn't follow it, co-change signal resets and H3 impact silently degrades. | M6 (depends on M7 rename detection) | H4's rename detection emits a key-remap row ([02](./02-data-model.md) §3) that History/Diff consume; sequence so M7's remap is available to M6's History `apply` (or fall back to `file_cochange` until then). |
| R4 | **Single-corpus benchmark generalization.** A frontier proven on one repo may not transfer; we could overfit primitives to the M1/M2 corpus. | M2 (compounds through M8) | Freeze ground-truth tasks early but grow the corpus at M5 (polyglot) and before M8's ablation; report per-corpus, never a single blended number; treat any primitive that only wins on one corpus as unproven. |
| R5 | **MCP SDK thinness in Rust.** We hand-roll JSON-RPC/stdio framing ([README](./README.md) locked decisions); subtle protocol bugs (framing, cancellation, notifications) can break real clients. | M0 (and every tool addition) | Build the framing once in `carto-mcp` ([03](./03-mcp-tool-surface.md) §3) and gate M0 on a real-client `initialize`+`tools/list` handshake; add a conformance test per tool; honor `$/cancelRequest` and `index_building` status from the start. |
| R6 | **Scope creep.** Six hypotheses plus optional backends invite building breadth before the slice is proven, or pulling M9 work forward. | continuous (esp. M1→M3) | The vertical-slice rule and the "no merge without a frontier datapoint" gate (§1); locked decisions ([README](./README.md)) are not re-opened; Slice, learned H5, LSP, embeddings are fenced to M9 by spec. |

---

## 5. What "v1 done" means

v1 ships at the **end of M8**. The concrete bar:

1. **All five hypotheses shipping.** H1 ([04](./04-h1-recursive-repo-map.md)), H2
   (05-h2), H3 (06-h3), H4 (08-h4), and H5 (09-h5) are implemented behind their
   canonical tools ([03](./03-mcp-tool-surface.md)): `outline`, `expand`,
   `get_symbol`, `who_calls`, `neighborhood`, `search_symbols`, `impact`,
   `diff_context`, `plan_retrieval`.
2. **Multi-language.** 4–5 grammars; the cross-language graph and the
   language-agnostic claim are validated on the polyglot corpus (M5), with no
   per-language core process ([README](./README.md) constraint #2).
3. **SO gate met.** Single-file edit **p95 ≤ 200 ms**; branch switch bounded and
   reported ([README](./README.md) §"Hard operating constraints",
   [10](./10-incremental-sync.md)), measured in `carto-bench`.
4. **Frontier beats grep by a stated margin.** On the benchmark task mix at a
   fixed token budget, `plan_retrieval` (H5 composing H1–H4) achieves
   **≥ 5× lower Token Cost at equal-or-higher TRA** than the grep+full-file
   baseline, reported per-corpus ([11](./11-benchmark-harness.md)). This is the
   single number that makes the `[ER]` thesis ([README](./README.md)
   §"Evaluation") true rather than asserted.
5. **No re-litigated rejects.** No LSP/embedding/compression-DSL/trace mechanism
   on the core path ([README](./README.md) §"Rejected / deferred"); any such
   backend is opt-in and deferred to M9.

A primitive that does not advance the recorded frontier does not count toward v1;
it is cut or deferred.

---

## 6. Immediate next actions (first PRs)

From M0, in order — each is a small, reviewable PR:

1. **`carto/` workspace + crate skeletons.** `Cargo.toml` workspace and the nine
   crates per [01](./01-architecture.md) §2 with the no-cycle dependency edges
   wired and empty `lib.rs`/`main.rs`. CI builds green.
2. **`carto-store` schema + migrations.** Full DDL from [02](./02-data-model.md)
   §2 (WAL, all indexes, `meta` seeds) behind a migration runner; round-trip test
   on fresh + existing DB.
3. **`carto-cli` subcommand wiring.** `index` / `serve` / `query`
   ([01](./01-architecture.md) §1) resolving `<repo>/.carto/index.db`; `index` on
   an empty dir produces a valid DB (the M0 exit gate, half).
4. **`carto-mcp` JSON-RPC stdio skeleton.** Newline framing, `initialize`,
   `tools/list` returning the catalog skeleton, stderr-only logging
   ([03](./03-mcp-tool-surface.md) §3); real-client handshake test (the M0 exit
   gate, other half).
5. **Core traits.** `Producer` + `IndexReader` + `Stage`
   ([01](./01-architecture.md) §3) with stable signatures and `todo!()` bodies,
   plus the single-sync-loop / WAL-snapshot scaffold — the seams M1 fills.

Then open **M1's first PR**: vendor the TypeScript grammar + `tags.scm` and stand
up the Parse-stage `Producer` writing `files`/`symbols`/`skeletons`
([04](./04-h1-recursive-repo-map.md) §3–4) — the start of the vertical slice.

---

See also: [README](./README.md) (locked decisions, hypothesis scores),
[01-architecture.md](./01-architecture.md) (crates, stages, traits),
[02-data-model.md](./02-data-model.md) (schema), [03-mcp-tool-surface.md](./03-mcp-tool-surface.md)
(tools), [04-h1-recursive-repo-map.md](./04-h1-recursive-repo-map.md) (H1),
[10-incremental-sync.md](./10-incremental-sync.md) (SO),
[11-benchmark-harness.md](./11-benchmark-harness.md) (TRA/TC/SO).
