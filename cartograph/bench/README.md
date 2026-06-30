# Cartograph Benchmark Corpus and Task Set

This directory holds the **small, self-contained benchmark fixture** for Cartograph:
a coherent TypeScript application (`corpus/`) that Cartograph indexes, a
machine-readable set of retrieval tasks with ground-truth answers
(`tasks.json`), and a token-counting helper (`tokens.mjs`) that mirrors the Rust
token heuristic so baseline-vs-Cartograph comparisons are apples-to-apples.

It is a **micro-corpus**, not the full ≥100k-LOC polyglot corpus described in
`docs/codebase-understanding/11-benchmark-harness.md`. Its purpose is fast,
deterministic, offline iteration on the producers/tools and on the scorer wiring
(localization@k, impact precision/recall, bug-localize) before pointing the
harness at the heavy real-world repo.

## Layout

```
bench/
├── README.md            ← this file
├── tasks.json           ← retrieval tasks with ground-truth answer keys
├── tokens.mjs           ← ceil(bytes/4) token estimator (lib + CLI)
└── corpus/              ← the indexed TypeScript app ("taskman")
    ├── package.json
    ├── tsconfig.json
    └── src/
        ├── index.ts         barrel / public API
        ├── main.ts          CLI entry; fires synthetic requests through the router
        ├── server.ts        composition root: wires every layer together
        ├── config/          env-driven config loading + validation
        │   ├── env.ts         typed accessors over an env bag
        │   ├── load.ts        loadConfig / validateConfig
        │   └── schema.ts      AppConfig shape + DEFAULT_CONFIG
        ├── domain/          pure business logic (no I/O)
        │   ├── task.ts        Task entity + lifecycle state machine
        │   ├── priority.ts    priority ranking + comparators
        │   ├── query.ts       filter/sort descriptors
        │   ├── validate.ts    input validators -> Result<_, DomainError>
        │   ├── errors.ts      DomainError taxonomy + statusForError
        │   ├── stats.ts       aggregate metrics over tasks
        │   ├── search.ts      inverted-index full-text search
        │   └── events.ts      domain events + EventBus
        ├── repo/            storage + application services
        │   ├── types.ts       TaskRepository port
        │   ├── memory.ts      InMemoryTaskRepository (central hub)
        │   ├── observable.ts  decorator that emits events on mutation
        │   ├── audit.ts       AuditTrail (subscribes to the bus)
        │   ├── service.ts     TaskService (repo + search + audit)
        │   └── seed.ts        deterministic seed data
        └── http/            HTTP-ish request/response layer (no real socket)
            ├── types.ts       HttpRequest/HttpResponse shapes
            ├── parse.ts       query/page parsing
            ├── respond.ts     Result/error -> HttpResponse mapping
            ├── handlers.ts    task CRUD handlers (busiest caller)
            ├── router.ts      pattern router + dispatch
            ├── stats_handler.ts
            └── search_handler.ts
```

`*.test.ts` files are colocated (Vitest). They are part of the corpus on purpose:
they give the bug-localize tasks an executable spec and add realistic test-to-source
edges to the symbol graph.

### Why this shape

The app has **genuine cross-file references** so the symbol graph is non-trivial:

- `http/handlers.ts` calls into `repo/`, `domain/validate.ts`, `domain/task.ts`,
  and the local `http/` helpers — it is the highest-fan-out caller.
- `server.ts` imports from every layer (the natural "how does a request flow"
  entry point).
- `util/result.ts` (the `Result` type) and `util/paginate.ts` (`paginate`) are
  **high-fan-in** leaves: many modules depend on them, which is what the
  `what-breaks` / impact tasks traverse.
- Some files carry a leading JSDoc banner describing purpose; a few (e.g. the
  `types.ts` ports) are intentionally terser — this exercises summary/skeleton
  extraction with and without doc hints.

### The deliberate bug

`corpus/src/util/paginate.ts` `paginate()` contains a **clearly-commented
off-by-one**: the slice end is computed as `start + pageSize - 1` instead of
`start + pageSize`, so every page silently drops its last item. The rest of the
function is plausible and correct. `corpus/src/util/paginate.test.ts` encodes the
*intended* behavior (so it fails against the current code) and serves as the
ground-truth spec for the `bug-localize` tasks. This is the only intentional bug.

