# 04 — H1: Recursive Repo Map

Tiered skeleton hydration. The agent descends a 100k+ LOC repo top-down — file
tree + one-line purpose, then tree-sitter signatures, then full bodies — pulling
only the tier it needs at each step instead of reading whole files.

| TRP | AF | IC | II | US | Ceiling |
|---|---|---|---|---|---|
| 9 | 7 | 5 | 8 | 9 | [ER] |

H1 is the **lowest-IC `[ER]` primitive** and the validation vehicle for the
lazy-hydration loop: it needs only the Parse stage
([01-architecture.md](./01-architecture.md) §3) and the `files` / `symbols` /
`skeletons` tables ([02-data-model.md](./02-data-model.md)), and it exercises the
full `outline → expand` descent that every other hypothesis composes against. If
H1 cuts tokens at the projected rate, the core thesis holds.

---

## 1. Goal & token economics

**Goal:** answer "what is in this codebase and where" without ever paying for a
full-file read until the agent has narrowed to one symbol.

A naive agent `cat`s files. Token math at ~100k LOC:

- ~100k LOC × ~9 tokens/line ≈ **900k tokens** to read the whole repo — multiples
  of any context window. Even reading the ~30 files a task plausibly touches is
  tens of thousands of tokens, most of it bodies the agent never reasons about.

Tiered hydration prices each layer separately:

| Tier | Unit | Approx tokens (100k LOC repo) |
|---|---|---|
| **0** — tree + purpose | 1 line / file, ~1500 files | ~12k tokens for the *entire* repo map; a single dir slice is ~200–600 |
| **1** — signatures + docstrings | ~10k–40k symbols, sig+doc only | ~15–40 tokens/symbol; a dir's worth is ~800–2500 |
| **2** — full body | one symbol via `expand` | ~100–400 tokens, paid once, only when chosen |

A realistic descent (root Tier-0 → one dir Tier-1 → two `expand`s) lands a
correct answer in **~1.5k–3k tokens** versus ~30k–60k for "read the candidate
files." That ~10–20× reduction at no fidelity loss (every token is real source or
a faithful structural derivation) is the TRP-9 / `[ER]` claim. The ceiling is
exponential because cost scales with *answer specificity*, not repo size: a
bigger repo grows Tier-0 linearly but the descent path stays roughly constant.

---

## 2. The three tiers

All three are produced by the **Parse stage** and read back by the `outline` /
`expand` tools ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md)). Nothing here
introduces a new table or tool — H1 *is* the read model of `files`, `symbols`,
and `skeletons`.

### Tier-0 — file tree + one-line purpose

- **Contains:** repo-relative path, `lang`, `size_bytes`, and a single-line
  `purpose` string per file. No symbols.
- **Produced by:** the purpose heuristics in §3, run once per file during parse.
- **Stored in:** `files.purpose` (the canonical column;
  [02-data-model.md](./02-data-model.md) §2). Directory structure is derived from
  `files.path` at query time, not stored as a separate tree.
- **Served by:** `outline(path, tier=0)`.

### Tier-1 — tree-sitter skeletons (signatures, no bodies)

- **Contains:** every symbol's `signature`, `doc`, `kind`, and `rank`, nested by
  `parent_id`. Bodies are excluded by construction.
- **Produced by:** tree-sitter tags queries (§4) during parse; one row per symbol
  in `symbols`, plus a pre-rendered `skeletons` row at `tier=1`.
- **Stored in:** `symbols.signature` / `symbols.doc` (structured) **and**
  `skeletons(symbol_id, tier=1, text, token_est)` (pre-rendered text so `outline`
  is a cheap read, not a re-render — [02-data-model.md](./02-data-model.md) §2).
  A `skeletons` row at `tier=0` holds the per-symbol purpose line where useful.
- **Served by:** `outline(path, tier=1)`.

### Tier-2 — full bodies on demand

- **Contains:** the verbatim source span of one symbol.
- **Produced by:** *nothing at index time.* Tier-2 is **read live from disk** via
  `symbols.start_byte..end_byte` against the current file. It is deliberately not
  stored ([02-data-model.md](./02-data-model.md) §2) — storing bodies would
  duplicate the repo and re-bloat the index on every edit.
