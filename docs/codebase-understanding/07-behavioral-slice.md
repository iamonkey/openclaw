# 07 — Behavioral Slice Hydration (post-v1)

> **Status: RESERVE hypothesis. Spec'd now, sequenced after v1.** v1 ships
> [H3 Impact-Radius](./06-h3-impact-radius.md) as the dependency-reasoning
> primitive. This document defines the *Behavioral Slice* alternative and the
> swap criterion that would promote it. Do not build it until that criterion
> trips. The `slice` tool in [03-mcp-tool-surface.md](./03-mcp-tool-surface.md)
> is marked post-v1 for the same reason.

Backs the `slice(symbol, direction, criterion)` tool. Goal: return the **minimal
set of statements** that affect (backward slice) or are affected by (forward
slice) a target criterion — statement granularity, not whole-function
granularity — for maximal precision per token in dependency reasoning.

| Metric | Score | Note |
|---|---|---|
| TRP — Token Reduction | 7 | Statement-level minimality beats body dumps, but slices can fan out. |
| AF — Architectural Fidelity | 8 | Follows real def-use + control dependence, not heuristics. |
| IC — Implementation Complexity | 7 | Dependence graph construction without an LSP is the hard part. |
| II — Index Integrity | 7 | Def-use must recompute on every body edit. |
| US — Update Speed | 7 | Intra-procedural recompute is cheap; cross-fn summaries less so. |
| Ceiling | `[ER]` | Precise slicing compounds with H2/H5; exponential, not plateau. |

---

## 1. Status banner & swap criterion

Behavioral Slice and H3 answer overlapping questions ("what depends on this?")
with different substrates: H3 uses **git co-change** (history), Slice uses
**program dependence** (structure). We cannot afford to build both for v1, and
the two are close enough in TRP/AF that the choice must be made on **measured
task accuracy**, not intuition. v1 therefore ships H3 (cheaper, language-agnostic,
captures non-code coupling for free) and reserves Slice.

**Swap criterion (promote Slice from reserve to build):**

> On the efficiency frontier in [11-benchmark-harness.md](./11-benchmark-harness.md),
> for the *dependency-reasoning task class* (TRA — Task Retrieval Accuracy on
> "what affects this value / what breaks if I change this"), precise slicing must
> beat H3's empirical coupling on **both** axes simultaneously:
>
> 1. **Accuracy:** Slice TRA ≥ H3 TRA + 5 points on the dependency-reasoning suite, and
> 2. **Efficiency:** Slice tokens-per-correct-answer ≤ H3's (i.e. the precision
>    gain is not paid for with a larger context).
>
> If Slice wins accuracy but loses on tokens (slice fan-out blows the budget), it
> stays reserved. If H3 holds either axis, H3 stays the v1 primitive. The matrix
> in §7 enumerates the regimes.

Until then this spec is a design contract only. No tables in
[02-data-model.md](./02-data-model.md) are created for it in v1; §5 proposes them
as **additions** to be applied if and when the criterion trips.

---

## 2. Program slicing primer (applied here)

A **program slice** is the subset of statements that are relevant to a chosen
*slicing criterion* `C = (location, variable)` — a point in the program and a
value observed there. Two directions:

- **Backward slice** — every statement that *can affect* the value at `C`. "What
  computes this return?" / "What feeds this assertion?" Answers root-cause and
  "why is this value wrong" questions.
- **Forward slice** — every statement that *is affected by* the value at `C`.
  "If I change this parameter, what downstream computation moves?" Answers
  impact and "what breaks if I touch this" questions.

Slicing closes over two dependence relations:

- **Data dependence (def-use):** statement *B* data-depends on *A* if *A* defines
  a variable that *B* uses, with a def-clear path between them (no intervening
  redefinition).
- **Control dependence:** statement *B* control-depends on *A* if *A* is a
  branch/loop predicate whose outcome decides whether *B* executes.

The slice is the transitive closure over both, starting from `C`, walking
backward (predecessors) or forward (successors).

**Why this is higher precision than the alternatives:**

