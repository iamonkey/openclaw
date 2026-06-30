# 09 — H5: Adaptive Context Budgeter

The **integration spine**. H1–H4 are retrieval primitives that each answer one
narrow question ([04](./04-h1-recursive-repo-map.md),
[05](./05-h2-symbol-rank-topology.md), [06](./06-h3-impact-radius.md),
[08](./08-h4-semantic-diff-hydration.md)). H5 is the query-planner meta-layer that
compiles a **token-budget-constrained retrieval plan** across all of them: given a
task and a hard token budget, it chooses which primitives to call, in what order,
with what params, to **maximize expected relevance per token**.

H5 backs the `plan_retrieval(task, token_budget)` tool
([03](./03-mcp-tool-surface.md) §2). It is a pure **consumer** of the index — it
calls `IndexReader` methods ([01](./01-architecture.md) §3), composes their
outputs, and **never writes the store** and **never registers as a `Producer`**
([01](./01-architecture.md) §3). It is the only component that issues multiple
reads to satisfy one agent request.

| Score | TRP | AF | IC | II | US | Ceiling |
|---|---|---|---|---|---|---|
| H5 | 9 | 9 | 8 | 6 | 6 | [ER] |

---

## 1. Goal: a system, not a pile of primitives

H1–H4 alone leave a hard problem on the agent's side of the wire. To investigate a
bug the agent must decide, per task: outline at tier 0 or tier 1? neighborhood
depth 1, 2, or 3? include co-change or just static impact? expand which symbols
fully and which to leave as signatures? Every one of those knobs trades tokens for
relevance, and the right setting depends on the *task* and the *budget*, not on a
fixed default. Hand-tuning that for every query is exactly the cognitive load a
context tool is supposed to remove.

H5 compiles that decision. The agent says *what it wants to do* and *how many
tokens it can spend*; H5 returns a concrete, executed plan and the hydrated
context. The objective is mechanical: **maximize expected relevance per token under
a hard budget `B`**. The primitives already expose the two numbers this needs —
a relevance signal (H2 `rank`, H3 `weight`, query match) and a cost (`token_est`,
cached on the schema, [02](./02-data-model.md) §6). H5 is the layer that turns
"a set of tools" into "a coherent retrieval system" by optimizing across them
instead of leaving the optimization to the caller.

Why this is the spine and not a sixth primitive: H1–H4 each maximize relevance
*within* their own output. None of them can trade a tier-1 outline against a
neighborhood expansion, because they don't see each other. H5 is the only place
that comparison happens.

---

## 2. Task classification → retrieval strategy

The first stage maps a free-text `task` to a **strategy**: an ordered template of
tool stages with default params. Classification in v1 is a **cheap heuristic /
keyword matcher** — no model call on the hot path. The three benchmark task classes
([11](./11-benchmark-harness.md)) plus a default:

### Class A — bug localize-and-patch
Trigger: "fix", "bug", "off-by-one", "crash", "throws", "incorrect", "regression",
a stack-trace shape, or a file:line.
Strategy — locate, then read the site and its immediate blast radius:
```
search_symbols(query)            → candidate sites
outline(path, tier=1)            → signatures around the best candidate
neighborhood(sym, depth=1, both) → direct callers/callees (what the fix touches)
impact(sym, cochange=true)       → what historically breaks with this symbol
expand(sym)                      → full body of the patch site (read live)
```

### Class B — "where is X implemented"
Trigger: "where is", "where's", "find", "locate", "which file", "implementation of".
Strategy — find and confirm, shallow:
```
search_symbols(X)                → ranked matches
get_symbol(best)                 → signature + location + rank
outline(file_of_best, tier=1)    → surrounding context to confirm the right one
expand(best)  [shallow / optional] → only if budget allows and one clear winner
```