- **Served by:** `expand(symbol)`.

```
outline(tier=0) ──narrow dir/file──▶ outline(tier=1) ──pick symbol──▶ expand(symbol)
   files.purpose                        skeletons(tier=1)               live disk read
```

---

## 3. Tier-0 purpose derivation (no LLM in the core path)

A faithful one-line purpose is produced by **deterministic heuristics** during
parse — no model call on the sync path, because §1 of
[01-architecture.md](./01-architecture.md) forbids anything that can't recompute
at interactive speed. Heuristics, in priority order:

1. **Module docstring / header doc** — the file-level docstring tree-sitter
   exposes (Python module string, Rust `//!`, JSDoc `@fileoverview`, Go package
   comment). First sentence, trimmed. *High confidence.*
2. **Leading comment block** — the first contiguous comment run before any code,
   if it reads as prose (not a license header or shebang — those are filtered).
   *High confidence.*
3. **Dominant export** — if one symbol carries the file's name (e.g.
   `ConfigLoader` in `config_loader.rs`) and has a docstring, reuse its doc.
   *Medium confidence.*
4. **README proximity** — a sibling `README`/`README.md` line that names the file
   or its directory. *Medium confidence.*
5. **Path / name fallback** — humanized path: `src/config/load.ts` →
   `"config / load"`. *Low confidence — always flagged.*

Confidence is recorded so the agent and H5 can weight it; low-confidence purposes
render with a marker:

```jsonc
{ "path": "src/config/load.ts",
  "purpose": "Loads and validates config.toml",
  "purpose_src": "docstring",      // docstring|comment|export|readme|path
  "purpose_confidence": "high" }   // high|medium|low
```

`purpose_src` / `purpose_confidence` are carried in the `outline` payload and
backed by `skeletons(tier=0)` text plus the structured `files.purpose` column; no
new columns are mandated beyond what fits in the existing rows (encode the marker
inline in `purpose` if a column add is undesirable, e.g. a leading `~` for low
confidence).

**Optional offline LLM enrichment.** A `carto index --enrich` pass may rewrite
low-confidence purposes with an LLM. Rules:

- Runs **out of band**, never on the `serve` sync loop; it is a batch job.
- Results are **cached keyed by `content_hash`** so an unedited file is enriched
  once; a re-edit invalidates only that file's cached purpose.
- It **never blocks sync**: if enrichment hasn't run, Tier-0 ships the heuristic
  purpose. Enrichment is strictly an upgrade of the `path`-fallback rung.

---

## 4. Tier-1 skeleton rendering via tree-sitter tags

Per-language extraction uses tree-sitter **tags queries** (the `tags.scm`
convention, vendored under `grammars/` — [01-architecture.md](./01-architecture.md)
§2). Each grammar's query captures a language-agnostic set of node kinds:

| Capture | Maps to `symbols.kind` |
|---|---|
| `@definition.function` | `function` |
| `@definition.method` | `method` |
| `@definition.class` | `class` |
| `@definition.type` / `@definition.interface` | `type` / `interface` |
| `@definition.constant` | `const` |
| `@definition.module` | `module` |
| `@definition.field` | `field` |
| `@doc` (preceding comment/docstring) | → `symbols.doc` |

**Signature rendering is language-agnostic by construction:** we take the source
span from the symbol's start to the first body-delimiter node the grammar marks
(`{`, `:` + suite, `=>` body, `where`/block), and keep everything before it. That
yields the real declaration text — name, params, return/where clauses, generics,
visibility — with the body removed, no per-language pretty-printer:

```rust
// rust
pub fn build_index(ctx: &BuildCtx, store: &mut StoreTxn) -> Result<()>
// typescript
parse(src: string): Config
// python
def parse(self, src: str) -> Config
// go
func (l *ConfigLoader) Parse(src string) (*Config, error)
```

The rendered string lands in `symbols.signature` and, with the doc, in the
pre-rendered `skeletons(tier=1).text`.

**Nesting** renders from `symbols.parent_id`
([02-data-model.md](./02-data-model.md) §2). `outline` reconstructs the class →
method tree by `parent_id` and indents accordingly, eliding bodies with `…`:

```
class ConfigLoader            // src/config/load.ts#ConfigLoader:class  rank 0.81
  "Reads layered config."     // symbols.doc
  parse(src: string): Config  // …/ConfigLoader/parse:method  rank 0.74
  merge(a: Config, b: Config): Config
```

Symbols with no captured signature (anonymous, macro-generated) are still listed
by `kind` + location so the tree is complete.

---

## 5. The descent loop (worked example)

Task: **"where is config validated?"**

**Step 1 — `outline("", tier=0, max_tokens=2000)`.** Repo root, purpose lines
only. The agent scans ~1500 one-liners (truncated by rank to fit 2000 tokens) and
spots the `src/config/` cluster.
*≈ 1.6k tokens.*

```jsonc
{ "entries": [
  { "path": "src/config/load.ts",   "purpose": "Loads and validates config.toml" },
  { "path": "src/config/schema.ts", "purpose": "Config schema + validation rules" },
  { "path": "src/config/defaults.ts","purpose": "Default config values" } ],
  "truncated": true, "dropped": 1100, "token_est": 1580 }
```

**Step 2 — `outline("src/config", tier=1, depth=1)`.** Signatures + docstrings
for that directory only. `validate` and `Schema.check` surface.
*≈ 0.9k tokens.*

```jsonc
{ "entries": [
  { "path": "src/config/schema.ts", "symbols": [
    { "key": "src/config/schema.ts#Schema:class", "signature": "class Schema", "rank": 0.7 },
    { "key": "src/config/schema.ts#validate:function",
      "signature": "validate(c: Config, s: Schema): Result<Config>",
      "doc": "Validates a parsed config against the schema.", "rank": 0.78 } ] } ],
  "token_est": 910 }
```

**Step 3 — `expand("src/config/schema.ts#validate:function")`.** The one body
that answers the question, read live from disk.
*≈ 0.3k tokens.*

**Total: ≈ 2.8k tokens** to a precise, source-grounded answer — against ~30k+ to
read the three candidate files in full, and with no risk of missing a fourth file
the grep didn't match (Tier-0 listed them all).

---

## 6. Incremental behavior (II 8, US 9)

A single-file edit invalidates **only that file's H1 rows**. The Parse stage's
`apply()` ([01-architecture.md](./01-architecture.md) §3,
[10-incremental-sync.md](./10-incremental-sync.md)) does delete-and-reinsert
scoped by `file_id`:

1. Watcher fires; `content_hash` (blake3) is recomputed. If unchanged (no-op save),
   the delta is **dropped before any parse**
   ([02-data-model.md](./02-data-model.md) §4) — H1 does zero work.
2. If changed: tree-sitter **incremental reparse** of the dirty subtree only, then
   within one write transaction:
   - rewrite that file's `symbols` (and their `skeletons` rows),
   - re-derive `files.purpose` (heuristics in §3 re-run; cheap),
   - bump `meta.generation`; touched rows stamp the new `generation`.
3. Tier-2 needs **no invalidation** — it is a live disk read, so it is current the
   instant the file is saved; only `start_byte..end_byte` must track, which the
   reparse updates.

Why the scores are high:

- **II 8 (integrity under mutation):** `stable_key` keeps agent-held handles valid
  across the edit ([02-data-model.md](./02-data-model.md) §3); a body change does
  not move a symbol's identity, so an `expand` handle from before the edit still
  resolves (to the new body). Only rename breaks it, and H4 emits a remap row.
- **US 9 (update speed):** the unit of work is one file's symbols — typically tens
  of rows — well inside the single-file-edit **p95 ≤ 200 ms** target
  ([README](./README.md) §"Hard operating constraints"). No cross-file recompute:
  Tier-0 and Tier-1 are purely local to the edited file. (Rank, which *is*
  cross-file, is H2's concern, not H1's.)

---

## 7. Budgeting

`outline` implements the **shared token-budget protocol**
([03-mcp-tool-surface.md](./03-mcp-tool-surface.md) §4):