- **vs H3 (historical coupling):** H3 says *"these changed together 14× in git
  history"* — a correlation that includes test files, config, docs, and
  refactors that have nothing to do with the dataflow. Slice says *"these exact
  statements are on the dependence path to your value"* — a causal,
  history-free fact. H3 has recall on non-code coupling that Slice can't see;
  Slice has precision on dataflow that H3 only approximates.
- **vs H2 (symbol-granularity neighborhood):** `neighborhood` returns whole
  *symbols* (functions/methods) connected by call/typeref edges. A slice returns
  the **statements inside** those symbols that actually participate. For a 60-line
  function where only 7 lines feed the return, H2 hands the agent all 60; Slice
  hands it 7. Statement granularity vs symbol granularity is the entire TRP case.

---

## 3. The dependence substrate (tree-sitter, no LSP)

Slicing needs a dependence graph. We build it **intra-procedurally** over the
tree-sitter AST already produced by `carto-parse`
([01-architecture.md](./01-architecture.md) §3), with **no language server**.
This is the crux — and the precision ceiling.

### What's tractable language-agnostically

Within a single function body, with a per-language tree-sitter query pack:

- **Statement segmentation.** Enumerate statement nodes (assignments, calls,
  returns, branch/loop headers) from the AST.
- **Local def-use.** For each statement, extract *defs* (LHS of assignments,
  declarations, formal params, `for`/`with` binders) and *uses* (identifiers in
  RHS, conditions, call args). This is a syntactic walk keyed by identifier name
  within the function's lexical scope — no type resolution required.
- **Intra-procedural control dependence.** Build a control-flow graph from the
  AST's nesting (if/else, loop, switch, early return/break/continue), compute the
  post-dominator tree, and derive control dependence. Tree-sitter gives us the
  structured nesting directly; unstructured `goto` is rare in target languages
  and handled conservatively (treat as a fan-out predicate).
- **Reaching definitions** within the function via a standard forward dataflow
  fixpoint over that CFG. Cheap: function bodies are small.

A per-language query pack (`grammars/<lang>/defuse.scm`) tags `@def`, `@use`,
`@predicate`, and scope-introducing nodes, the same mechanism H1/H2 already use
for tags. Adding a language = writing that query pack; the dataflow engine is
language-agnostic.

### What's hard (and where we approximate)

A tree-sitter-only build cannot resolve semantics. We **conservatively
over-approximate** to stay sound-ish (favor false positives — an extra statement
in the slice — over false negatives — a dropped dependence):

| Hard problem | Why tree-sitter can't | Conservative handling |
|---|---|---|
| **Aliasing** (`b = a; b.x = 1`) | No points-to analysis | Assume any field/index write may affect any aliased read of the same base name; widen the def to the whole object. |
| **Pointers / references** | No memory model | Treat through-reference mutation as a def of the referent's name; over-include. |
| **Dynamic dispatch** (`obj.method()`) | Receiver type unknown | Treat the call as a use of all args + a def of any out-params/receiver; do not try to pick the callee body (see §4 resolution gate). |
| **Inter-procedural effects** | Single-function scope | A call is a barrier: assume it may read all its args and write its receiver + return. Refine only via §4 summaries. |
| **Globals / captured state** | No whole-program escape analysis | Module-level names referenced inside the function are treated as live-in defs (over-include). |
| **Reflection / `eval` / macros** | Opaque to AST | Mark the enclosing statement `imprecise: true`; never silently drop. |

Every slice node carries a `precision` flag. Where the engine widened (alias,
dynamic dispatch, opaque call), the response says so, so the agent never mistakes
an over-approximation for a proven-minimal set. **This is the honest ceiling:
without type information the slice is sound-ish (rarely drops a real dependence)
but not minimal (sometimes includes a spurious one).** That is the right side to
err on for an LLM context tool.

---

## 4. Inter-procedural extension (bounded, summary-based)

A pure intra-procedural slice stops at the first call. To follow dataflow across
boundaries we reuse H2's `call` edges in the `edges` table
([02-data-model.md](./02-data-model.md) §2) rather than inventing a new graph.

