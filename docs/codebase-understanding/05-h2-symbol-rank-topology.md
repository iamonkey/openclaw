# 05 — H2: Symbol-Rank Topology Graph

Cross-language def/ref/import graph over the whole repo, plus a PageRank-style
centrality score that ranks every symbol by *architectural gravity*. This is the
backbone the rest of the system reads from: H3 ([06-h3-impact-radius.md](./06-h3-impact-radius.md))
fuses its static neighbors with co-change, the Behavioral Slice
([07-behavioral-slice.md](./07-behavioral-slice.md)) walks its edges, and the
`rank` it writes is the truncation key that every budgeted tool uses to decide
what to keep ([09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md)).

| TRP | AF | IC | II | US | Ceiling |
|---|---|---|---|---|---|
| 8 | 9 | 7 | 8 | 9 | [ER] |

Backs the `get_symbol`, `who_calls`, `neighborhood`, and `search_symbols` tools
([03-mcp-tool-surface.md](./03-mcp-tool-surface.md)). Implemented in `carto-graph`
(rank, traversal) and `carto-parse` (extraction); both write through `carto-store`
([01-architecture.md](./01-architecture.md) §2).

---

## 1. Goal — why topology beats similarity (AF 9)

The agent's hardest retrieval question is not "what code *looks* like this query"
but "what code is *wired to* the thing I'm touching." Those are different graphs.
Embedding/vector-RAG retrieves the former — lexically or semantically adjacent
chunks — and was rejected as a core index precisely because it has **no topology**
([README.md](./README.md), *Rejected*). A validator and a `validate()` call site
can be lexically distant and embed nowhere near each other, yet be one `call` edge
apart; two functions named `parse` in different subsystems embed close yet share no
edge at all.

H2 indexes the **actual wiring**: which symbol defines, calls, imports, inherits,
implements, or type-references which other symbol. On top of that wiring it runs
PageRank, so the score answers a structural question — *how much of the system
flows through this node?* — not a textual one. That is why H2 carries the highest
**AF** in the set: it is the only mechanism whose ranking is a property of the
architecture rather than of the prose. A config loader that 200 sites import scores
high because it *is* central; a one-off helper scores low because it *is*
peripheral. The agent gets the load-bearing walls first and the trim last, which
is exactly the order a budget should spend tokens in.

This graph is also the substrate the other hypotheses stand on, so its fidelity
compounds: a missed edge here is a missed impact node in H3 and a missing
statement in the Slice.

---

## 2. Symbol & edge extraction (tree-sitter)

### 2.1 Definitions → `symbols`

Each grammar in `grammars/` ships a tags-style query (`<lang>/symbols.scm`) that
captures definition nodes and their parts. The Parse stage
([01-architecture.md](./01-architecture.md) §3, `Stage::Parse`) runs the query over
the parse tree and writes one `symbols` row per `@definition` capture:

```scheme
; grammars/typescript/symbols.scm  (excerpt)
(class_declaration
  name: (type_identifier) @name) @definition.class

(method_definition
  name: (property_identifier) @name) @definition.method

(function_declaration
  name: (identifier) @name) @definition.function

(public_field_definition
  name: (property_identifier) @name) @definition.field
```

The capture suffix (`class`, `method`, …) maps to `symbols.kind`
(`function|method|class|type|interface|const|module|field`,
[02-data-model.md](./02-data-model.md) §2). `parent_id` is the nearest enclosing
`@definition` (method → class); `signature` and `doc` are rendered from the node's
header and leading comment for H1's Tier-1 skeleton; `start/end_byte/row` are the
node span. `stable_key` is assembled as
`"<path>#<container-path>/<name>:<kind>"` ([02](./02-data-model.md) §3).

### 2.2 References, imports, inheritance, type-refs → `edges`

A companion query (`<lang>/refs.scm`) captures *use* sites. Each capture becomes a
candidate edge whose `src_id` is the enclosing symbol (the def the use site sits
inside) and whose `kind` is determined by the capture:

| Capture | `edges.kind` | Source construct |
|---|---|---|
| `@reference.call` | `call` | call expression `foo(...)`, method call `x.foo()` |
| `@reference.import` | `import` | `import`, `use`, `require`, `from … import` |
| `@reference.inherit` | `inherit` | `extends Base`, `class C(Base)` |
| `@reference.implement` | `implement` | `implements I`, Rust `impl Trait for T` |
| `@reference.type` | `typeref` | type annotation, generic arg, return type |
| `@reference.read` | `read` | identifier read of a const/field/module global |
| `@reference.write` | `write` | assignment to a const/field/module global |

```scheme
; grammars/typescript/refs.scm  (excerpt)
(call_expression
  function: [(identifier) @reference.call
             (member_expression property: (property_identifier) @reference.call)])

(class_heritage (extends_clause value: (_) @reference.inherit))
(import_statement (import_clause (named_imports (import_specifier
  name: (identifier) @reference.import))))
(type_annotation (type_identifier) @reference.type)
```

`read`/`write` are captured but **off by default** in the rank graph (config
`[graph].edge_kinds`); they explode in count and add little centrality signal.
They are still written so `neighborhood` can request them explicitly.

### 2.3 The hard part — resolution without an LSP

The query gives us a **use site with a name and a syntactic scope**, not a target
def. Turning `@reference.call name="parse"` into `dst_id = <the parse method>` is
name resolution, and we deliberately do it with heuristics rather than a
per-language type-checker — an LSP per language is rejected as a core mechanism
([README.md](./README.md), *Rejected*; [01](./01-architecture.md) §constraint 2).
The resolver runs **inside one Parse transaction after all defs are written**, so
the symbol table for the whole delta is available:

1. **Import-aware scope.** Build a per-file import map from the `import` captures:
   local name → (module path, original name). A bare `parse` resolves first against
   in-file defs, then against imported names, then against module-level/global
   defs. This is what makes cross-file `call`/`typeref` edges land on the right def
   instead of any same-named symbol.