### Class C — "what breaks if I change Y"
Trigger: "what breaks", "impact of", "safe to change", "what depends on",
"who uses", "blast radius", "ripple".
Strategy — anchor, then radiate outward:
```
get_symbol(Y)                    → anchor the target
impact(Y, cochange=true, depth=2)→ static + co-change blast radius (H3)
neighborhood(Y, depth=2, in)     → reverse-dependency frontier, rank-ordered
expand  [omitted by default]     → callers stay at signature tier to fit budget
```

### Default — exploratory / unclassified
No strong keyword signal. Strategy — top-down map, then drill:
```
outline("", tier=0)              → repo map, purpose lines
search_symbols(salient nouns)    → entry candidates from the task text
outline(path, tier=1) / neighborhood(depth=1) on the top candidate
```

The strategy is a **template, not a fixed budget**: it fixes the *ordering and
candidate generation*; §3 decides how much budget each stage actually gets and what
gets truncated. A stage that wins no budget is dropped from the plan.

> **LLM router (later).** A small model could classify and even draft params more
> accurately than keywords. It is allowed — but only here, at plan time, which is
> **per-query, not per-edit**. It must never sit in the latency-critical resync
> path ([01](./01-architecture.md) §5, [10](./10-incremental-sync.md)); resync
> performance is gated independently of planning. v1 stays heuristic; the router is
> an opt-in upgrade behind the same `plan_retrieval` contract.

---

## 3. Budget allocation: greedy expected-value fill

Given budget `B`, H5 does **not** statically partition `B` across stages. It runs
the chosen strategy's stages as **candidate generators**, scores every candidate
*item* they produce, and fills a single global budget by **best value/cost ratio
first**. This is the shared budget protocol of [03](./03-mcp-tool-surface.md) §4
applied across tools instead of within one.

Each candidate retrieval item has:
- **`cost`** = `token_est`, cached on the row (`skeletons.token_est`,
  `ast_diffs.token_est`) or estimated by the same bytes/4 heuristic
  ([02](./02-data-model.md) §6). No re-tokenizing at plan time.
- **`value`** = an estimate of expected relevance, combining the signals the
  primitives already expose:

```
value(item) =
      w_match * query_match(item, task)      // lexical/fuzzy overlap with task text
    + w_rank  * normalize(rank)              // H2 centrality (02 §5 ORDER BY rank)
    + w_impact* weight                       // H3 impact weight, if from impact()
    - w_depth * graph_depth                  // discount distance from the anchor
    + w_stage * stage_prior(strategy, stage) // class-specific prior (e.g. patch
                                             //   site > distant caller for Class A)
```

`w_*` are config-tunable (`[budget]` in [01](./01-architecture.md) §6) and held
constant in v1; §8 replaces them with a learned model. Candidates are filled by
**`value/cost` ratio**, greatest first, until `B` is exhausted — the classic
fractional-knapsack greedy, which is optimal when items are independent and
near-optimal under the mild dependencies here (an `expand` depends on its symbol
being located first; we enforce that with stage ordering, below).

```text
plan_retrieval(task, B):
    strategy   = classify(task)                       # §2, cheap heuristic
    snapshot   = store.read_snapshot()                # pin one generation (01 §5)
    plan       = []                                   # auditable record
    context    = []                                   # hydrated payloads
    spent      = 0
    candidates = []                                   # min-heap by -value/cost

    # Stage 1 is always a locator (search_symbols/get_symbol/outline tier 0).
    # It is cheap and unlocks all downstream candidate generation.
    anchor = run_locator(strategy, task, snapshot)
    spend(anchor)                                      # always afford the anchor

    # Generate candidates from each downstream stage WITHOUT hydrating yet:
    # we ask each tool for its ranked item list + per-item token_est only.
    for stage in strategy.stages[1:]:
        for item in stage.enumerate(anchor, snapshot): # metadata only, no bodies
            item.value = score(item, task, strategy, stage)   # §3 formula
            candidates.push(item)

    # Greedy fractional-knapsack fill on value/cost.
    while candidates not empty and spent < B:
        item = candidates.pop_best()                   # max value/cost
        if item.requires and item.requires not in hydrated:
            continue                                   # dependency not (yet) met
        if spent + item.cost > B:
            # try to truncate this item to remaining budget (tools support max_tokens)
            item = item.truncate_to(B - spent)         # 03 §4: rank-ordered drop
            if item is None: continue                  # indivisible & too big → skip
        payload = hydrate(item, snapshot)              # the actual tool call now
        context.append(payload)
        plan.append({ tool: item.tool, args: item.args,
                      why: item.why, token_est: item.cost })
        spent += payload.token_est

    return { generation: snapshot.generation, budget: B, spent_est: spent,
             plan: plan, context: context }
```