- **Depth bound.** Slicing descends/ascends `call` edges to a configured
  `max_call_depth` (default 2). Beyond that, the call stays a barrier and the
  node is marked `precision: "depth-bounded"`. This keeps the closure finite on
  recursive and highly-connected code.
- **Procedure summaries.** For each callee we compute a compact **dependence
  summary** once and cache it: which formal parameters flow to the return, which
  flow to which out-params/receiver fields, and whether the body reads/writes
  module state. A summary is `param_index → {affects: [return | out:field | global]}`.
  Backward slicing through a call then consults the summary instead of re-slicing
  the callee inline; only summarized-relevant arguments get pulled into the
  caller's slice.
- **Resolution gate.** Summaries are only computed for calls whose `edges.resolved
  = 1` (H2 resolved the callee to a concrete def). Unresolved/dynamic calls
  (`resolved = 0`) fall back to the §3 conservative barrier. This ties slice
  precision directly to H2's resolution quality — a clean dependency, not a new
  failure mode.
- **Recursion / cycles.** Summaries are fixed-pointed over the call SCC; an
  in-progress summary defaults to "all params affect all outputs" until it
  converges, preserving soundness during computation.

Summaries are the tractability lever: they bound work to *one* slice of each
reachable callee, reused across all callers, instead of re-expanding bodies per
query.

---

## 5. Storage & tools

### Precompute vs on-demand

Slices are **query-specific** (criterion = location + variable + direction), so
the slice itself is computed **on demand**. What we cache is the reusable
substrate underneath it:

- **Cached (incrementally maintained):** per-function def-use facts and the
  control-dependence relation; per-symbol procedure summaries (§4).
- **On demand (per `slice` call):** the transitive closure from the criterion
  over the cached facts, bounded by `max_call_depth` and `max_tokens`.

This mirrors H2: store the graph, traverse at query time.

### Proposed schema additions (apply only if the swap criterion trips)

These are **additions** to [02-data-model.md](./02-data-model.md), not part of the
v1 schema. Keyed by `symbols.id` so they ride the existing
delete-and-reinsert-per-dirty-file incremental path (§02 §4).

```sql
-- ── Def-use facts (intra-procedural), one row per dependence edge ─────────
-- Statement-granular. `stmt_*` are byte spans within the owning function body.
CREATE TABLE defuse (
  symbol_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE, -- owning function
  def_start   INTEGER NOT NULL,   -- defining statement span (byte offsets)
  def_end     INTEGER NOT NULL,
  use_start   INTEGER NOT NULL,   -- using statement span
  use_end     INTEGER NOT NULL,
  var         TEXT NOT NULL,      -- variable name carrying the dependence
  dep_kind    TEXT NOT NULL,      -- data | control
  precision   TEXT NOT NULL DEFAULT 'exact', -- exact | alias | dispatch | depth-bounded | opaque
  generation  INTEGER NOT NULL,
  PRIMARY KEY (symbol_id, def_start, use_start, var, dep_kind)
);
CREATE INDEX idx_defuse_symbol ON defuse(symbol_id);
CREATE INDEX idx_defuse_use    ON defuse(symbol_id, use_start);   -- backward walk
CREATE INDEX idx_defuse_def    ON defuse(symbol_id, def_start);   -- forward walk

-- ── Procedure dependence summaries (inter-procedural lever, §4) ───────────
CREATE TABLE proc_summary (
  symbol_id   INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  param_index INTEGER NOT NULL,  -- formal parameter position (-1 = receiver/this)
  affects     TEXT NOT NULL,     -- JSON: ["return","out:field","global:name"]
  reads_state INTEGER NOT NULL DEFAULT 0,
  writes_state INTEGER NOT NULL DEFAULT 0,
  generation  INTEGER NOT NULL,
  PRIMARY KEY (symbol_id, param_index)
);
CREATE INDEX idx_proc_summary_symbol ON proc_summary(symbol_id);
```

No new top-level entity is needed: `defuse` hangs off `symbols`, summaries hang
off `symbols`, and cross-function descent reuses `edges(kind='call')`.

### The `slice` tool contract