The corpus is **syntactically valid TypeScript that tree-sitter can parse**
(verified with `node --experimental-strip-types --check` on every file). It does
not need to compile under `tsc` strict mode or run.

## Task schema (`tasks.json`)

`tasks.json` is a JSON array. Each task:

```json
{
  "id": "where-pagination",
  "class": "where-is",
  "query": "Where is result pagination implemented?",
  "answer_files": ["src/util/paginate.ts"],
  "answer_symbols": ["src/util/paginate.ts#paginate:function"],
  "notes": "ground truth for localization@k"
}
```

| Field | Meaning |
|---|---|
| `id` | Stable task id. |
| `class` | One of `where-is`, `what-breaks`, `bug-localize`. |
| `query` | The natural-language prompt handed to the agent (no file/line hints). |
| `answer_files` | Ground-truth file set (repo-relative to `corpus/`). |
| `answer_symbols` | Ground-truth symbol set as **stable keys** (see below). |
| `notes` | Rationale / scoring guidance; not shown to the agent. |

### Task classes (map to harness metrics)

These mirror the three scored classes in
`docs/codebase-understanding/11-benchmark-harness.md` §4:

- **`where-is`** → localization@k. Agent emits a ranked symbol/file list; score is
  whether any gold symbol is in the top-k. (8 tasks.)
- **`what-breaks`** → impact precision/recall@k (F1). Agent predicts the impacted
  set when a symbol changes; compared to the co-change / call-graph gold set.
  (4 tasks.)
- **`bug-localize`** → localization for a bug. Agent points at the symbol holding
  the root cause given only the *symptom*. (2 tasks; both resolve to
  `paginate()`. One states the bug directly; the other surfaces the symptom at
  the `GET /tasks` endpoint so the agent must follow the call chain
  `handlers → repo.list → paginate` down to the leaf.)

### `answer_symbols` use stable keys

Each symbol is identified by the Cartograph **stable_key** format:

```
<path>#<container>/<name>:<kind>
```

- `<path>` is repo-relative to `corpus/` (e.g. `src/repo/memory.ts`).
- `<container>/` is present only for nested symbols (a method's class); top-level
  symbols omit it (e.g. `src/util/paginate.ts#paginate:function`).
- `<kind>` is the symbol kind: `function`, `class`, `method`, `interface`,
  `type`, `variable` (top-level `const`/`let`).

Examples from this corpus:

```
src/util/paginate.ts#paginate:function
src/repo/memory.ts#InMemoryTaskRepository:class
src/repo/memory.ts#InMemoryTaskRepository/create:method
src/domain/task.ts#Task:interface
src/util/result.ts#Result:type
src/domain/task.ts#VALID_TRANSITIONS:variable
```

All `answer_files` / `answer_symbols` were verified against the actual corpus, so
they are exact ground truth, not approximations.

## `tokens.mjs` — token heuristic parity

`tokens.mjs` is a dependency-free Node ESM module that mirrors the Rust
`estimate_tokens` heuristic used by `carto-core`:

```js
estimateTokens(text) === text.length === 0 ? 0 : Math.ceil(byteLength(text) / 4)
```

It counts **UTF-8 bytes** (matching Rust `str::len`), so for ASCII source the
estimate is byte-identical to the Rust path. Keeping the JS and Rust estimators in
lockstep means the harness can compare the **baseline** (naive grep + full-file
reads — every byte of every opened file costs tokens) against the **Cartograph**
arm (skeleton + lazy hydration) on the same ruler. See harness doc §2 (baseline)
and §5 (Token Cost / context precision).

Usage:

```bash
# library
node -e "import('./tokens.mjs').then(m => console.log(m.estimateTokens('hello world')))"  # 3

# CLI: per-file tokens + a total line
node tokens.mjs corpus/src/util/paginate.ts corpus/src/domain/task.ts

# whole corpus (enable globstar in your shell, or pass a find result)
node tokens.mjs $(find corpus/src -name '*.ts')
```

CLI output is `<tokens>\t<path>` per file followed by `<total>\ttotal`.

## Verifying

```bash
# tasks.json is valid JSON
node -e "JSON.parse(require('fs').readFileSync('tasks.json'))"

# token helper runs
node tokens.mjs corpus/src/util/paginate.ts

# every corpus file is syntactically valid TS
for f in $(find corpus/src -name '*.ts'); do node --experimental-strip-types --check "$f"; done
```