Notes:
- **Two-phase per stage.** Enumerate (cheap: ids, ranks, weights, `token_est`)
  then hydrate (the real read) only for items that win budget. This is what keeps
  H5 from paying to fetch context it will discard.
- **Truncation reuses the per-tool contract.** When a winning item overshoots the
  remaining budget, H5 calls the tool with `max_tokens = B - spent`; the tool drops
  its lowest-rank entries and reports `truncated`/`dropped` ([03](./03-mcp-tool-surface.md)
  §4). H5 never invents a truncation of its own.
- **Dependencies.** `expand(sym)` requires `sym` to have been located; co-change
  candidates require their anchor. `requires` gates these so the greedy never
  hydrates an orphan.
- **Anchor is privileged.** The locator stage is always afforded (it is cheap and
  every other candidate depends on it); only post-anchor stages compete for `B`.

---

## 4. The plan as a first-class, auditable object

`plan_retrieval` returns **both** the plan and the context
([03](./03-mcp-tool-surface.md) §2):

```jsonc
{
  "generation": 412,
  "budget": 6000,
  "spent_est": 5120,
  "task_class": "bug-localize-patch",      // which §2 strategy fired
  "plan": [
    { "tool": "search_symbols", "args": { "query": "paginat" },
      "why": "locate pagination logic from task nouns", "token_est": 180 },
    { "tool": "neighborhood",
      "args": { "symbol": "src/list/page.ts#paginate:function",
                "depth": 1, "dir": "both" },
      "why": "direct callers/callees the off-by-one fix may touch",
      "token_est": 1430 },
    { "tool": "expand",
      "args": { "symbol": "src/list/page.ts#paginate:function" },
      "why": "read the off-by-one site in full", "token_est": 320 }
  ],
  "context": [ /* hydrated payloads, already budget-truncated */ ]
}
```

- **`plan`** is the audit trail: every tool call, its args, a one-line **`why`**,
  and the tokens it cost. **`context`** is the pre-truncated hydrated payload so the
  agent acts in **one round-trip** instead of replaying the plan itself.
- **`dry_run: true`** returns the `plan` only (no `context`, no hydration) — for
  inspection, cost preview, and tests.

Why auditability is load-bearing:
- **Trust.** The agent (and a human reviewing the agent) can see *why* each token
  was spent, not just receive an opaque blob.
- **Debugging.** When a plan retrieves the wrong context, the `plan` + `why` +
  `task_class` localizes the failure to classification (§2), valuation (§3), or a
  primitive — without re-running anything.
- **Replay.** The benchmark harness ([11](./11-benchmark-harness.md)) records plans
  and replays them against a pinned `generation`, so plan quality is measured
  deterministically (Task-Resolution-Accuracy / Token-Cost / Sufficiency-Overhead),
  and a regression in the planner is attributable to a specific stage.

---

## 5. Worked example

Task: **"Fix the off-by-one in pagination when page size is 1"**, `B = 6000`.

1. **Classify.** Keywords "Fix" + "off-by-one" → **Class A (bug localize-and-patch)**.
2. **Anchor (always afforded).**
   `search_symbols("paginat")` → top match
   `src/list/page.ts#paginate:function` (rank 0.71). **cost ≈ 180**, spent = 180.
