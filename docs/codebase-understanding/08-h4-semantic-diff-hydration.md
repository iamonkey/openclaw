# 08 — H4: Semantic Diff Hydration

> **Scores:** TRP 9 · AF 6 · IC 6 · II 7 · US 9 · Ceiling **[ER]**

The interactive-loop **refresh layer**. Once an agent has seen a region of code,
re-sending changed files to keep its context fresh is wasteful. H4 instead emits
an **AST-level structural diff** — "`foo`: +param `x`", "`bar` → `baz` renamed",
"moved to `src/util/x.ts`" — describing exactly what changed *semantically*, in a
handful of tokens. It backs the [`diff_context`](./03-mcp-tool-surface.md#diff_context--h4)
tool and reads/writes the `snapshots` and `ast_diffs` tables
([02-data-model.md](./02-data-model.md) §2). It runs as the **Diff stage** of the
indexer pipeline ([01-architecture.md](./01-architecture.md) §3, Stage 3).

H4 is a primitive: H5 ([09](./09-h5-adaptive-context-budgeter.md)) composes it,
and it leans on the same `stable_key` identity ([02](./02-data-model.md) §3) every
other hypothesis uses.

---

## 1. Goal & token economics (TRP 9)

In a long agent session the model has *already* read the code. After an edit or a
branch switch the naïve refresh is to re-paste the changed files. That re-pays for
context the model already holds, and buries the one fact that matters — what
*changed* — under hundreds of unchanged tokens.

A structural diff pays only for the delta, at **symbol granularity** and in
native-code phrasing the model already understands:

```
parse(src: string): Config
        ↓  edit adds a parameter
parse(src: string, strict: boolean): Config
```

| Refresh strategy | Payload | Token cost |
|---|---|---|
| Resend the whole file (`load.ts`, ~120 LOC) | full source | ~480 |
| Resend just the changed function body (~40 LOC) | full body | ~320 |
| **H4 structural diff** | `parse: signature +param strict: boolean` | **~12** |

A 3-line signature change is a ~12-token diff instead of a ~400-token resend —
a >30× reduction on the refresh, and it composes: a 10-symbol changeset is still a
couple hundred tokens, not several thousand. This is why **TRP is 9**.

AF is **6** (not higher) by design: a diff is local by nature. It tells the agent
*what moved*, not *why it matters system-wide* — that is H2/H3's job. H4's value
is freshness-per-token in the inner loop, not architectural reach.

---

## 2. The structural diff algorithm

H4 never diffs text lines. It diffs **trees of symbols**.

```
old snapshot ──▶ tree-sitter parse ──▶ symbol set Sₒ (keyed by stable_key)
new snapshot ──▶ tree-sitter parse ──▶ symbol set Sₙ (keyed by stable_key)
                                         │
                                         ▼
                         symbol-granular tree diff
                                         │
              ┌──────────┬──────────┬────┴─────┬──────────┬──────────┐
            added     removed    renamed   signature    body       moved
```

Both sides reuse the Parse stage's extraction (`carto-parse`), so the new side is
usually already in the store — the diff compares the freshly parsed symbols
against the prior snapshot's rows. Matching is by `stable_key`
(`<path>#<container>/<name>:<kind>`), then a second pass resolves the keys that
*didn't* line up — the high-value cases line-diff misses.

Each changed symbol is classified into exactly one `ast_diffs.change` value:

| `change` | Trigger | Notes |
|---|---|---|
| `added` | key in Sₙ, not in Sₒ | new symbol |
| `removed` | key in Sₒ, not in Sₙ | deleted symbol |
| `signature` | same key, signature subtree differs | params / return type / generics / modifiers |
| `body` | same key, only the body span hash differs | implementation churn |
| `renamed` | unmatched-removed ↔ unmatched-added, **same shape** | see below |
| `moved` | matched by shape but **different file/parent** | see below |

### Matching pass (the cheap 90%)

For every `stable_key` present on both sides:

```rust
match (old.sig_hash == new.sig_hash, old.body_hash == new.body_hash) {
    (true,  true ) => Unchanged,          // skip; emit nothing
    (false, _    ) => Signature,          // signature subtree changed
    (true,  false) => Body,               // body churned, signature intact
}
```

`sig_hash` is a hash of the *normalized signature subtree* (name, params, return
type, generics, modifiers — formatting and comments stripped). `body_hash` is a
hash of the body span's token stream, also formatting-normalized so a reflow alone
emits nothing (§9).

### Rename & move detection (the hard, high-value 10%)

After matching, two leftover sets remain: keys only-in-Sₒ (`removed?`) and keys
only-in-Sₙ (`added?`). H4 tries to pair them before emitting `added`/`removed`,
because a bare add+remove is what loses the agent's thread.

- **Rename** = same body/shape, different name. For each `removed?` symbol, look
  for an `added?` symbol with **matching `body_hash`** (or shape similarity ≥
  threshold for near-identical bodies) and the **same enclosing container**.
  Pair → `renamed`. This is exactly the case `stable_key` cannot represent (a
  rename *breaks* the key by design, [02](./02-data-model.md) §3).
- **Move** = same symbol, different file or parent. Pair a `removed?` with an
  `added?` that has a **matching `sig_hash` + `body_hash`** but a different
  `file_id` or `parent_id`. Pair → `moved`.

Both pass simultaneously: a symbol can be renamed *and* moved; H4 emits the
higher-significance label (`moved` carries the location, `detail` notes the rename
too — see §3).

### The rename-remap (what H3 and `stable_key` depend on)

A confirmed `renamed`/`moved` pairing is not just a diff row — it emits a
**key-remap** so downstream identity survives:

```
remap: "src/config/load.ts#validate:function"
    →  "src/config/load.ts#validateStrict:function"
```

Per [02](./02-data-model.md) §3, the Diff stage feeds this remap back so the
agent's held handles, the `symbols.stable_key`, and `cochange` history follow the
rename instead of resetting to zero support. Without H4's rename detection a
refactor would orphan all of H3's accumulated co-change signal for that symbol.
This is the one place H4 writes *back into* identity, not just into `ast_diffs`.

---

## 3. The `detail` rendering

`ast_diffs.detail` is a compact, **faithful** description of the change — readable
by both the model and a human auditor. Per the project's *auditable fidelity*
constraint ([README](./README.md) §"Hard operating constraints"), it uses native
code fragments and a tiny fixed vocabulary — **no lossy invented shorthand**.

| `change` | `detail` examples |
|---|---|
| `signature` | `+param strict: boolean` · `-param legacy` · `return type Config → Result<Config>` · `+async` · `<T> → <T: Clone>` |
| `renamed` | `validate → validateStrict` |
| `moved` | `moved to src/util/parse.ts` · `moved into class ConfigLoader` |
| `added` | `+ parse(src: string, strict: boolean): Config` (full signature) |
| `removed` | `- legacyParse(src: string): Config` |
| `body` | `body changed (12 lines)` · `body changed (+3 −1 stmt)` |

Rendering rules:

- **Signature diffs are itemized**, one fragment per atomic change, joined by `, `
  when several occur on one symbol: `+param strict: boolean, return type T → Result<T>`.
- **Type changes** show old → new verbatim from the source, never a paraphrase.
- **Added/removed** symbols carry their rendered signature (Tier-1 text), so the
  agent learns the new shape without a follow-up `expand`.
- **`body`** changes are deliberately coarse — the agent gets a flag and a size,
  not the body. If it wants the new body it issues one `expand`
  ([03](./03-mcp-tool-surface.md#expand--h1)). Bodies are the cheap-to-refetch,
  low-information-per-token case; we don't pay to inline them.

Everything in `detail` is derived from real parsed source — auditable, and round-
trip-checkable against `expand`.

---

## 4. Triggers & snapshots

H4 diffs **snapshots**. A `snapshots` row is `(id, rev, taken_at_generation)`,
where `rev` is a git rev *or* the literal `"WORKING"` for the dirty working tree
([02](./02-data-model.md) §2).

Three things trigger a diff:

1. **Working-tree edit** — `WORKING` vs the last committed snapshot. The fs watcher
   debounces (~50ms), the Parse stage reparses dirty files, then the Diff stage
   (Stage 3, [01](./01-architecture.md) §3–§4) compares the new parse against the
   prior snapshot and writes `ast_diffs`. Cheap because only touched files reparse
   (§5).
2. **Branch switch / HEAD move** — `rev A` vs `rev B`. The git-HEAD watcher fires;
   the changeset is the tree delta between the two revs. H4 diffs only the symbols
   in files that differ between A and B.
3. **Explicit `diff_context(from, to, scope)`** — an agent (or H5) asks for the
   diff between any two arbitrary revs, optionally narrowed by `scope` (path
   prefix). Revs may be commit-ish (`HEAD~1`, a branch, a SHA) or `WORKING`.

### How and when snapshots are taken

- A **baseline** snapshot is recorded at cold start (`carto index`), `rev = HEAD`.
- On every committed delta the **generation counter** bumps
  ([01](./01-architecture.md) §5). The Diff stage records the `WORKING` snapshot's
  `taken_at_generation` so a diff is always anchored to a concrete index state and
  staleness is detectable.
- On a HEAD move, a snapshot is recorded for the new `rev` after the resync
  commits.
- `WORKING` is **mutable**: it is re-derived each delta and always means "current
  dirty tree". Diffs against it are recomputed, not cached across generations —
  only committed-rev↔committed-rev diffs are durable cache rows.

```
snapshots
  id  rev          taken_at_generation
  1   <sha @HEAD>   100          ← baseline (cold start)
  2   WORKING       137          ← current dirty tree, re-derived each delta
  3   <sha feat/x>  140          ← recorded after branch switch resync
```

`ast_diffs` rows reference `from_snap`/`to_snap`, so the same diff is reusable and
budgetable without recomputation when both ends are committed revs.

---

## 5. Incremental behavior (US 9)

Diff is **naturally incremental** — and that is why **US is 9**. The unit of work
is "what files changed", which is exactly the `Delta` the sync loop already
carries ([01](./01-architecture.md) §4):

- Only **touched files reparse** (tree-sitter incremental reparse,
  [10-incremental-sync.md](./10-incremental-sync.md)). A 3-line edit reparses one
  file and diffs the handful of symbols in it; nothing else is read.
- The matching pass is a `stable_key` set difference scoped to the touched files —
  no whole-repo comparison.
- A working-tree diff is therefore O(changed symbols), not O(repo).

### Why II is 7 (not higher)

H4 keeps integrity under fast mutation well, but three things keep **II at 7**, not
8–9:

1. **Diffs accrete.** `ast_diffs` is append-only ([02](./02-data-model.md) §4);
   long sessions accumulate rows that must be pruned by retention and coalesced
   (§7) or they distort budgeting.
2. **Snapshot management.** `WORKING` is mutable and re-derived; getting its
   anchoring wrong (diffing against a stale baseline after several deltas) yields a
   confusing diff. The generation anchor (§4) is the guard, but it is a moving part.
3. **Rename/move heuristics can misfire.** Shape-similarity matching is a
   heuristic; a wrong pairing produces a bogus `renamed` *and* a bad key-remap that
   propagates into H3 (§9). The conservative thresholds that prevent this are the
   reason II isn't higher.

---

## 6. Integration with the agent loop

H4 is the cheap "what changed since you last looked" call. Typical uses:

- **After the agent makes an edit.** It applied a change; it issues
  `diff_context(from="<HEAD>", to="WORKING", scope="<edited path>")` to confirm the
  structural effect of its own edit (did the signature change land? did it
  accidentally rename something?) for ~12 tokens instead of re-reading the file.
- **After the user switches branches.** Mid-task the user checks out a different
  branch. Rather than re-outlining everything, the agent calls
  `diff_context(from="<old rev>", to="<new rev>")` and gets the symbol-level delta —
  staying oriented for a couple hundred tokens.

### Auto-injection by H5

The **generation counter** ([01](./01-architecture.md) §5) lets H5
([09](./09-h5-adaptive-context-budgeter.md)) detect that the index advanced *mid-
session*. When it does, H5 may **auto-prepend a `diff_context` summary** to the top
of a freshly planned retrieval — a one-line-per-changed-symbol header — so the plan
is built against, and the agent reads, the *current* code rather than the snapshot
it last hydrated. The diff is the cheapest possible "your mental model is N
generations stale; here's the delta" preamble. This is H4 acting as H5's freshness
layer, not a separate agent action.

---

## 7. Budget

H4 obeys the shared token-budget protocol ([03](./03-mcp-tool-surface.md) §4):

- Every `ast_diffs` row carries a cached **`token_est`** ([02](./02-data-model.md)
  §2), computed once at write time (bytes/4 or the small BPE estimator,
  [02](./02-data-model.md) §6) — the planner never re-tokenizes during budgeting.
- When a diff exceeds `max_tokens`, truncate **by change significance**, not by
  arrival order:

  ```
  signature ≈ renamed ≈ moved  >  added ≈ removed  >  body
  ```

  Signature/rename/move changes are the high-information, hard-to-recover-by-
  refetch cases — they survive truncation. `body` churn is dropped first (the agent
  can always `expand` to recover it). When truncated, report `truncated: true` and
  `dropped: <count>` so nothing vanishes silently.
- Within a significance tier, ties break by symbol **rank** (H2 centrality), so a
  signature change to a hub symbol outranks one to a leaf.

---

## 8. Worked example

The user switches from `main` to `feat/strict-config`. On that branch, `validate`
was renamed to `validateStrict` and gained a `strict` parameter. The naïve refresh
resends both file versions (~900 tokens). H4 sends this:

**Request**

```jsonc
{ "from": "main", "to": "feat/strict-config", "scope": "src/config" }
```

**Response** (`diff_context`, [03](./03-mcp-tool-surface.md#diff_context--h4))

```jsonc
{
  "generation": 412,
  "changes": [
    { "symbol": "src/config/load.ts#validateStrict:function",
      "change": "renamed",
      "detail": "validate → validateStrict",
      "token_est": 8 },
    { "symbol": "src/config/load.ts#validateStrict:function",
      "change": "signature",
      "detail": "+param strict: boolean",
      "token_est": 11 }
  ]
}
```

Total payload: **~20 tokens** vs ~900 for two full files — and the agent's held
handle (`…#validate:function`) is remapped to `…#validateStrict:function` (§2), so
its prior knowledge of that symbol's callers (H2) and co-change history (H3)
follows the rename instead of resetting.

Behind it, the Diff stage:

```sql
-- the two rows written for this branch-switch diff
INSERT INTO ast_diffs (from_snap, to_snap, symbol_key, change, detail, token_est)
VALUES
  (:main_snap, :feat_snap, 'src/config/load.ts#validateStrict:function',
   'renamed',   'validate → validateStrict',  8),
  (:main_snap, :feat_snap, 'src/config/load.ts#validateStrict:function',
   'signature', '+param strict: boolean',     11);
```

```sql
-- read back for diff_context(main, feat/strict-config, scope='src/config')
SELECT symbol_key, change, detail, token_est
FROM ast_diffs
WHERE from_snap = :main_snap AND to_snap = :feat_snap
  AND symbol_key LIKE 'src/config/%'
ORDER BY CASE change
           WHEN 'signature' THEN 0 WHEN 'renamed' THEN 0 WHEN 'moved' THEN 0
           WHEN 'added' THEN 1 WHEN 'removed' THEN 1
           ELSE 2                       -- body last
         END;
```

---

## 9. Open questions & risks

- **Rename false positives.** Shape-similarity matching can pair two distinct
  symbols that happen to share a body (boilerplate getters, stubbed `todo()`
  bodies). A wrong pairing emits a bogus `renamed` *and* a bad key-remap that
  corrupts H3 co-change. *Mitigation:* require same enclosing container + exact
  `body_hash` for cheap renames; gate similarity-based renames behind a high
  threshold and only when the add/remove counts in the file are balanced; never
  remap across unrelated kinds.
- **Large mechanical refactors.** A repo-wide rename or formatter run can produce
  thousands of `ast_diffs` rows and an unusable diff payload. *Mitigation:* **cap +
  summarize** — above a per-call change cap, collapse to a summary
  (`"142 symbols moved src/old/ → src/new/ (bulk rename)"`) plus the top-N by
  significance/rank, with `truncated`/`dropped` reported.
- **Formatting-only changes suppressed.** Reflows, import sorting, whitespace, and
  comment edits must emit **nothing**. This is enforced by hashing the
  *normalized* signature/body subtrees (§2), but normalization coverage is
  per-grammar and incomplete grammars may leak formatting noise. *Open:* maintain
  per-language normalization queries alongside the tags queries in `grammars/`.
- **`WORKING` staleness window.** A diff against `WORKING` is only as fresh as the
  last committed delta; the generation anchor (§4) makes staleness *detectable* but
  not zero. The window is measured and gated in
  [11-benchmark-harness.md](./11-benchmark-harness.md).
- **Cross-language refactors.** Moving a symbol between files in *different*
  languages (rare, e.g. a config ported TS→Rust) can't be shape-matched; these fall
  back to plain `added`/`removed`. Acceptable for v1.

---

## Cross-references

- Snapshots & `ast_diffs` schema, `stable_key`, generation: [02-data-model.md](./02-data-model.md) §2–§4
- `diff_context` contract & budget protocol: [03-mcp-tool-surface.md](./03-mcp-tool-surface.md#diff_context--h4)
- Diff stage ordering & data flow: [01-architecture.md](./01-architecture.md) §3–§5
- Incremental reparse / branch-switch resync: [10-incremental-sync.md](./10-incremental-sync.md)
- Auto-injection into plans: [09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md)
- Rename-remap consumer (co-change identity): [06-h3-impact-radius.md](./06-h3-impact-radius.md)
