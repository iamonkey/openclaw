# 03 — MCP Tool Surface

Canonical tool contracts Cartograph exposes over MCP (stdio, JSON-RPC 2.0). These
are the *only* interface the agent sees; the hypothesis specs describe what backs
each one. Every tool is a thin adapter over an `IndexReader` method
([01-architecture.md](./01-architecture.md) §3) plus budget accounting.

Design rules:
- **Return the smallest useful unit + a handle to expand.** Never a whole file
  when a skeleton answers the question.
- **Handles are `stable_key` strings** ([02-data-model.md](./02-data-model.md) §3),
  valid across resyncs. Tools also accept integer ids for scripting.
- **Every response carries `generation`** so the client can detect staleness.
- **Every response carries `token_est`** of its payload so the agent (and H5) can
  budget. Tools accept a `max_tokens` cap and truncate by rank, reporting `truncated`.

---

## 1. Tool catalog

| Tool | Hypothesis | Purpose |
|---|---|---|
| `outline` | H1 | Tiered skeleton of a path or subtree (file tree → signatures). |
| `expand` | H1 | Full source body of one symbol (Tier-2), read live from disk. |
| `get_symbol` | H2 | Look up symbol(s) by name/fqn/kind; returns signature, location, rank. |
| `who_calls` | H2 | Reverse call/reference edges into a symbol. |
| `neighborhood` | H2 | Bounded, rank-ordered subgraph around a symbol. |
| `impact` | H3 | Blast radius: static neighbors fused with git co-change. |
| `slice` | (R) | Minimal statement set affecting a target. *(post-v1)* |
| `diff_context` | H4 | Structural AST diff between two revisions / working tree. |
| `plan_retrieval` | H5 | Budget-constrained multi-tool plan for a task description. |
| `search_symbols` | — | Fuzzy/substring symbol search (entry point when the agent has only a name fragment). |

---

## 2. Contracts

JSON Schemas below are abbreviated (types + required fields). Full schemas live in
`carto-mcp`. All tools may return `{ "error": { code, message } }` per JSON-RPC.

### `outline` — H1
The primary entry point. Descend the repo top-down.

```jsonc
// params
{
  "path": "src/config",        // file, dir, or "" for repo root
  "tier": 1,                    // 0 = tree+purpose, 1 = +signatures/docstrings  (2 via expand)
  "depth": 2,                   // dir recursion depth for directory paths
  "max_tokens": 2000           // optional cap; truncate low-rank entries first
}
// result
{
  "generation": 412,
  "token_est": 1840,
  "truncated": false,
  "entries": [
    { "path": "src/config/load.ts", "purpose": "Loads and validates config.toml",
      "symbols": [
        { "key": "src/config/load.ts#ConfigLoader:class", "signature": "class ConfigLoader",
          "rank": 0.81, "doc": "Reads layered config." },
        { "key": "src/config/load.ts#ConfigLoader/parse:method",
          "signature": "parse(src: string): Config", "rank": 0.74 }
      ]
    }
  ]
}
```

### `expand` — H1
```jsonc
{ "symbol": "src/config/load.ts#ConfigLoader/parse:method",
  "include_doc": true }
// →
{ "generation": 412, "token_est": 320,
  "symbol": "…/parse:method", "lang": "typescript",
  "source": "parse(src: string): Config {\n  …full body…\n}",
  "location": { "path": "src/config/load.ts", "start_row": 40, "end_row": 78 } }
```

### `get_symbol` — H2
```jsonc
{ "name": "parse", "kind": "method", "fqn": "ConfigLoader.parse" }  // any subset
// →
{ "generation": 412, "matches": [
  { "key": "src/config/load.ts#ConfigLoader/parse:method",
    "signature": "parse(src: string): Config", "rank": 0.74,
    "location": {…}, "doc": "…" } ] }
```

### `who_calls` — H2
```jsonc
{ "symbol": "…/parse:method", "kinds": ["call","typeref"], "max_tokens": 1500 }
// →
{ "generation": 412, "callers": [
  { "key": "src/cli/main.ts#run:function", "rank": 0.66, "edge": "call",
    "site": { "path": "src/cli/main.ts", "row": 22 } } ],
  "truncated": false }
```

