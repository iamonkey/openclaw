# 06 — H3: Impact-Radius Mapping

> **Scores** — TRP 7 · AF 9 · IC 6 · II 8 · US 8 · Ceiling **[ER]**

H3 answers one question an editing agent keeps asking: *"if I touch X, what else
breaks, needs updating, or must be re-read?"* It fuses H2's static call graph
([05-h2-symbol-rank-topology.md](./05-h2-symbol-rank-topology.md)) with a **git
co-change** signal mined from history, and serves both through the `impact`
tool ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md) §2).

Unlike H1/H2, which serve the *retrieval/understand* phase, H3 serves the
**MODIFY** phase. It is the only hypothesis whose primary consumer is an agent
about to write code, not just read it.

---

## 1. Why static-only impact misses things

H2's `edges` table is precise but *complete only for relationships the parser can
see in source*. An edit's true blast radius routinely escapes the AST:

- **Config / serialization coupling.** A function reads a key from
  `config.toml`; the loader that produces that key lives in another file with no
  call edge to the reader. Rename the key, both must change — no edge connects
  them.
- **Test coupling.** `parse()` and its `parse.test.ts` change together on every
  behavioral edit, but the test imports a fixture, not the symbol under test in a
  way that produces a clean `call` edge — and even when it does, the *fixture
  file* has no edge at all.
- **Protocol / wire coupling.** A serializer and a deserializer in different
  languages (a Rust struct and its TypeScript decoder) must stay in lock-step.
  No cross-language static edge exists.
- **Convention coupling.** A registry, a switch over an enum, and the enum's
  variants: adding a variant means three edits, only one of which is a real
  reference.

Git history *already recorded* every one of these as "these things changed in the
same commit, repeatedly." Static analysis says what *can* reach what; co-change
says what *did* change with what. The two are orthogonal and both true.

**"Blast radius"** for an editing agent = the ranked set of symbols and files
that, empirically or structurally, must be inspected or edited when X changes.
H3 returns that set small-first-best so the agent reads the three things that
matter, not the thirty things one hop away.

---

## 2. Co-change mining (the History stage)

Owned by `carto-git` (Stage `History = 2`, [01-architecture.md](./01-architecture.md)
§3). On cold start it walks `git log` once and writes `cochange` /
`file_cochange` ([02-data-model.md](./02-data-model.md) §2).

### 2.1 Walk

```rust
// crates/carto-git/src/cochange.rs (sketch)
let mut walk = repo.revwalk()?;
walk.push_head()?;
walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

for (i, oid) in walk.enumerate() {
    if i >= cfg.cochange_window_commits { break; }   // [history].cochange_window_commits
    let commit = repo.find_commit(oid?)?;
    if commit.parent_count() > 1 { continue; }        // skip merges (see §3)
    let touched: Vec<SymbolId> = attribute(&repo, &commit)?;  // §3 attribution
    if touched.len() > cfg.cochange_commit_cap { record_file_level(&commit); continue; } // §3 cap
    accumulate_pairs(&mut counts, &touched, weight_for(touched.len()));
}
```

For each commit we compute the **diff against its single parent**, attribute each
changed line range to a *current* symbol (§3), then for the resulting symbol set
`S` we increment counts for every unordered pair `(a, b) ∈ S × S, a ≠ b` and the
per-symbol occurrence count.

### 2.2 Metrics — support, confidence, lift

Let `N` = number of commits in the mined window. For symbols `a`, `b`:

- `occ(a)` = commits (in window) that touched `a`.
- `co(a,b)` = commits that touched **both** `a` and `b`.

Then, matching the `cochange` columns exactly:

```
support(a,b)    = co(a,b)                          -- raw # commits touching both
confidence(a→b) = co(a,b) / occ(a)                 -- P(touch b | touched a)
base_rate(b)    = occ(b) / N                        -- unconditional P(touch b)
lift(a,b)       = confidence(a→b) / base_rate(b)
              = (co(a,b) * N) / (occ(a) * occ(b))   -- symmetric form
```

