# 02 — Data Model

Canonical SQLite schema for the Cartograph index. Every producer writes here;
every tool and the planner (H5) read here. This is the single source of truth for
table and column names referenced by the hypothesis specs.

SQLite in **WAL mode**. IDs are stable across incremental updates wherever
possible (see *Symbol identity* below) so that handles the agent holds survive a
resync.

---

## 1. Entity overview

```
files ──< symbols ──< edges >── symbols
  │           │
  │           ├──< skeletons        (H1 tiered text)
  │           ├──< symbol_rank      (H2 centrality)        [denormalized onto symbols.rank too]
  │           └──< cochange >── symbols   (H3 git co-change pairs)
  │
  └──< file_cochange >── files       (H3 file-level fallback)

snapshots ──< ast_diffs              (H4 structural deltas between revisions)
meta                                  (generation counter, schema version, build stamps)
```

---

## 2. Schema (DDL)

```sql
-- ── Files ────────────────────────────────────────────────────────────────
CREATE TABLE files (
  id           INTEGER PRIMARY KEY,
  path         TEXT NOT NULL UNIQUE,        -- repo-root-relative, forward slashes
  lang         TEXT,                        -- tree-sitter grammar id, NULL if unknown
  purpose      TEXT,                        -- Tier-0 one-line purpose (H1)
  size_bytes   INTEGER NOT NULL,
  content_hash BLOB NOT NULL,               -- blake3 of file bytes; drives dirty detection
  generation   INTEGER NOT NULL             -- generation this row was last written at
);
CREATE INDEX idx_files_lang ON files(lang);

-- ── Symbols ──────────────────────────────────────────────────────────────
-- A symbol = a named, addressable code entity (function, method, class, type,
-- interface, const, module). Bodies are NOT stored here (see skeletons/expand).
CREATE TABLE symbols (
  id          INTEGER PRIMARY KEY,
  file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  stable_key  TEXT NOT NULL,                -- see "Symbol identity"; UNIQUE per repo
  name        TEXT NOT NULL,                -- local name, e.g. "parseConfig"
  fqn         TEXT,                         -- best-effort fully-qualified name
  kind        TEXT NOT NULL,                -- function|method|class|type|interface|const|module|field
  signature   TEXT,                         -- rendered signature (Tier-1), no body
  doc         TEXT,                         -- leading docstring/comment, trimmed
  start_byte  INTEGER NOT NULL,             -- body span in current file snapshot
  end_byte    INTEGER NOT NULL,
  start_row   INTEGER NOT NULL,
  end_row     INTEGER NOT NULL,
  parent_id   INTEGER REFERENCES symbols(id) ON DELETE CASCADE,  -- enclosing symbol (method→class)
  rank        REAL NOT NULL DEFAULT 0,      -- denormalized PageRank (H2) for fast sort
  generation  INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_symbols_stable ON symbols(stable_key);
CREATE INDEX idx_symbols_file   ON symbols(file_id);
CREATE INDEX idx_symbols_name   ON symbols(name);
CREATE INDEX idx_symbols_fqn    ON symbols(fqn);
CREATE INDEX idx_symbols_rank   ON symbols(rank DESC);
CREATE INDEX idx_symbols_parent ON symbols(parent_id);

-- ── Edges (the topology graph, H2) ───────────────────────────────────────
-- Directed relationships between symbols. `src` references/depends-on `dst`.
CREATE TABLE edges (
  id        INTEGER PRIMARY KEY,
  src_id    INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  dst_id    INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  kind      TEXT NOT NULL,                  -- call|import|inherit|implement|typeref|read|write
  resolved  INTEGER NOT NULL DEFAULT 1,     -- 1=resolved to a def, 0=unresolved/heuristic
  generation INTEGER NOT NULL
);
CREATE INDEX idx_edges_src ON edges(src_id, kind);
CREATE INDEX idx_edges_dst ON edges(dst_id, kind);   -- powers who_calls (reverse)
CREATE UNIQUE INDEX idx_edges_uniq ON edges(src_id, dst_id, kind);

-- ── Skeletons (H1 tiered hydration text) ─────────────────────────────────
-- Pre-rendered text per symbol at each tier so outline() is a cheap read, not
-- a re-render. Tier-2 (full body) is read live from disk via expand(), not stored.
CREATE TABLE skeletons (
  symbol_id  INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  tier       INTEGER NOT NULL,              -- 0=purpose line, 1=signature+doc
  text       TEXT NOT NULL,
  token_est  INTEGER NOT NULL,              -- cached token estimate for budgeting (H5)
  PRIMARY KEY (symbol_id, tier)
);

-- ── Symbol co-change (H3) ────────────────────────────────────────────────
-- Association-rule signal: how often two symbols changed in the same commit.
CREATE TABLE cochange (
  a_id        INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  b_id        INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  support     INTEGER NOT NULL,             -- # commits touching both
  confidence  REAL NOT NULL,                -- support / (# commits touching a)
  lift        REAL NOT NULL,                -- confidence / base_rate(b)
  generation  INTEGER NOT NULL,
  PRIMARY KEY (a_id, b_id)
);
CREATE INDEX idx_cochange_a ON cochange(a_id, lift DESC);

-- File-level fallback when symbol-level attribution is unavailable
-- (e.g. unparsed files, pre-history symbols).
CREATE TABLE file_cochange (
  a_file     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  b_file     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  support    INTEGER NOT NULL,
  confidence REAL NOT NULL,
  lift       REAL NOT NULL,
  PRIMARY KEY (a_file, b_file)
);

-- ── Snapshots & AST diffs (H4) ───────────────────────────────────────────
CREATE TABLE snapshots (
  id        INTEGER PRIMARY KEY,
  rev       TEXT NOT NULL,                  -- git rev or "WORKING" for dirty tree
  taken_at_generation INTEGER NOT NULL
);

CREATE TABLE ast_diffs (
  id         INTEGER PRIMARY KEY,
  from_snap  INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
  to_snap    INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
  symbol_key TEXT NOT NULL,                 -- stable_key of the changed symbol
  change     TEXT NOT NULL,                 -- added|removed|renamed|signature|body|moved
  detail     TEXT NOT NULL,                 -- compact human+machine readable, e.g. "+param x: int"
  token_est  INTEGER NOT NULL
);
CREATE INDEX idx_ast_diffs_range ON ast_diffs(from_snap, to_snap);

-- ── Meta ─────────────────────────────────────────────────────────────────
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- seeded keys: schema_version, generation, last_full_build_rev, head_rev
```