3. **Enumerate downstream candidates** (metadata only):
   - `outline("src/list", tier=1)` → 6 signature items, each ~40–90 tok.
   - `neighborhood(paginate, depth=1, both)` → 9 nodes; high-rank caller
     `renderList` (rank 0.69), callee `clampPageSize` (rank 0.52), …
   - `impact(paginate, cochange=true)` → `page.test.ts` (lift 3.1, weight 0.7),
     `Pager:class` (static, weight 0.9).
   - `expand(paginate)` → full body, **cost 320**, very high stage prior (it *is*
     the patch site).
4. **Greedy value/cost fill** into the remaining 5820:
   | picked | why | cost | running |
   |---|---|---|---|
   | `expand(paginate)` | patch site, top stage prior | 320 | 500 |
   | `neighborhood(paginate, depth=1, both)` | callers/callees of the fix | 1430 | 1930 |
   | `impact(paginate)` → `Pager`, `page.test.ts` | what co-changes with paginate | 1100 | 3030 |
   | `outline("src/list", tier=1)` | sibling signatures for context | 1620 | 4650 |
   | `expand(clampPageSize)` | size-1 boundary likely lives here | 470 | 5120 |
5. **Truncation.** Next best candidate was `expand(renderList)` (cost 880). Only
   880 remains until `B`; the tier-1 signature of `renderList` was already in the
   neighborhood payload, so its `value/cost` had fallen below the `outline` siblings
   and it **lost the budget** — *dropped*, not truncated. No item needed mid-item
   truncation in this run.
6. **Result:** `spent_est = 5120`, `budget = 6000`, headroom 880 left unspent
   (see §9 on underspend). Plan + context returned in one call. The agent has the
   patch site body, its one-hop graph, its co-change set, and sibling signatures —
   the minimal set to fix a size-1 boundary bug.

---

## 6. Interaction with H4 (refresh stale context cheaply)

`plan_retrieval` accepts an optional **`since_generation`** — the generation the
agent last saw ([01](./01-architecture.md) §5). When the pinned snapshot's
`generation` has advanced past it, H5 **prepends a diff stage** before the strategy
runs:

```
diff_context(from=<since_generation snapshot>, to=WORKING, scope=<task scope>)
```