Matches [03-mcp-tool-surface.md](./03-mcp-tool-surface.md) §2. Thin adapter over a
new `IndexReader::slice` method, plus the shared budget protocol (§03 §4).

```jsonc
// params
{
  "symbol": "src/config/load.ts#ConfigLoader/parse:method",
  "direction": "backward",      // backward = what affects C | forward = what C affects
  "criterion": "return",        // "return" | a variable name | "row:NN" | "row:NN:var"
  "max_call_depth": 2,          // optional; default from config
  "max_tokens": 2500            // optional; truncate by distance-from-criterion
}
// result
{
  "generation": 412,
  "token_est": 540,
  "center": "src/config/load.ts#ConfigLoader/parse:method",
  "direction": "backward",
  "criterion": { "location": "row:78", "variable": "cfg" },
  "statements": [
    { "path": "src/config/load.ts", "row": 41, "text": "const raw = TOML.parse(src);",
      "dep": "data", "via": "raw", "precision": "exact", "depth": 0 },
    { "path": "src/config/load.ts", "row": 52, "text": "if (raw.version !== 2) raw = migrate(raw);",
      "dep": "control", "via": "raw", "precision": "exact", "depth": 0 },
    { "path": "src/config/migrate.ts", "row": 9, "text": "return upgrade(old);",
      "dep": "data", "via": "return", "precision": "depth-bounded", "depth": 1 }
  ],
  "truncated": false,
  "dropped": 0
}
```

