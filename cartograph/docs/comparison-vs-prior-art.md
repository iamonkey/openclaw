# Cartograph vs Prior Art: cognee and Codebase-Memory (arXiv:2603.27277)

A technical, evidence-based comparison of **Cartograph** against two external
codebase-understanding approaches:

- **cognee** — open-source AI memory platform; its "persistent codebase memory"
  guide ([cognee.ai](https://www.cognee.ai/blog/guides/ai-coding-agent-persistent-codebase-memory)).
- **Codebase-Memory** — Vogel et al., *"Codebase-Memory: Tree-Sitter-Based
  Knowledge Graphs for LLM Code Exploration via MCP"*, arXiv:2603.27277v1
  ([abs](https://arxiv.org/abs/2603.27277v1), [html](https://arxiv.org/html/2603.27277v1)).

> **Source-retrieval caveat (read first).** Both external pages return HTTP 403
> through this environment's policy egress proxy, and `WebFetch` failed on both
> after a retry. The hosts `arxiv.org` and `www.cognee.ai` are blocked by org
> egress policy, so the **full text of neither source could be retrieved
> directly.** Every external claim below is sourced from **web-search result
> summaries** of those two pages (search engine snippets/abstracts), not from the
> primary documents. Claims drawn this way are marked *[search-summary]*. Where a
> source does not state something, this report says so rather than guessing.
> Cartograph numbers, by contrast, are read directly from the in-repo
> `README.md`, `bench/REPORT.md`, and `docs/codebase-understanding/`.

---

## 1. TL;DR

- **Different paradigm.** Cartograph and Codebase-Memory both build a
  **structural graph directly from source via tree-sitter** and serve it over
  MCP; cognee is a **knowledge-graph + vector-embedding memory layer** that runs
  an LLM extraction pipeline and embeds chunks into a vector store
  *[search-summary]*. The core axis is *structural truth from the AST* (Cartograph,
  Codebase-Memory) vs *semantic similarity + persistent cross-session memory*
  (cognee).
- **Cartograph likely wins on tokens-per-task and latency under rapid edits.**
  Measured 96.6% token reduction vs naive grep+full-file and p95 query latency
  2.45 ms (`bench/REPORT.md`). Its hard design constraint is interactive resync
  (single-file edit p95 ≤ 200 ms), and it deliberately avoids re-embedding churn.
- **cognee likely wins on fuzzy NL→code and cross-session recall** ("what did we
  decide", architectural decisions) because it embeds content and persists memory
  across sessions *[search-summary]* — exactly the use case Cartograph **demoted
  embeddings to a fallback** for (`docs/codebase-understanding/README.md`,
  "Rejected/deferred").
- **Codebase-Memory is the closest cousin** and the most direct accuracy
  reference: it reports **83% answer quality vs 92% for a file-exploration agent,
  at ~10× fewer tokens and 2.1× fewer tool calls** across 31 repos
  *[search-summary]* — an independent data point consistent with Cartograph's
  "fewer tokens, slightly lower single-shot localization" tradeoff.
- **Freshness is the sharpest dividing line.** Cartograph treats incremental
  resync as a *disqualifying* core constraint (though v1 ships full-build only;
  `README.md`). Codebase-Memory reports a file watcher with XXH3 content-hash
  incremental re-indexing *[search-summary]*; cognee processes only new/updated
  files on re-runs but still runs an LLM-extract + embed pipeline per changed file
  *[search-summary]* — heavier per-edit cost.
- **Operational complexity diverges hard.** Cartograph = one Rust binary +
  embedded SQLite + tree-sitter, no model in the loop. cognee = LLM-in-the-loop
  extraction + embedding model + vector store + graph store *[search-summary]*.

---

## 2. Approach summaries

### Cartograph
A language-agnostic Rust CLI/MCP tool implementing **reactive lazy context
hydration**: the agent pulls signatures first and full bodies only on demand
(`README.md`). It builds a tiered repo map (H1), a tree-sitter symbol graph with
PageRank centrality (H2), git co-change impact (H3, next milestone), AST semantic
diffs (H4), and a token-budget planner (H5), all backed by an embedded **SQLite**
index with recursive-CTE graph traversal (`docs/codebase-understanding/01,02`).
No model or per-language server sits in the core path; tree-sitter grammars are
the common substrate, and "every token the agent sees is real source or a
faithful structural derivation of it" (auditable fidelity constraint). v1 is the
H1+H2 vertical slice with full-build indexing; incremental `apply()` is specified
but deferred.

### cognee
An open-source **AI memory platform** whose codebase guide builds a **code
knowledge graph plus vector embeddings**. Per search-summaries of the guide, its
six-stage "cognify" pipeline classifies documents, chunks them, uses an **LLM to
extract entities and relationships**, generates summaries, **embeds everything
into a vector store**, and commits edges to a graph; a dedicated "codegraph"
pipeline parses the AST to map functions/classes/calls/dependencies. It targets
**persistent, cross-session memory** (functions, classes, modules, dependencies,
*and architectural decisions*) queried over MCP *[search-summary]*. Freshness:
"only new or updated files are processed on re-runs," with content-hash skip of
unchanged files (`incremental_loading=True` by default) *[search-summary]*. The
guide's own metrics (token counts, latency, accuracy) were **not retrievable** and
are not stated in the available summaries.

### Codebase-Memory (arXiv:2603.27277v1)
Vogel et al. present an open-source system that constructs a **persistent,
tree-sitter-based knowledge graph served via MCP**, parsing **66 languages**
through a multi-phase pipeline with parallel worker pools, **call-graph
traversal, impact analysis, and community discovery** *[search-summary]*.
Freshness: a **background file watcher with adaptive polling**, where an **XXH3
content hash triggers incremental re-indexing** on modification *[search-summary]*.
It optimizes for token-efficient structural exploration vs file-reading/grep
agents. Reported evaluation (31 real-world repos): **83% answer quality vs 92%**
for a file-exploration agent, at **~10× fewer tokens and 2.1× fewer tool calls**;
on graph-native queries (hub detection, caller ranking) it **matches or exceeds**
the explorer on 19 of 31 *[search-summary]*. This is architecturally the nearest
neighbor to Cartograph (tree-sitter + graph + MCP + impact analysis), minus
Cartograph's git co-change fusion, PageRank-ranked lazy tiering, and explicit
token-budget planner.

---

## 3. Head-to-head comparison

| Dimension | Cartograph | cognee | Codebase-Memory (arXiv:2603.27277) |
|---|---|---|---|
| **Retrieval paradigm** | Structural graph from AST: tiered repo map (H1) + symbol/ref graph + PageRank (H2) + git co-change (H3) + AST diff (H4), planned under a token budget (H5). Lazy hydration. | Knowledge graph **+ vector embeddings**; LLM-extracted entities/relations + summaries embedded into a vector store; AST "codegraph" pipeline *[search-summary]*. | Tree-sitter knowledge graph; call-graph traversal, impact analysis, community discovery; served over MCP *[search-summary]*. |
| **Token efficiency at inference** | **Measured 96.6%** reduction vs naive grep+full-file (195,387→6,566 tok / 14 tasks); reads **3.4%** of baseline (`bench/REPORT.md`). | Not stated in available summaries. *(could not retrieve)* | **~10× fewer tokens** than a file-exploration agent *[search-summary]*. |
| **Freshness / incremental update on edit** | Core constraint: edit p95 ≤ 200 ms target; incremental `apply()` specified per producer. **v1 ships full-build only** (288 ms for 2178 LOC) (`README.md`). Avoids re-embedding by design. | Re-runs process **only new/updated files**; content-hash skip of unchanged *[search-summary]*. But each changed file still hits **LLM extraction + embedding** — heavier per-edit. | **File watcher + adaptive polling; XXH3 hash triggers incremental re-index** *[search-summary]*. Re-index cost per file not quantified in summaries. |
| **Latency** | **Measured p50 1.74 ms / p95 2.45 ms** query (gate 200 ms p95) (`bench/REPORT.md`). | Not stated in summaries. Embedding/vector-search + possible LLM steps generally add latency. *(could not retrieve)* | Not stated as a query-latency number in summaries; reports **2.1× fewer tool calls** *[search-summary]*. |
| **Accuracy / fidelity** | **Structural truth** — every token is real source or faithful structural derivation. Single-shot localization: symbol 8/14, file 12/14 vs grep 14/14, at ~30× fewer tokens (`bench/REPORT.md`). | Mix of structural (AST codegraph) and **semantic similarity** (embeddings) — retrieves what *looks* related. Accuracy not stated in summaries. | **83% answer quality vs 92%** for file-exploration agent; matches/exceeds on graph-native queries (19/31) *[search-summary]*. |
| **Language-agnosticism** | Tree-sitter grammars; **TS-only in this slice**, new langs = a tree-sitter query + node-kind mappings (`README.md`). No per-language server in core. | Language coverage not quantified in summaries; AST codegraph + general ingestion across "30+ sources" *[search-summary]*. | **66 languages** via tree-sitter *[search-summary]* — broadest stated coverage. |
| **Cold-start / indexing cost** | **288 ms full build** for 2178 LOC (`bench/REPORT.md`); pure parse + graph, no model calls. | Heavier: LLM entity/relation extraction + embedding of all chunks on first ingest *[search-summary]* (rate limits noted; uses fastembed for codegraph). | Multi-phase pipeline with parallel worker pools *[search-summary]*; cold-start cost not quantified in summaries. |
| **Infra & operational complexity** | **Low**: one Rust binary + embedded **SQLite (WAL)** + vendored tree-sitter. No vector DB, no embedding model, no LLM in core path. | **High**: LLM in the loop + embedding model + **vector store + graph store** + pipeline orchestration *[search-summary]*. | **Moderate**: tree-sitter + graph + MCP server + file watcher; graph/storage backend not detailed in summaries. |
| **Auditability / explainability** | **High** by design: results are real source spans or structural derivations; graph edges typed (call/import/inherit/typeref/...) with a `resolved` flag (`docs/.../02-data-model.md`). | Lower: vector similarity + LLM-extracted relations are probabilistic; retrieval rationale less directly traceable to source. *(not addressed in summaries)* | Graph edges are AST-derived (auditable); LLM not in the retrieval path per summaries, so retrieval is explainable. |

---

## 4. Where each wins

**cognee wins** when the query is **fuzzy natural-language → code** ("where do we
handle retries on flaky payments?") and when the value is **cross-session,
cross-artifact memory** — architectural decisions, prior conversations,
documentation, and 30+ ingestion sources unified in one graph
*[search-summary]*. Embeddings give it semantic recall over content that is *not*
structurally connected (comments, docs, naming-by-intent), which a pure topology
graph cannot surface. Its persistent-memory framing also targets "what did we
decide and why" recall across sessions — a use case Cartograph does not address
at all.

**Codebase-Memory wins** on **breadth (66 languages today)** and is a strong,
independent validation of the *graph-from-tree-sitter* thesis with published
accuracy numbers (83% vs 92% at ~10× fewer tokens) *[search-summary]*. It already
ships the file-watcher + content-hash incremental path that Cartograph has only
specified. For teams that want a polyglot structural graph *now*, it is the more
complete artifact.

**Cartograph wins** on:
- **Tokens-per-resolved-task and latency** — measured 96.6% reduction and 2.45 ms
  p95 (`bench/REPORT.md`), with a hard interactive-resync mandate.
- **Exact topology and blast radius** — PageRank-ranked symbol graph (H2) plus
  **git co-change-fused impact** (H3) is a capability neither external system
  describes; it answers "what breaks if I change this" with structural + historical
  signal rather than similarity.
- **Freshness under rapid edits** — the architecture *disqualifies* any mechanism
  that cannot recompute incrementally at interactive speed, and explicitly avoids
  the per-edit re-embedding churn that an embedding pipeline incurs.
- **Auditability** — no invented shorthand, no probabilistic retrieval in the core
  path; every token traces to source.

**The deliberate embedding demotion.** Cartograph's spec
(`docs/codebase-understanding/README.md`, "Rejected / deferred") records that
**embedding / vector-RAG chunks** were demoted to a *"Fuzzy NL→code fallback
only,"* with the stated reason: *"Retrieves what looks similar, not what's
connected; no topology; re-embedding churns on every edit."* This is the precise
boundary with cognee: cognee leans into the embedding strategy Cartograph
intentionally relegated. Both can be right — they optimize different objectives
(semantic recall + persistent memory vs structural truth + freshness + tokens).

---

## 5. Predicted performance (projection — NOT measured)

> The following is **reasoned projection**, not measurement. Cartograph numbers
> are measured on its single 2178-LOC TS corpus (`bench/REPORT.md`); the external
> systems were **not run** on that corpus. Treat as hypotheses.

**On `where-is` (locate the code that does X):**
- *Cartograph*: measured strong on tokens (97–98% reduction per task) but
  single-shot localization is honest-imperfect (file 12/14, symbol 8/14) because
  it uses a prefix-search heuristic with no query planner yet (`README.md`). H5 +
  an agent loop are expected to close the gap.
- *cognee*: **projected to win on NL-phrased where-is** (semantic match over
  comments/intent), but likely at higher tokens/latency and lower auditability.
- *Codebase-Memory*: projected comparable to Cartograph on structural where-is;
  its published 83% answer quality at ~10× fewer tokens *[search-summary]* is
  consistent with Cartograph's "fewer tokens, near-baseline localization" profile.

**On `what-breaks` (blast radius / impact):**
- *Cartograph*: **projected strongest** once H3 lands — git co-change *fused with*
  the static call graph is a signal the other two do not describe. Measured today:
  impact tasks already hit symbol+file (4/4) at 90–96.6% reduction (`bench/REPORT.md`).
- *Codebase-Memory*: competitive — it lists **impact analysis and caller ranking**
  as graph-native strengths (matches/exceeds explorer on 19/31) *[search-summary]*,
  but without the git co-change dimension.
- *cognee*: weakest projected here — similarity ≠ dependency; blast radius needs
  topology, which embeddings do not provide (Cartograph's stated rejection
  rationale).

**On `bug-localize`:**
- *Cartograph*: measured 98.5–98.6% reduction with file hits but no symbol hit on
  the two bug tasks (`bench/REPORT.md`) — finds the right file cheaply, exact
  symbol pinpointing is the gap a planner/agent loop should close.
- *cognee*: projected mixed — good if the bug is describable in NL or tied to a
  remembered decision; weak for purely structural off-by-one localization.
- *Codebase-Memory*: projected similar to Cartograph (structural), without the
  co-change/history hint.

**On the Sync-Overhead / freshness gate (edit p95 ≤ 200 ms):**
- *Cartograph*: query latency is far inside budget (2.45 ms p95, measured), but
  the **incremental-edit path is unmeasured in v1** (full build = 288 ms for 2178
  LOC). This is the explicitly-acknowledged open milestone.
- *Codebase-Memory*: **projected to pass** — it already does content-hash
  incremental re-indexing on a watcher *[search-summary]*; per-edit cost not
  published.
- *cognee*: **projected to struggle against a 200 ms interactive gate** — even
  with "only changed files" re-ingestion, each changed file traverses an
  LLM-extract + embed pipeline *[search-summary]*, which is orders of magnitude
  above a tree-sitter incremental reparse. cognee is architected for *persistent
  memory*, not *type-speed resync* — likely a goals mismatch, not a defect.

---

## 6. Caveats & threats to validity

- **Could not retrieve primary sources.** Both `arxiv.org` and `www.cognee.ai`
  are blocked by org egress policy (HTTP 403 via the policy proxy; `WebFetch`
  retried and failed both). All external claims are from **web-search result
  summaries**, marked *[search-summary]*, and may omit or simplify the primary
  text. In particular, **cognee's own token/latency/accuracy metrics were not in
  any retrieved summary** and are reported as unstated, not zero.
- **Single small corpus.** Cartograph's measurements are on one **2178-LOC
  TypeScript** corpus, 14 tasks (`bench/REPORT.md`). Token-reduction and latency
  ratios may not hold at 100k+ LOC or on other languages/grammars.
- **TS-only slice.** Cartograph v1 indexes TypeScript only; its
  language-agnostic claim is architectural, not yet broadly demonstrated, whereas
  Codebase-Memory reports 66 languages *[search-summary]*.
- **Projections are not measurements.** Section 5 reasons about systems that were
  **not run** on Cartograph's corpus; treat as hypotheses, not benchmarks. The
  external numbers come from each system's *own* (differently-defined)
  evaluations — e.g. cognee/Codebase-Memory "answer quality" and "tokens" are not
  the same metric as Cartograph's `ceil(bytes/4)` token-reduction ratio, so
  cross-system numeric comparisons are indicative only.
- **Possibly different goals.** cognee targets **persistent cross-session IDE
  memory** (incl. architectural decisions); Cartograph targets a **token-efficient
  retrieval primitive** with hard freshness constraints. Some "wins/losses" above
  reflect divergent objectives, not strict superiority.
- **Maturity asymmetry.** Cartograph's H3/H4/H5 and incremental `apply()` are
  specified but partly unbuilt in v1 (`README.md` status table); some "projected
  wins" depend on milestones not yet measured.

---

## Sources

- Cartograph (in-repo, read directly): `cartograph/README.md`,
  `cartograph/bench/REPORT.md`, `docs/codebase-understanding/README.md`,
  `docs/codebase-understanding/01-architecture.md`,
  `docs/codebase-understanding/02-data-model.md`.
- cognee — *Persistent Codebase Memory for Coding Agents 2026*
  ([cognee.ai](https://www.cognee.ai/blog/guides/ai-coding-agent-persistent-codebase-memory))
  *(full text not retrievable; cited via search summaries)*; supporting:
  [topoteretes/cognee](https://github.com/topoteretes/cognee),
  [memgraph.com: From RAG to Graphs](https://memgraph.com/blog/from-rag-to-graphs-cognee-ai-memory).
- Vogel et al., *Codebase-Memory: Tree-Sitter-Based Knowledge Graphs for LLM Code
  Exploration via MCP*, arXiv:2603.27277v1
  ([abs](https://arxiv.org/abs/2603.27277v1), [html](https://arxiv.org/html/2603.27277v1))
  *(full text not retrievable; cited via search summaries)*.