Interpretation:
- **support** — how much evidence backs the pair. Low support = noise.
- **confidence** — directional: of the times you touched `a`, what fraction also
  touched `b`. `confidence(a→b) ≠ confidence(b→a)`; we store the row keyed on
  `a_id` so `idx_cochange_a` powers "given I'm editing `a`, rank candidates."
- **lift** — corrects confidence for `b`'s base churn. A file that changes in
  *every* commit (a changelog, a lockfile) has high confidence with everything
  but **lift ≈ 1** — it is not specifically coupled to `a`. `lift > 1` means the
  pair co-changes *more than chance*; that is the real signal. `lift` is
  symmetric, which is why it is the default sort and the value surfaced in the
  tool's `reason` string.

Both directional rows are written: `(a→b)` and `(b→a)` share `support` and `lift`
but carry their own `confidence`.

### 2.3 Pruning

- **`cochange_min_support`** (default 3) — drop any pair with `support <
  min_support` before writing. Two symbols that changed together once are
  coincidence, not coupling. This is the dominant row-count reducer (pair counts
  are quadratic in commit size before pruning).
- **`cochange_window_commits`** (default 5000) — only mine the most recent `N`
  commits. Bounds cold-start cost and biases toward *current* coupling; ancient
  history reflects a structure that may no longer exist.

### 2.4 File-level fallback (`file_cochange`)

Symbol attribution fails for: unparsed files (Tier-0 only — config, markdown,
JSON fixtures), files whose pre-history symbols no longer map to today's tree,
and renames we could not bridge (§3). For these we accumulate the **same three
metrics at file granularity** into `file_cochange`, keyed on `files.id`.

`file_cochange` is always populated (it is strictly cheaper than symbol
attribution) and is the substrate that recovers the *config* and *fixture*
coupling cases from §1, where one side has no symbols at all. The fusion query
(§4) reads symbol-level rows first and falls back to file-level for any file
whose symbols produced no `cochange` hits.

---

## 3. Attribution — the hard parts

Mapping a **historical diff** onto **today's symbols** is where H3 earns its IC 6.

### 3.1 Line ranges → current symbols

A commit's diff gives changed line ranges *in that commit's version of the file*.
Our `symbols` spans (`start_row`/`end_row`, [02-data-model.md](./02-data-model.md)
§2) describe **today's** file. We do not re-parse every historical revision (too
slow). Instead:

1. Resolve the changed *file* to today's `files.id` (following renames, §3.3).
2. Use H4's per-revision line remap ([08-h4-semantic-diff-hydration.md](./08-h4-semantic-diff-hydration.md))
   to project the historical changed rows forward to current rows when the file
   still exists. This is the same line-tracking H4 maintains for diff context;
   H3 reuses it rather than duplicating blame.
3. Intersect projected rows with current symbol spans → the symbol set `S`.
4. Any changed range that projects to *no* current symbol (deleted code, moved
   out, top-of-file imports) contributes only to `file_cochange`, not `cochange`.

When the remap is unavailable or too lossy (large rewrites between then and now),
we **degrade that commit to file-level attribution** rather than guess a symbol.
Co-change is a statistical signal; one commit dropping to file granularity does
not corrupt it.

### 3.2 `stable_key` is the accumulator key

We accumulate against **`symbols.stable_key`**, not `symbols.id`, throughout the
walk, and resolve to `id` only at write time. This is load-bearing:
`stable_key` survives body edits and line shifts ([02-data-model.md](./02-data-model.md)
§3), so two commits that touched "the same logical function" accumulate to the
same bucket even though every intervening edit shifted its bytes.

### 3.3 Renames and moves

A rename breaks `stable_key` by design. H4 emits a **key-remap row** on
`change='renamed'` ([02-data-model.md](./02-data-model.md) §3, §3 there). The
History stage consults this remap so that pre-rename occurrences fold into the
post-rename key — co-change history *follows the rename* instead of resetting to
zero support. The same applies to file renames via libgit2's rename detection
(`DiffFindOptions::renames`), which we enable on the diff before attribution.

### 3.4 Commits that touch huge swaths

A formatting sweep, a license-header bump, or a mass `cargo fmt` touches hundreds
of symbols and would, unpruned, create `O(n²)` spurious pairs that all look
mutually coupled. Two guards:

- **Cap** (`cochange_commit_cap`, default ~50 touched symbols): commits over the
  cap skip pair accumulation entirely and contribute only to `file_cochange`
  (and even there, weighted down).
- **Weight by commit size**: pair increments are weighted `w = 1 /
  log2(|S| + 2)` rather than `+1`. A 3-symbol commit is strong evidence of
  coupling between those 3; a 40-symbol commit is weak per-pair evidence. The
  stored `support` is the rounded sum of weights (so a single tight commit can
  still cross `min_support`, a broad one cannot on its own).

### 3.5 Merge commits

Skipped (`parent_count() > 1`). A merge's diff against *one* parent is the other
branch's entire delta — not a coherent unit of intent. The constituent commits
are walked individually on a topological walk, so their co-change is captured
without double-counting.

### 3.6 Vendored / excluded dirs

Paths matching `[index].exclude` ([01-architecture.md](./01-architecture.md) §6)
and `.gitignore` are dropped from attribution before pair accumulation. A
`vendor/` bump that rides along with a real edit must not manufacture coupling to
third-party code the agent will never edit.

---

## 4. The fusion model — what `impact(X)` actually does

`impact` ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md) §2) returns one
ranked `items` list unioning **two sources** — `source: "static"` and
`source: "cochange"` — each item carrying `key`, `source`, `reason`, `weight`.

### 4.1 Static side

Reuse H2's bounded neighborhood (the `neighborhood` CTE,
[02-data-model.md](./02-data-model.md) §5) out to `static_depth` (param, default
2), over impact-relevant edge kinds (`call`, `typeref`, `import`, `implement`,
`inherit`). Static weight decays with hop distance:

```
weight_static(n) = rank(n) * STATIC_DECAY^(depth(n) - 1)     -- STATIC_DECAY ≈ 0.6
```

Closer neighbors of higher centrality score higher. `reason` = `"<kind> depth
<d>"`, e.g. `"typeref depth 1"`.

### 4.2 Co-change side

Read `cochange` for `X`, filtered by `min_lift` (param, default 1.5) so churny
non-specific pairs (lift ≈ 1) are excluded:

```sql
-- co-change candidates for symbol :x
SELECT b_id AS id, support, confidence, lift, 'cochange' AS source
FROM cochange
WHERE a_id = :x AND lift >= :min_lift
ORDER BY lift DESC;
```

Then, for any *file* of `X` whose symbols yielded no rows, fall back:

```sql
SELECT b_file, support, confidence, lift, 'file_cochange' AS source
FROM file_cochange
WHERE a_file = (SELECT file_id FROM symbols WHERE id = :x)
  AND lift >= :min_lift
ORDER BY lift DESC;
```

Co-change weight blends evidence (support) and specificity (lift), squashed to
`[0,1)`:

```
weight_cochange = sigmoid( log(support) * LIFT_GAIN * log(lift) )   -- LIFT_GAIN ≈ 0.5
```

High lift with thin support, or high support with lift ≈ 1, both stay modest;
the product rewards pairs that are *both* well-evidenced and specifically
coupled. `reason` = `"changed together <support>× (lift <lift>)"`, e.g.
`"changed together 14× (lift 3.2)"`.

### 4.3 Union, dedup, rank

```sql
WITH static_side AS ( /* neighborhood CTE → id, depth, kind */ ),
     cc_side     AS ( /* cochange query above → id, support, lift */ )
SELECT
  COALESCE(s.id, c.id)                       AS id,
  CASE WHEN c.id IS NOT NULL THEN 'cochange' ELSE 'static' END AS source,
  -- a symbol present in BOTH sources is the strongest signal:
  CASE
    WHEN s.id IS NOT NULL AND c.id IS NOT NULL
      THEN w_static(s) + w_cochange(c) + BOTH_BONUS      -- BOTH_BONUS ≈ 0.15
    WHEN c.id IS NOT NULL THEN w_cochange(c)
    ELSE w_static(s)
  END                                        AS weight
FROM static_side s
FULL OUTER JOIN cc_side c ON s.id = c.id
ORDER BY weight DESC;
```