### `neighborhood` — H2
```jsonc
{ "symbol": "…/parse:method", "depth": 2, "dir": "both",   // in | out | both
  "kinds": ["call","import","typeref"], "max_tokens": 2500 }
// →
{ "generation": 412,
  "center": "…/parse:method",
  "nodes": [ { "key":"…", "rank":0.74, "depth":0, "dir":"self", "signature":"…" }, … ],
  "edges": [ { "src":"…", "dst":"…", "kind":"call" }, … ],
  "truncated": true }            // low-rank frontier dropped to fit budget
```

### `impact` — H3
```jsonc
{ "symbol": "…/parse:method", "static_depth": 2,
  "include_cochange": true, "min_lift": 1.5, "max_tokens": 2000 }
// →
{ "generation": 412, "cochange": "available",
  "items": [
    { "key": "src/config/schema.ts#Schema:class", "source": "static",
      "reason": "typeref depth 1", "weight": 0.9 },
    { "key": "src/config/load.test.ts#tests:module", "source": "cochange",
      "reason": "changed together 14× (lift 3.2)", "weight": 0.7 } ],
  "truncated": false }
```

### `slice` — Behavioral Slice (post-v1)
```jsonc
{ "symbol": "…/parse:method", "direction": "backward",  // backward = what affects it
  "criterion": "return", "max_tokens": 2500 }
// → minimal ordered statement set with locations (see 07-behavioral-slice.md)
```

### `diff_context` — H4
```jsonc
{ "from": "HEAD~1", "to": "WORKING", "scope": "src/config" }   // scope optional
// →
{ "generation": 412, "changes": [
  { "symbol": "…/parse:method", "change": "signature",
    "detail": "+param strict: boolean", "token_est": 12 },
  { "symbol": "…/validate:function", "change": "renamed",
    "detail": "validate → validateStrict", "token_est": 8 } ] }
```

### `plan_retrieval` — H5
```jsonc
{ "task": "Fix the off-by-one in pagination when page size is 1",
  "token_budget": 6000 }
// →
{ "generation": 412, "budget": 6000, "spent_est": 5120,
  "plan": [ { "tool":"search_symbols", "args":{…}, "why":"locate pagination" },
            { "tool":"neighborhood", "args":{…}, "why":"callers of paginate" },
            { "tool":"expand", "args":{…}, "why":"read the off-by-one site" } ],
  "context": [ /* hydrated payloads, already budget-truncated */ ] }
```
The planner returns *both* the plan (auditable) and the hydrated context so the
agent can act in one round-trip, or just the plan if `dry_run: true`. See
[09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md).

### `search_symbols` — entry point
```jsonc
{ "query": "paginat", "limit": 20 }   // substring/fuzzy over name + fqn
// → ranked matches (by rank then match quality), same shape as get_symbol
```

---

## 3. Transport & lifecycle

- **Framing:** newline-delimited JSON-RPC 2.0 over stdio (MCP default). `carto
  serve` reads requests on stdin, writes responses/notifications on stdout, logs
  on stderr only.
- **Capabilities:** advertises `tools` (the catalog above). No prompt/resource
  capabilities in v1.
- **Notifications:** during cold build or large resync, emit
  `notifications/message` (logging) with progress; tools return
  `{ "status": "index_building", "generation": N }` until ready rather than
  blocking or erroring.
- **Cancellation:** honor MCP `$/cancelRequest`; long planner calls check the
  cancel flag between steps.

---

## 4. Token-budget protocol (shared)

Every retrieval tool implements the same budget contract so H5 can compose them:

1. Accept optional `max_tokens`.
2. Order candidate payload items by **rank** (H2 centrality) — or by H3 weight for
   `impact`, by depth-then-rank for `neighborhood`.
3. Fill until the next item would exceed `max_tokens`; set `truncated: true` and
   include a `dropped: <count>` field so nothing is silently lost.
4. Always report actual `token_est` of what was returned.

This is the contract that makes "maximum relevance per token" mechanical rather
than guessed.