This is an H4 structural diff ([08](./08-h4-semantic-diff-hydration.md),
[03](./03-mcp-tool-surface.md) §2) of just the symbols that changed — `token_est`
is tiny (e.g. `"+param strict: boolean"`, 12 tokens, [02](./02-data-model.md) §2).
Prepending it lets the plan **refresh only what went stale** instead of re-hydrating
context the agent already holds: H5 then *skips* re-fetching symbols whose
`stable_key` is absent from the diff (the agent's prior copy is still valid) and
spends budget only on changed and newly-relevant symbols. The diff stage is
afforded like the anchor — it is cheap and it determines what the rest of the plan
can safely reuse.

---

## 7. Why II / US are low (6 / 6) — and why that is fine

H5 scores **II = 6, US = 6**, the lowest in the set. Both metrics
([README](./README.md)) measure resilience and resync speed of an *index*. H5 has
**no persistent index**: it is computed fresh on every `plan_retrieval` call, at
query time, never at edit time. There is nothing in H5 to keep in sync — so the
metrics that punish a stale or slow-to-resync index barely apply.

What the 6 actually reflects: H5's **value estimates read H2 ranks**
(`symbols.rank`, [02](./02-data-model.md) §2). During heavy resync those ranks
**lag slightly** — the Rank stage recomputes its affected region after Parse
([01](./01-architecture.md) §3), so for the staleness window a freshly-edited
symbol may carry a marginally stale centrality.

**Consequence — and why it is acceptable:**
- The planner reads a **pinned WAL snapshot** ([01](./01-architecture.md) §5), so a
  plan is always *internally consistent*; it is never torn.
- A slightly stale rank shifts *ordering at the margin* — a borderline candidate may
  win or lose budget it otherwise wouldn't. It does **not** corrupt results: every
  hydrated payload is real source at the pinned generation.
- Plans are **per-query and disposable**. The next call re-plans against the newest
  generation; there is no accumulated drift the way a stale persistent index would
  accrue. A mis-ranked plan self-heals on the following call.

So H5 trades a little ranking precision during the resync window for zero index
maintenance cost — the right trade for a query-time consumer.

---

## 8. v1 vs. later

**v1 (ships with the vertical slice, [12](./12-roadmap.md)):**
- Heuristic keyword classifier (§2), no model on the hot path.
- Greedy `value/cost` fill (§3) with **fixed, config-tuned `w_*` weights**.
- Composes **H1 / H2 / H3** only (`outline`, `expand`, `search_symbols`,
  `get_symbol`, `neighborhood`, `impact`). H4's `diff_context` is wired in as the
  §6 refresh stage as soon as H4 lands; `slice` (post-v1) plugs in as another
  candidate generator behind the same enumerate/hydrate contract.

**Later:**
- **Learned value model** replacing the hand-tuned `w_*`: regress expected
  task-resolution contribution on the per-item features (rank, weight, depth,
  match, stage prior).
- **Outcome feedback.** The benchmark harness emits a per-plan success signal
  ([11](./11-benchmark-harness.md)); feeding "did this plan resolve the task" back
  into classification and valuation closes the loop and tunes both the strategy
  templates and the weights from real outcomes.
- **LLM router** (§2) as an opt-in classifier/param-drafter — still strictly
  per-query, never in resync.

---

## 9. Open questions / risks

- **Mis-classification cost.** A Class-A bug routed as Class-C wastes budget on a
  blast-radius sweep and may starve the patch site. Mitigation: strategies overlap
  heavily at the anchor; the `expand`-the-anchor stage carries a high prior in
  *every* class, so the patch site is rarely starved. Open: measure
  mis-classification rate on the benchmark and whether a confidence threshold
  should fall back to the exploratory default.
- **Budget underspend.** A plan leaving large headroom (the §5 example left 880)
  may be under-retrieving. Should H5 *backfill* with the next-best candidates until
  `B` is nearly full, or is leaving headroom correct (fewer tokens is the whole
  point)? Open: a `fill_ratio` target vs. strict minimality — likely task-class
  dependent.
- **Budget overspend / indivisible items.** An item larger than remaining budget is
  either rank-truncated or skipped; a stage whose cheapest useful unit exceeds `B`
  yields nothing. Open: a minimum-viable-context floor that warns when `B` is too
  small for the task class rather than silently returning a thin plan.
- **Value-estimation calibration.** `value` mixes lexical match, centrality, impact
  weight, and depth on one scale; mis-calibrated `w_*` makes greedy pick the wrong
  items even with perfect classification. This is the single biggest lever on plan
  quality and the primary motivation for §8's learned model. Open: how to calibrate
  pre-benchmark, and whether `query_match` needs more than lexical overlap without
  reintroducing embeddings (rejected as core, [README](./README.md)).
- **Greedy vs. optimal.** Fractional-knapsack greedy is optimal only when items are
  independent; real items overlap (a neighborhood payload already contains a
  signature an outline would re-fetch). v1 de-dupes by `stable_key` before
  enumerating; open whether residual overlap justifies a more expensive solver.

---

### Cross-references
- Tool contract & shared budget protocol: [03-mcp-tool-surface.md](./03-mcp-tool-surface.md) §2, §4
- Rank/weight/token_est sources: [02-data-model.md](./02-data-model.md) §2, §5, §6
- Planner is a read-only consumer: [01-architecture.md](./01-architecture.md) §3, §5
- Primitives composed: [04](./04-h1-recursive-repo-map.md) · [05](./05-h2-symbol-rank-topology.md) · [06](./06-h3-impact-radius.md) · [08](./08-h4-semantic-diff-hydration.md)
- Replay & scoring: [11-benchmark-harness.md](./11-benchmark-harness.md) · build order: [12-roadmap.md](./12-roadmap.md)
