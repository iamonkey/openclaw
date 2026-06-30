//! SQLite schema (DDL) for the Cartograph index.
//!
//! Mirrors `docs/codebase-understanding/02-data-model.md`. For this vertical
//! slice we materialize: `meta`, `files`, `symbols`, `edges`, `skeletons`, plus
//! the v1-only `raw_refs` staging table (see comment below). All `CREATE`s are
//! `IF NOT EXISTS` so the migration is idempotent and safe to run on every open.

use anyhow::Result;
use rusqlite::Connection;

/// Bump when the DDL changes in a way that needs a migration.
pub const SCHEMA_VERSION: i64 = 1;

/// Apply the schema (idempotent) and record the schema version in `meta`.
pub fn apply(conn: &Connection) -> Result<()> {
    // Foreign keys must be enabled per-connection for ON DELETE CASCADE to fire.
    conn.execute_batch(
        r#"
PRAGMA foreign_keys = ON;

-- ── Meta ───────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- ── Files ──────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS files (
  id           INTEGER PRIMARY KEY,
  path         TEXT NOT NULL UNIQUE,
  lang         TEXT,
  purpose      TEXT,
  size_bytes   INTEGER NOT NULL,
  content_hash BLOB NOT NULL,
  generation   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_files_lang ON files(lang);

-- ── Symbols ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS symbols (
  id          INTEGER PRIMARY KEY,
  file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  stable_key  TEXT NOT NULL,
  name        TEXT NOT NULL,
  fqn         TEXT,
  kind        TEXT NOT NULL,
  signature   TEXT,
  doc         TEXT,
  start_byte  INTEGER NOT NULL,
  end_byte    INTEGER NOT NULL,
  start_row   INTEGER NOT NULL,
  end_row     INTEGER NOT NULL,
  parent_id   INTEGER REFERENCES symbols(id) ON DELETE CASCADE,
  rank        REAL NOT NULL DEFAULT 0,
  generation  INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_symbols_stable ON symbols(stable_key);
CREATE INDEX IF NOT EXISTS idx_symbols_file   ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_name   ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_fqn    ON symbols(fqn);
CREATE INDEX IF NOT EXISTS idx_symbols_rank   ON symbols(rank DESC);
CREATE INDEX IF NOT EXISTS idx_symbols_parent ON symbols(parent_id);

-- ── Edges ──────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS edges (
  id        INTEGER PRIMARY KEY,
  src_id    INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  dst_id    INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  kind      TEXT NOT NULL,
  resolved  INTEGER NOT NULL DEFAULT 1,
  generation INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src_id, kind);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst_id, kind);
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_uniq ON edges(src_id, dst_id, kind);

-- ── Skeletons ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS skeletons (
  symbol_id  INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  tier       INTEGER NOT NULL,
  text       TEXT NOT NULL,
  token_est  INTEGER NOT NULL,
  PRIMARY KEY (symbol_id, tier)
);

-- ── Raw refs (v1 staging table; NOT in doc 02) ─────────────────────────────
-- Unresolved references emitted by the parser, keyed by callee *name* rather
-- than a concrete symbol id. `resolve_edges()` reads this table, performs
-- name resolution, and materializes the `edges` graph. Rows cascade-delete
-- with their owning symbol so re-indexing a file clears its stale refs.
CREATE TABLE IF NOT EXISTS raw_refs (
  src_id      INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  target_name TEXT NOT NULL,
  kind        TEXT NOT NULL,
  generation  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_raw_refs_src    ON raw_refs(src_id);
CREATE INDEX IF NOT EXISTS idx_raw_refs_target ON raw_refs(target_name);

-- ── File co-change (H3, `06-h3-impact-radius.md`) ──────────────────────────
-- File-level association-rule signal mined from git history. Symmetric: both
-- (a,b) and (b,a) are stored so a single-direction lookup suffices.
CREATE TABLE IF NOT EXISTS file_cochange (
  a_file     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  b_file     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  support    INTEGER NOT NULL,
  confidence REAL NOT NULL,
  lift       REAL NOT NULL,
  PRIMARY KEY (a_file, b_file)
);
CREATE INDEX IF NOT EXISTS idx_file_cochange_a ON file_cochange(a_file, lift DESC);
"#,
    )?;

    // Record schema version (idempotent upsert).
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;
    // Seed generation counter if absent.
    conn.execute(
        "INSERT OR IGNORE INTO meta(key, value) VALUES('generation', '0')",
        [],
    )?;

    Ok(())
}