**Budget truncation.** Order statements by **distance from the criterion** in the
dependence graph (nearest dependences first — they're most explanatory), tie-break
by H2 `rank` of the owning symbol. Fill until the next statement would exceed
`max_tokens`; set `truncated: true` and report `dropped: <count>` so a pruned tail
is never silent (same contract as every other retrieval tool). Statements pulled
across a call boundary always carry their `path` so a multi-file slice stays
navigable.

---

## 6. Why IC / II / US are middling (7 / 7 / 7)

- **IC = 7.** The dataflow engine (CFG construction, post-dominators, reaching
  definitions, summary fixpoint) is real compiler work, and every supported
  language needs a correct `defuse.scm` query pack plus alias/dispatch widening
  rules. More than H2's edge extraction, less than a full LSP. The conservative
  approximation logic is where the bugs hide.
- **II = 7.** Def-use facts are **body-sensitive**: unlike H2 edges (which often
  survive a body edit), *any* statement change inside a function invalidates that
  function's `defuse` rows and may invalidate the summaries of its callers. The
  blast radius of a one-line edit is wider than for the symbol graph.
- **US = 7.** Intra-procedural recompute on a dirty function is fast (small
  bodies, local fixpoint). The cost is **summary maintenance**: re-summarizing a
  function whose body changed, then re-validating every caller's summary up the
  `call` graph to `max_call_depth`. Bounded, but more than the near-free
  append H3 does on a new commit. This caps US below H2/H3's 8–9.

These three are exactly why Slice is the reserve, not the v1 pick: it is more
expensive to keep correct under fast mutation than co-change is to keep current.

---

## 7. H3-vs-Slice decision matrix

This is the table that justifies the deferral. Same question class, two
substrates; pick per regime.

| Dimension | H3 — Git co-change | Slice — Program dependence |
|---|---|---|
| **Signal** | Historical correlation (commits) | Causal dataflow + control dependence |
| **Granularity** | Symbol / file | **Statement** |
| **Needs history?** | Yes — weak on new/shallow repos | **No** — works on first commit |
| **Captures non-code coupling** | **Yes** (tests, config, docs that co-change) | No — only what's in the dependence graph |
| **Cross-language coupling** | **Yes** (file-level, language-agnostic) | Per-language query pack required |
| **Precision on dataflow** | Low (correlation includes noise) | **High** (causal, minimal-ish) |
| **Cost to keep current** | Cheap (append per commit) | Moderate (def-use + summary recompute) |
| **Failure mode** | False coupling from refactor commits | Over-approximation from aliasing/dispatch |
| **Best for** | "What tends to change with this?" / blast radius across the whole repo incl. non-code | "What exactly computes / is computed from this value?" / root-cause + minimal context |

**Verdict for v1:** H3 wins the *first bet* because it is cheap, language-agnostic
out of the box, needs no per-language dependence engine, and captures
test/config/doc coupling that Slice structurally cannot see. Slice wins precisely
when the agent needs **statement-level dataflow truth with no history available**
— which is real, but is a narrower task class than the general blast-radius
question H3 serves. Build the broad, cheap primitive first; promote the precise,
expensive one only when §1's measured criterion proves the precision pays for
itself.

---

## 8. Worked example — backward slice of a return value

```ts
// src/config/load.ts
 40  parse(src: string): Config {
 41    const raw = TOML.parse(src);              // def raw
 42    const env = process.env.NODE_ENV ?? "dev"; // def env   (NOT on path to return)
 43    log.debug("parsing config", env.length);   // use env   (NOT on path)
 44    const defaults = this.loadDefaults();      // def defaults
 45    let merged = { ...defaults, ...raw };      // def merged ← defaults, raw
 46    if (raw.version !== 2) {                    // control predicate on raw
 47      merged = migrate(merged);                //   def merged ← merged (migrate)
 48    }
 49    this.audit.push(Date.now());               // side effect (NOT on path)
 50    const cfg = validate(merged);              // def cfg ← merged
 51    return cfg;                                 // criterion C = (row 51, cfg)
 52  }
```

**Backward slice of `(row 51, cfg)`** — transitive def-use + control closure:

```
51 return cfg            ← criterion
50 cfg = validate(merged)   data: cfg ← merged
47 merged = migrate(merged) data: merged ← merged
46 if (raw.version !== 2)   control: guards line 47
45 merged = {...defaults,...raw}  data: merged ← defaults, raw
44 defaults = loadDefaults()      data: ← defaults
41 raw = TOML.parse(src)          data: ← raw
```

**7 statements** in the slice (rows 41, 44, 45, 46, 47, 50, 51). **Dropped as
irrelevant:** rows 42, 43 (env logging), 49 (audit side effect) — they neither
affect `cfg` nor are guarded by anything that does.

| Approach | What the agent receives | ~tokens |
|---|---|---|
| `expand` (whole body, Tier-2) | all 12 lines of `parse` | ~120 |
| `slice` (backward, `cfg`) | 7 dependence-relevant statements | ~55 |

~54% fewer tokens here, and — more importantly — the noise (logging, audit) is
gone, so the agent reasons about *only* what produces the value. The gap widens on
larger functions: a 200-line method where 15 lines feed the return is where
statement-granularity becomes an order-of-magnitude win and the case for promotion
gets strong on the §11 frontier.

---

## 9. Open questions & risks

- **Slice fan-out vs budget.** Backward slices on heavily-shared state can
  explode toward "most of the function plus its callees." Truncation (§5) keeps it
  bounded, but a truncated slice is unsound (a real dependence may be dropped) —
  the response must flag `truncated` loudly. Open: should over-budget slices
  *degrade to H2 neighborhood* rather than return a partial dependence set?
- **Precision-flag trust.** Does an LLM correctly down-weight `precision: "alias"`
  / `"dispatch"` nodes, or does the flag get ignored? Needs a benchmark probe;
  if ignored, the over-approximation noise erodes the TRP advantage.
- **Query-pack burden.** Each language needs a hand-written, correct
  `defuse.scm` + widening rules. Mis-tagging a `@def` silently corrupts every
  slice in that language. How do we test query packs for dependence correctness,
  not just tag coverage?
- **Summary invalidation cost at scale.** On hot files in a large repo, does
  caller-summary revalidation stay inside the 200 ms single-edit p95 budget
  ([README](./README.md) constraint 1), or does it need to go async/lazy
  (recompute summary on next `slice` call rather than on edit)?
- **Aliasing realism.** Is the "widen to whole object on any field write"
  approximation *too* conservative on idiomatic OO code, pulling in so much that
  the precision win over H2 evaporates? This is the empirical question that
  decides the swap — and the reason it stays in §11's hands, not this doc's.