SQLite lacks `FULL OUTER JOIN` pre-3.39; in practice we materialize both sides
into a temp table and `UNION`/group in Rust (`carto-git` + `carto-core`),
keeping the per-`id` max-merge and `BOTH_BONUS`. The semantics above are the
contract.

**Dedup rule:** a symbol reached *both* statically and via co-change is the
highest-confidence impact item (the graph and history agree) and gets
`BOTH_BONUS`; its `source` is reported as `"cochange"` with a fused `reason`
(e.g. `"typeref depth 1; changed together 9× (lift 2.4)"`) so the agent sees both
justifications.

### 4.4 Budget truncation

Per the shared budget protocol ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md)
§4): order `items` by `weight` descending, fill until the next item would exceed
`max_tokens`, set `truncated: true`, report `dropped: <count>`. H3's ordering key
is `weight` (not raw H2 rank), because for the MODIFY phase a high-lift test file
is more useful than a high-centrality node three static hops away.

---

## 5. Incremental behavior (US 8)

`cochange` / `file_cochange` are **append-only accruing** tables
([02-data-model.md](./02-data-model.md) §4). The History producer's `apply()`
([01-architecture.md](./01-architecture.md) §3) handles two delta kinds:

- **New commit(s) on HEAD.** Walk only the commits between the last-mined HEAD
  and the new HEAD (commit-addressed; see [10-incremental-sync.md](./10-incremental-sync.md)).
  Attribute each, then **increment** support/occurrence counters and recompute
  confidence/lift for the touched pairs only. No full re-walk. This is why US is
  8: a single new commit updates a handful of rows.
- **Pruning** happens lazily against `cochange_window_commits` /
  `cochange_min_support` on idle, not on the hot path.

### 5.1 Branch switch is co-change-stable

Co-change is **commit-addressed**: every contribution is keyed on the commit OID
it came from, and we record the set of mined OIDs in `meta`. Switching branches
therefore does *not* invalidate the signal — the shared history is identical.
Only **branch-unique commits** differ:

- Switching to a branch whose commits are a superset of HEAD's history → mine the
  extra commits forward (cheap `apply`).
- Switching to a branch missing some commits HEAD had → those commits' OIDs are
  simply absent from this branch's reachable set; their contributions remain in
  the table but reference unreachable history. We treat them as *retained but
  decaying* (they fall out naturally as the window slides), because branch
  switches are frequent and transient and re-mining on every toggle would violate
  the interactive-resync constraint. The `meta` mined-OID set lets a full rebuild
  reconcile exactly when the user asks.

Net: branch switching never triggers a co-change re-walk of shared history; the
graph (H2) and diffs (H4) bear the branch-switch resync cost, H3 rides along.

---

## 6. Degradation

H3 is the one hypothesis with an external dependency (a usable git history).
Degradation is explicit and already declared in
[01-architecture.md](./01-architecture.md) §7:

| Condition | Behavior |
|---|---|
| No git repo | `impact` returns `cochange: "unavailable"`; `items` are **static-only** (§4.1). |
| Shallow clone (`--depth`) | Mine whatever commits are present; `cochange: "partial"` when the available depth `< cochange_window_commits`. |
| Squash-merge-only repo | Mine as available; coupling granularity is coarse (§9). Still `available`. |
| History mid-build | `impact` returns static items immediately with `cochange: "building"`; co-change items stream in as the History stage completes. |

In every degraded case the **static side always works** — H3 never errors for
lack of history, it narrows. The `cochange` field on the response tells the agent
exactly how much to trust the empirical side.

---

## 7. Worked example

Agent is about to edit `src/config/load.ts#ConfigLoader/parse:method`.

```jsonc
// request
{ "symbol": "src/config/load.ts#ConfigLoader/parse:method",
  "static_depth": 2, "include_cochange": true, "min_lift": 1.5,
  "max_tokens": 2000 }

// response
{
  "generation": 412,
  "cochange": "available",
  "items": [
    { "key": "src/config/schema.ts#Schema:class",
      "source": "static",
      "reason": "typeref depth 1",
      "weight": 0.90 },
    { "key": "src/config/load.test.ts#tests:module",
      "source": "cochange",
      "reason": "changed together 14× (lift 3.2)",
      "weight": 0.71 },
    { "key": "src/config/defaults.toml",
      "source": "file_cochange",
      "reason": "file changed together 9× (lift 2.1)",
      "weight": 0.58 }
  ],
  "truncated": false
}
```