---

## 3. Symbol identity (`stable_key`)

Handles returned to the agent must survive edits, so a symbol's identity cannot be
its byte offset or row id alone. `stable_key` is a content-independent path:

```
stable_key = "<repo-rel-path>#<container-path>/<name>:<kind>"
e.g.        "src/config/load.ts#ConfigLoader/parse:method"
            "crates/carto-core/src/lib.rs#build_index:function"
```

- Survives body edits, line shifts, and signature changes (same name+kind+container).
- A **rename** breaks the key by design — H4's diff detects rename
  (`change='renamed'`) and the History/Diff stages emit a key-remap row so handles
  and co-change history follow the rename instead of resetting.
- Collisions (overloads, same-name nested scopes) get a disambiguating
  `~<ordinal>` suffix assigned in source order, stable as long as count is stable.

The integer `symbols.id` is the join key internally (fast); `stable_key` is the
external handle exposed in tool responses. Tools accept either.

---

## 4. Generation & incremental writes

- `meta.generation` is a monotonic counter bumped once per committed delta.
- Every row carries the `generation` it was last written at. Incremental
  `apply()` (see [10-incremental-sync.md](./10-incremental-sync.md)) only rewrites
  rows for touched files: delete-and-reinsert symbols/edges/skeletons for each
  dirty file within one transaction, then bump `generation`.
- `content_hash` on `files` is the dirty-detection gate: if a watcher event's file
  hashes unchanged (editor save with no content change), the delta is dropped
  before any parse.
- Append-only tables (`cochange` accrues, `ast_diffs` accrues) are pruned by the
  retention keys in `[history]` config rather than rewritten in place.

---

## 5. Graph queries via recursive CTEs

The graph lives in `edges`; traversal is plain SQL. Canonical patterns the tools
compile to:

**`who_calls(X)` — reverse edges, one hop:**
```sql
SELECT s.* FROM edges e JOIN symbols s ON s.id = e.src_id
WHERE e.dst_id = :x AND e.kind = 'call';
```

**`neighborhood(X, depth=N)` — bounded BFS, both directions:**
```sql
WITH RECURSIVE nb(id, depth, dir) AS (
  SELECT :x, 0, 'self'
  UNION
  SELECT e.dst_id, nb.depth+1, 'out' FROM edges e JOIN nb ON e.src_id = nb.id
    WHERE nb.depth < :n
  UNION
  SELECT e.src_id, nb.depth+1, 'in'  FROM edges e JOIN nb ON e.dst_id = nb.id
    WHERE nb.depth < :n
)
SELECT DISTINCT s.*, nb.depth, nb.dir
FROM nb JOIN symbols s ON s.id = nb.id
ORDER BY s.rank DESC;          -- H2 centrality ranks the frontier
```

The `ORDER BY rank DESC` is what lets H5 truncate a neighborhood to a token budget
while keeping the architecturally heaviest nodes (see
[09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md)).

**`impact(X)` — static neighbors ∪ co-change, ranked:** see
[06-h3-impact-radius.md](./06-h3-impact-radius.md) for the fusion query.

---

## 6. Sizing & performance notes

- At ~100k LOC expect roughly 10k–40k symbols and 50k–200k edges — comfortably in
  SQLite's wheelhouse; the recursive CTEs stay sub-millisecond with the indexes
  above for `depth ≤ 3`.
- `token_est` is cached on `skeletons` and `ast_diffs` so the planner never
  re-tokenizes during budgeting. Use a cheap heuristic (bytes/4 or a small BPE
  estimator) computed once at write time.
- WAL checkpointing runs on idle (no pending deltas) to keep the `-wal` file from
  growing without ever blocking a resync.