2. **Scope chain.** Walk enclosing scopes outward (block → function → class →
   module). Nearer binding wins; a parameter or local shadows a global of the same
   name and we emit **no edge** (locals aren't symbols).
3. **Receiver typing (best-effort).** For `x.foo()` we attempt to type `x` from its
   declaration (`const x: ConfigLoader = …`, `x = new ConfigLoader()`) using the
   `typeref` we already captured, then resolve `foo` as a method of that class. When
   `x`'s type is unknown, we fall back to *name-only* resolution across all methods
   named `foo`.
4. **Candidate scoring.** If a name resolves to exactly one def → resolved edge to
   it. If it resolves to several (overloads, same name in N classes) → edge to the
   **single best candidate by (import-visibility, then rank-at-write, then source
   proximity)**, and we may additionally fan out low-weight unresolved edges to the
   alternates so `who_calls` doesn't miss a real caller.

Every edge carries `resolved` ([02](./02-data-model.md) §2):

- `resolved = 1` — bound to a concrete def with confidence (rules 1–3, single
  candidate). Counts at full weight in PageRank.
- `resolved = 0` — name matched but binding ambiguous, *or* the name has no def in
  the repo (external/stdlib symbol; we still record the edge to a synthetic
  external symbol or drop it per config). Counts at reduced weight, and tools can
  filter it (`who_calls` exposes resolved-only by default).

**Accepted tradeoff.** This is high-recall, medium-precision. We accept:

- *False edges* from name collisions (two `parse`s, edge to the wrong one) — bounded
  by import-scope + receiver typing, and flagged `resolved=0` when ambiguous.
- *Missed edges* from dynamic dispatch, reflection, string-built calls, and macro
  expansion (see §8) — these are simply absent.

The bet is that for *navigation and ranking*, approximately-correct topology over
100% of the repo beats exact topology over the fraction an LSP can cheaply cover.
PageRank is robust to a few percent edge noise; a missing whole-language is not. We
measure resolution precision/recall against an LSP oracle on fixture repos in
[11-benchmark-harness.md](./11-benchmark-harness.md).

---

## 3. PageRank — computing architectural gravity

The Rank stage (`carto-graph`, `Stage::Rank`) reads the edge table, runs PageRank,
and writes the score back. *Architectural gravity* = the stationary distribution of
a random walk over reference edges: a symbol is heavy when many heavy symbols point
at it. Centrality flows **in the dependency direction** — `src` depends on `dst`, so
we rank on the **reverse** graph (mass accumulates on the things everyone depends
on: core types, base classes, the config loader), which is the "most depended-upon"
notion the agent wants first.

```rust
// crates/carto-graph/src/rank.rs  (sketch)
pub struct RankCfg { pub iterations: u32, pub damping: f32 } // from [graph] config

pub fn pagerank(g: &Graph, cfg: &RankCfg) -> Vec<f32> {
    let n = g.nodes.len();
    let mut r = vec![1.0 / n as f32; n];
    let teleport = (1.0 - cfg.damping) / n as f32;
    for _ in 0..cfg.iterations {
        let mut next = vec![teleport; n];
        let mut dangling = 0.0;
        for v in 0..n {
            let out = g.weighted_out_degree(v);        // reverse graph; see note
            if out == 0.0 { dangling += r[v]; continue; }
            for e in g.out_edges(v) {                  // e.dst, e.weight
                next[e.dst] += cfg.damping * r[v] * (e.weight / out);
            }
        }
        let spread = cfg.damping * dangling / n as f32; // redistribute sinks
        for x in next.iter_mut() { *x += spread; }
        r = next;
    }
    normalize_0_1(r)   // scale to [0,1] for stable cross-repo comparison + display
}
```

- **`iterations` / `damping`** come from `[graph].pagerank_iterations` (default 30)
  and `[graph].pagerank_damping` (default 0.85) ([01](./01-architecture.md) §6). 30
  iterations is ample convergence at this graph size.
- **Edge weighting by kind** (optional, `[graph].edge_weights`): `inherit`/
  `implement` and `typeref` weigh more than a single `call` because a base class or
  a shared type is structurally heavier than one call site; `import` is light (it's
  noisy and often re-exported); `resolved=0` edges are down-weighted (e.g. ×0.3) so
  guessed wiring can't manufacture rank. Defaults:
  `inherit=2.0, implement=2.0, typeref=1.5, call=1.0, import=0.5, read/write=0.25`.
- **Write-back.** Rank is **denormalized onto `symbols.rank`** in the same
  transaction so every tool sorts by an indexed column
  (`idx_symbols_rank`, [02](./02-data-model.md) §2) with zero join. The optional
  `symbol_rank` side table can retain the raw (pre-normalization) score plus a tiny
  *reasoning* blob — top in-edges and degree — surfaced as the `reason` an `outline`
  or `neighborhood` entry shows for why a node ranks where it does.

At ~100k LOC (10k–40k symbols, 50k–200k edges, [02](./02-data-model.md) §6) a full
PageRank is tens of milliseconds — fine for cold start, too slow to run on *every*
keystroke, which §5 addresses.

---

## 4. The four tools

All four are thin adapters over `IndexReader` ([01](./01-architecture.md) §3) plus
the shared budget protocol ([03](./03-mcp-tool-surface.md) §4): order by rank, fill
to `max_tokens`, set `truncated` + `dropped`.

### 4.1 `search_symbols` — entry point

Substring/fuzzy over `name` + `fqn`, the agent's way in when it has only a fragment.
Ranked by rank first so the central `parse` beats an obscure one.

```sql
SELECT s.* FROM symbols s
WHERE s.name LIKE '%' || :q || '%' OR s.fqn LIKE '%' || :q || '%'
ORDER BY s.rank DESC, length(s.name) ASC   -- centrality, then tighter match
LIMIT :limit;
```

### 4.2 `get_symbol` — exact lookup

Resolve a name/fqn/kind (any subset) to its def(s); returns signature, location,
rank. Uses `idx_symbols_name` / `idx_symbols_fqn`.

```sql
SELECT s.* FROM symbols s
WHERE (:name IS NULL OR s.name = :name)
  AND (:kind IS NULL OR s.kind = :kind)
  AND (:fqn  IS NULL OR s.fqn  = :fqn)
ORDER BY s.rank DESC;
```

### 4.3 `who_calls` — reverse edges, one hop

Extends the canonical reverse-edge pattern ([02](./02-data-model.md) §5) with
kind filtering and resolved-only default. `idx_edges_dst` powers it.

```sql
SELECT s.*, e.kind AS edge, e.resolved
FROM edges e JOIN symbols s ON s.id = e.src_id
WHERE e.dst_id = :x
  AND e.kind IN (/* :kinds, default ('call','typeref') */)
  AND (:include_unresolved OR e.resolved = 1)
ORDER BY s.rank DESC          -- heaviest callers first; truncate the long tail
LIMIT :limit;
```

The `site` (path,row) in the response comes from the caller symbol's span; precise
per-call-site rows are a v2 nicety (we'd store byte offsets on `edges`).

### 4.4 `neighborhood` — bounded, rank-ordered subgraph

Extends the recursive-CTE BFS from [02](./02-data-model.md) §5 with direction and
kind filters. `dir` ∈ `in|out|both`; the two `UNION` arms are gated on it.

```sql
WITH RECURSIVE nb(id, depth, dir) AS (
  SELECT :x, 0, 'self'
  UNION
  SELECT e.dst_id, nb.depth+1, 'out'            -- emit only when dir in (out,both)
    FROM edges e JOIN nb ON e.src_id = nb.id
    WHERE nb.depth < :n AND e.kind IN (:kinds) AND :want_out
  UNION
  SELECT e.src_id, nb.depth+1, 'in'             -- emit only when dir in (in,both)
    FROM edges e JOIN nb ON e.dst_id = nb.id
    WHERE nb.depth < :n AND e.kind IN (:kinds) AND :want_in
)
SELECT DISTINCT s.*, nb.depth, nb.dir
FROM nb JOIN symbols s ON s.id = nb.id
ORDER BY nb.depth ASC, s.rank DESC;   -- nearest first, then heaviest within a ring
```

**Budget truncation.** Per the protocol, order is **depth-then-rank**: keep the
inner rings whole, drop the lowest-rank nodes of the outermost ring first, set
`truncated` and `dropped`. This preserves locality (the immediate wiring) while
spending the remaining budget on the architecturally heaviest distant nodes.

#### Worked example — `neighborhood(depth=2, dir="both")`

Center: `src/config/load.ts#ConfigLoader/parse:method` (rank 0.74).

```jsonc
// request
{ "symbol": "src/config/load.ts#ConfigLoader/parse:method",
  "depth": 2, "dir": "both", "kinds": ["call","import","typeref"],
  "max_tokens": 2500 }
```

CTE expansion:

- depth 0: `…/parse` (self).
- depth 1 `in` (callers): `src/cli/main.ts#run:function` (0.66, call),
  `src/config/load.ts#ConfigLoader/reload:method` (0.40, call).
- depth 1 `out` (callees/refs): `src/config/schema.ts#Schema:class` (0.81, typeref),
  `src/config/load.ts#readFile:function` (0.22, call).
- depth 2 `in`: `src/cli/main.ts#main:function` (0.71, call → `run`).
- depth 2 `out`: `src/config/schema.ts#Schema/validate:method` (0.58, call from
  `Schema` usage), `src/util/fs.ts#exists:function` (0.12, call → `readFile`).

Ordered depth-then-rank, then filled to 2500 tokens: the depth-0/1 nodes and the
high-rank depth-2 nodes (`main`, `Schema/validate`) fit; the depth-2 tail
`util/fs.ts#exists` (rank 0.12) is dropped.

```jsonc
// result (abridged)
{ "generation": 412, "center": "…/parse:method",
  "nodes": [
    { "key": "…/parse:method", "rank": 0.74, "depth": 0, "dir": "self" },
    { "key": "src/config/schema.ts#Schema:class", "rank": 0.81, "depth": 1, "dir": "out" },
    { "key": "src/cli/main.ts#main:function", "rank": 0.71, "depth": 2, "dir": "in" },
    { "key": "src/cli/main.ts#run:function", "rank": 0.66, "depth": 1, "dir": "in" },
    { "key": "src/config/schema.ts#Schema/validate:method", "rank": 0.58, "depth": 2, "dir": "out" },
    { "key": "…/reload:method", "rank": 0.40, "depth": 1, "dir": "in" },
    { "key": "…/readFile:function", "rank": 0.22, "depth": 1, "dir": "out" }
  ],
  "edges": [
    { "src": "…/parse:method", "dst": "src/config/schema.ts#Schema:class", "kind": "typeref" },
    { "src": "src/cli/main.ts#run:function", "dst": "…/parse:method", "kind": "call" }
    /* … */
  ],
  "truncated": true, "dropped": 1 }
```

The agent sees the load-bearing context — who drives `parse`, what type it produces,
what validates it — without the file's full text and without the low-rank tail.

---

## 5. Incremental rank maintenance (US 9)

A full PageRank per keystroke violates the interactive-resync constraint (single-file
edit p95 ≤ 200 ms, [README.md](./README.md) §constraint 1). But a one-line edit
changes a handful of edges; recomputing global centrality from scratch is wasteful.
`Rank::apply(delta)` ([01](./01-architecture.md) §3) does a **localized re-rank** —
the detailed delta contract lives in [10-incremental-sync.md](./10-incremental-sync.md);
the strategy:

1. **Compute the dirty edge set.** Parse.apply has just delete/reinserted symbols
   and edges for the dirty files (§7). The changed/added/removed edges define a set
   of *affected nodes*: their endpoints plus the **k-hop closure** (default k=2 via
   `[graph].rerank_radius`) around them. Rank mass moves locally; nodes far away are
   essentially unperturbed.
2. **Re-rank the affected region.** Run PageRank restricted to the affected subgraph,
   holding the boundary nodes' incoming mass fixed (a personalized/localized push).
   This touches hundreds of nodes, not tens of thousands, and writes back only those
   `symbols.rank` rows — bumping `generation` on them.
3. **Drift correction.** Localized re-rank drifts slightly from the true global
   stationary distribution. We bound it by running a **full PageRank periodically** —
   on idle (no pending deltas, the same window WAL checkpointing uses,
   [02](./02-data-model.md) §6), or after N localized updates
   (`[graph].full_rerank_every`, default 200). Cold start and `carto index` always do
   the full computation.

**Tradeoff stated.** Localized re-rank trades a small, bounded ranking error between
full recomputations for the latency budget. Because rank is only a *sort/truncation*
key — never a correctness gate — a few-percent positional drift on mid-rank symbols
is invisible to the agent (top and bottom of the list are stable); the periodic full
pass erases accumulated drift before it matters. This is what earns H2 its **US 9**.

---

## 6. Cross-language graph unification

Symbols from every language share **one `symbols` table and one id/`stable_key`
namespace** ([02](./02-data-model.md) §3). There is no per-language partition: a
TypeScript class and a Rust struct are rows that differ only by their `files.lang`.
Edges are therefore free to cross language boundaries — nothing in the schema or the
recursive CTEs cares about language. What differs is **what the resolver can see**:

**Captured in v1 (same-language, in-repo):** the normal case — call/typeref/inherit/
import edges resolved by §2.3's import-aware, scoped name matching within a language.
This is the overwhelming majority of real edges.

**Cross-language edges, v1 best-effort:**

- **Codegen / shared schema by name.** When languages share a generated artifact or
  a schema (protobuf/OpenAPI types, a generated client), the *names line up*. We
  resolve a cross-language `typeref`/`call` by **fqn/name match across languages**
  when an unambiguous def exists in another language, marked `resolved=1` only when
  the match is unique; otherwise `resolved=0`. This catches "TS calls the generated
  gRPC stub whose name matches the Rust service method."
- **FFI / explicit boundaries.** `extern "C"`, N-API/`napi` exports, WASM imports,
  Python `ctypes`/`cffi` symbols — resolved by **declared symbol name** on the
  boundary (the export name is the join key). Edge kind stays `call`.
- **RPC / network boundaries.** Captured only when the call target is a **named,
  statically resolvable handler** (a route handler referenced by a typed client
  generated from the same IDL). Free-form string routes are not resolved.

**Not captured in v1 (honest scope):**

- Calls dispatched purely through a string (HTTP path literal, dynamic `require`,
  reflection by name) with no generated typed surface.
- Build-system or DI-container wiring that connects symbols only at runtime config
  time.
- Language pairs where names are deliberately mangled across the boundary with no
  recoverable mapping.

Cross-language edges are down-weighted like other heuristic edges so a wrong guess
can't inflate rank, and they're always inspectable via `resolved`.

---

## 7. Index integrity (II 8)

A small edit must not invalidate the global graph, and it doesn't, because identity
is decoupled from content:

- **`stable_key` handles** ([02](./02-data-model.md) §3) are
  `path#container/name:kind` — independent of byte offsets. Editing a function body,
  shifting lines, or changing a signature **preserves the key**, so every edge
  pointing at that symbol, every handle the agent is holding, and its co-change
  history survive the edit. Only a rename breaks the key, and H4 emits a key-remap
  row so even that follows through ([02](./02-data-model.md) §3).
- **Per-file delete/reinsert.** Parse.apply rewrites only the dirty files' symbols
  and edges inside one transaction ([02](./02-data-model.md) §4). `ON DELETE CASCADE`
  on `edges.src_id`/`dst_id` removes a deleted file's outgoing edges automatically;
  the resolver re-runs over the new defs and re-binds. Because endpoints are matched
  by `stable_key` on reinsert, an edge from an *untouched* file into an edited file
  re-binds to the same symbol id when the symbol still exists — the graph stays
  coherent across the seam.
- **`content_hash` dirty gate.** A no-op save (identical bytes) is dropped before any
  parse or rank work ([02](./02-data-model.md) §4), so editor churn never perturbs
  the graph at all.

The blast radius of an edit is thus *its files' symbols/edges plus the localized
rank region* (§5) — never the whole index. That bounded, content-stable update is
what gives H2 its **II 8**.

---

## 8. Open questions / risks

- **Resolution accuracy (the core risk).** Heuristic name+scope resolution will both
  invent and miss edges. We bound it (import-scope, receiver typing, `resolved`
  flagging, kind down-weighting) and measure precision/recall against an LSP oracle
  in [11-benchmark-harness.md](./11-benchmark-harness.md). Open: per-language tuning
  budget, and whether to offer the optional opt-in LSP backend (rejected as *core*,
  not as an *enhancement*) to upgrade `resolved=0` edges where one is available.
- **Dynamic dispatch / polymorphism.** A call through an interface/trait/virtual
  method has many possible targets. v1 either edges to the declared type's member
  (precise but may miss overrides) or fans low-weight unresolved edges to all
  implementors (recall over precision). The right default per kind is unsettled.
- **Reflection / metaprogramming.** Calls built from strings, decorators/annotations
  that register handlers, macros and codegen that synthesize symbols not present in
  source — invisible to a tree-sitter pass. Partly mitigated by indexing generated
  output when it's on disk; fundamentally a recall gap.
- **Rank semantics for hubs vs. utilities.** PageRank rewards *depended-upon*; a
  trivial logging util can score high purely on fan-in. Edge weighting (§3) and
  optionally penalizing very-high-in-degree leaf utilities are levers we may need to
  tune so "central" tracks "architecturally important," not just "frequently
  imported."
- **Localized re-rank drift bound.** We assert drift is invisible between full
  passes; the actual error envelope and the right `full_rerank_every` /
  `rerank_radius` defaults need empirical validation on real repos (§5,
  [10-incremental-sync.md](./10-incremental-sync.md)).