Reasoning the agent can trust:
- `Schema:class` — a **static** `typeref`: `parse` returns `Config`, which the
  schema defines. The compiler will tell you if you break it; read it first.
- `load.test.ts#tests:module` — **co-change**, lift 3.2: every behavioral change
  to `parse` historically updated this test. Not a static neighbor (the test
  imports a fixture), but empirically inseparable. Update it.
- `defaults.toml` — **file-level co-change**: an unparsed config file with no
  symbols, recovered only because it kept changing alongside `parse`. The exact
  case static analysis cannot see (§1).

---

## 8. Relationship to Behavioral Slice (doc 07)

H3 and the Behavioral Slice ([07-behavioral-slice.md](./07-behavioral-slice.md),
post-v1) both answer "what is affected by X," but with **different evidence**:

| | H3 — Impact Radius | Behavioral Slice |
|---|---|---|
| Evidence | **Empirical / historical** (git co-change) | **Static / precise** (dependency analysis over H2's graph) |
| Catches | Coupling with *no code edge* (config, tests, wire protocols, conventions) | Exact statements that data/control-flow-affect a criterion |
| Misses | Real dependencies that never happened to co-change (new or rarely-edited code) | Anything not expressible as a source edge (the §1 cases) |
| Confidence | Statistical ("usually changes together") | Provable ("definitely reads this value") |

They are **complementary**, not competing:

- **H3 wins** on mature codebases with rich history, for cross-cutting coupling
  the parser cannot see, and during MODIFY ("what else will I have to touch?").
- **Slice wins** for precise, provable reasoning about a single execution path,
  on young repos with no history, and when the agent needs *why* (the data-flow
  chain) rather than *correlation*.

The strongest signal is their intersection — a symbol that both co-changes *and*
slices into the target — which is why the v1 fusion already rewards H2-static ∩
H3-cochange agreement (`BOTH_BONUS`, §4.3); Slice will later extend that union as
a third `source`.

---

## 9. Open questions / risks

- **History rewrites.** Force-pushes, `rebase`, `filter-repo` change OIDs.
  Mined-OID provenance in `meta` becomes stale; the conservative fix is a History
  full rebuild when HEAD's history no longer contains the recorded mined OIDs.
  Detecting this cheaply (vs. a benign fast-forward) is open — likely a
  merge-base check on each HEAD move.
- **Squash-merge repos.** When every PR lands as one squashed commit, *all*
  symbols a PR touched collapse into a single commit → co-change degrades to
  "everything in a feature couples to everything." The commit-size cap and
  log-weighting (§3.4) blunt this but cannot fully recover intra-PR granularity.
  Mining PR metadata (the original branch commits via the forge API) is a
  possible but out-of-scope enhancement.
- **Young / thin-history repos.** Below a few hundred commits, support is too
  sparse for stable lift; pairs flicker above/below `min_support`. Mitigation:
  surface `cochange: "partial"` and lean on the static side; consider an
  adaptive `min_support` that scales with window size rather than the fixed
  default.
- **Author-churn skew.** A single contributor's habit (always touching a helper)
  can inflate confidence without representing real coupling. Weighting by
  distinct authors per pair is a candidate refinement, deferred.
- **Privacy / determinism.** The index embeds historical co-change structure;
  `.carto/` is `.gitignore`-d ([01-architecture.md](./01-architecture.md) §1) so
  it never ships, but a shared index would leak who-touches-what patterns.
  Out of scope for v1, flagged for any future shared-cache feature.

---

*See also:* [05-h2-symbol-rank-topology.md](./05-h2-symbol-rank-topology.md)
(static edges feeding the fusion) · [08-h4-semantic-diff-hydration.md](./08-h4-semantic-diff-hydration.md)
(rename remap + line projection H3 reuses) · [10-incremental-sync.md](./10-incremental-sync.md)
(append-only `apply` and branch-switch resync) · [09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md)
(consumes `impact` weights under budget).