1. Accept optional `max_tokens`.
2. Order candidate entries by **rank** — for Tier-1, `symbols.rank` (H2
   centrality, `ORDER BY rank DESC`); for Tier-0, the max rank of a file's symbols
   (files with no symbols sort last, after a small lexical tiebreak so siblings
   stay grouped).
3. Fill using cached `skeletons.token_est`
   ([02-data-model.md](./02-data-model.md) §6) until the next entry would exceed
   `max_tokens`; set `truncated: true` and report `dropped: <count>`.
4. Always report the actual `token_est` returned.

```sql
-- Tier-1 candidates for a directory, rank-ordered for budgeted fill.
SELECT sk.text, sk.token_est, s.rank
FROM symbols s
JOIN files f      ON f.id = s.file_id
JOIN skeletons sk ON sk.symbol_id = s.id AND sk.tier = 1
WHERE f.path LIKE :dir_prefix || '%'
ORDER BY s.rank DESC;     -- caller stops appending at max_tokens
```

Because ranking is by centrality, a truncated outline keeps the architecturally
heaviest symbols — the agent never loses the load-bearing API to a long tail of
private helpers. This is the same mechanism H5 relies on
([09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md)).

---

## 8. Edge cases

| Case | Behavior |
|---|---|
| **Unknown language / parse failure** | File gets a **Tier-0-only** entry (path, size, heuristic purpose from comments/README/path). No `symbols`, no Tier-1. The repo map still lists it; `outline(tier=1)` returns it with an empty `symbols` array and `parse: "unavailable"`. (Matches [01-architecture.md](./01-architecture.md) §7.) |
| **Generated / minified** | Excluded by `[index].exclude` globs and `max_file_bytes` ([01-architecture.md](./01-architecture.md) §6). Kept at Tier-0 with `purpose: "generated"` (low confidence) so the agent knows it exists but is steered away. Never expanded by default. |
| **Huge but real files** | Over `max_file_bytes` → Tier-0 only (skeleton extraction skipped to protect the p95 budget), flagged `oversized`. Under the cap but large: parsed normally; `outline(tier=1)` relies on `max_tokens` truncation so a 500-symbol file never floods the agent. |
| **Binary / non-text** | Detected pre-parse; listed at Tier-0 with `purpose` from extension only, no Tier-1, never expanded. |
| **Empty / whitespace-only** | Listed at Tier-0 with `purpose: "(empty)"`; no symbols. |
| **Vendored docs/configs (`.md`, `.toml`, `.json`)** | Tier-0 purpose from first heading / top-level key; no Tier-1 unless a grammar is registered for the format. |

---

## 9. Open questions / risks

- **Purpose quality without an LLM.** Heuristic purposes for comment-light files
  fall to the path fallback, which is weak. Risk: a `src/util/x.ts` with no doc
  reads as `"util / x"` and the agent skips a relevant file. Mitigation: the
  `--enrich` pass (§3); open question is whether to ship enrichment results in the
  index or as a sidecar so `.carto/index.db` stays a pure deterministic artifact.
- **Signature boundary heuristic.** "Everything before the body delimiter" is
  robust for mainstream grammars but can mis-slice exotic syntax (multi-clause
  Haskell-style guards, heavy macros, decorators that wrap signatures). Need a
  per-grammar override hook in `grammars/*/tags.scm` for the long tail.
- **Tier-0 budget at very large repos.** A monorepo with >50k files blows the
  Tier-0 token budget even at one line each. Rank-truncation helps, but the
  default root `outline` may need a directory-rollup mode (summarize a dir as one
  line until the agent descends). Open: is rollup an H1 concern or an H5 planning
  concern?
- **Confidence encoding.** Whether `purpose_confidence` warrants its own column on
  `files` or an inline marker in `purpose` — a schema-vs-payload tradeoff to settle
  with [02-data-model.md](./02-data-model.md) before v1.
- **Doc trimming fidelity.** Aggressively trimming docstrings to one line for
  Tier-1 can drop a `@param` that mattered. Tier-1 keeps the first sentence;
  `expand(include_doc=true)` recovers the full doc — but the agent must know to
  ask. Validate against the benchmark harness
  ([11-benchmark-harness.md](./11-benchmark-harness.md)).
