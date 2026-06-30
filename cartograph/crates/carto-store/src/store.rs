//! `Store` — the SQLite-backed index store for Cartograph.
//!
//! Owns a single `rusqlite::Connection` opened in WAL mode with the schema in
//! [`crate::schema`] applied. All write paths that touch multiple tables run in
//! a single transaction so re-indexing is atomic and idempotent.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use carto_model::{
    estimate_tokens, EdgeKind, FileId, FileRec, RawRef, RawSymbol, Skeleton, Symbol, SymbolId,
    SymbolKind, Tier,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::schema;

/// SQLite-backed index store. Holds one owned connection.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if absent) at `path`, apply schema, set WAL mode.
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite db at {}", path.display()))?;
        Self::init(conn)
    }

    /// In-memory store for tests.
    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory().context("opening in-memory sqlite db")?;
        Self::init(conn)
    }

    /// Shared setup: WAL + foreign keys + schema migration.
    fn init(conn: Connection) -> Result<Store> {
        // WAL for concurrent readers + non-blocking resync. (No-op for
        // `:memory:` but harmless.) `query_row` since WAL pragma returns a row.
        let _: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .context("setting WAL journal_mode")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::apply(&conn)?;
        Ok(Store { conn })
    }

    // ── Generation counter ──────────────────────────────────────────────────

    /// Current generation counter (meta key "generation", default 0).
    pub fn generation(&self) -> Result<i64> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Increment and return the new generation.
    pub fn bump_generation(&mut self) -> Result<i64> {
        let next = self.generation()? + 1;
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('generation', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [next.to_string()],
        )?;
        Ok(next)
    }

    // ── Upsert ──────────────────────────────────────────────────────────────

    /// Atomically upsert one file and its extracted content. Existing rows for
    /// this path are deleted first (symbols/skeletons/raw_refs cascade), then
    /// the file row, symbols (with parent links), tier-1 skeletons, and raw_refs
    /// are inserted. Returns the file id.
    pub fn upsert_file(
        &mut self,
        path: &str,
        lang: Option<&str>,
        purpose: Option<&str>,
        size_bytes: i64,
        content_hash: &[u8],
        symbols: &[RawSymbol],
        refs: &[RawRef],
        generation: i64,
    ) -> Result<FileId> {
        let tx = self.conn.transaction()?;

        // Delete any prior file row for this path. ON DELETE CASCADE clears the
        // file's symbols, and those cascade to skeletons/edges/raw_refs.
        tx.execute("DELETE FROM files WHERE path = ?1", params![path])?;

        // Insert the file row.
        tx.execute(
            "INSERT INTO files(path, lang, purpose, size_bytes, content_hash, generation)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![path, lang, purpose, size_bytes, content_hash, generation],
        )?;
        let file_id = tx.last_insert_rowid();

        // Insert symbols in order, mapping vec-index -> new db id so we can wire
        // up parent_id (RawSymbol.parent_idx) and raw_refs (RawRef.src_idx).
        // RawSymbol.parent_idx must reference an *earlier* index (parents are
        // emitted before children); we rely on that to resolve parent ids.
        let mut id_for_idx: Vec<SymbolId> = Vec::with_capacity(symbols.len());
        // Disambiguate colliding stable_keys within a file. Real-world code
        // produces same path+container+name+kind collisions (e.g. overloads, or
        // same-named members across sibling type literals); the schema's UNIQUE
        // index would otherwise reject the second one and abort the whole build.
        // We append "~<n>" in source order per the data-model spec (§3).
        let mut key_counts: HashMap<String, u32> = HashMap::new();
        {
            let mut insert_sym = tx.prepare(
                "INSERT INTO symbols(
                    file_id, stable_key, name, fqn, kind, signature, doc,
                    start_byte, end_byte, start_row, end_row, parent_id, rank, generation)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)",
            )?;
            let mut insert_skel = tx.prepare(
                "INSERT INTO skeletons(symbol_id, tier, text, token_est)
                 VALUES(?1, ?2, ?3, ?4)",
            )?;

            for (idx, rs) in symbols.iter().enumerate() {
                let parent_id: Option<SymbolId> = match rs.parent_idx {
                    Some(p) => Some(*id_for_idx.get(p).with_context(|| {
                        format!(
                            "symbol {idx} ({}) references parent_idx {p} which has not been \
                             inserted yet (parents must precede children)",
                            rs.name
                        )
                    })?),
                    None => None,
                };

                // Make the stable_key unique within the file (see key_counts above).
                let n = key_counts.entry(rs.stable_key.clone()).or_insert(0);
                *n += 1;
                let stable_key = if *n == 1 {
                    rs.stable_key.clone()
                } else {
                    format!("{}~{}", rs.stable_key, *n)
                };

                insert_sym.execute(params![
                    file_id,
                    stable_key,
                    rs.name,
                    rs.fqn,
                    rs.kind.as_str(),
                    rs.signature,
                    rs.doc,
                    rs.start_byte,
                    rs.end_byte,
                    rs.start_row,
                    rs.end_row,
                    parent_id,
                    generation,
                ])?;
                let sym_id = tx.last_insert_rowid();
                id_for_idx.push(sym_id);

                // Tier-1 skeleton: signature + optional doc.
                let text = render_skeleton(rs);
                let token_est = estimate_tokens(&text);
                insert_skel.execute(params![sym_id, Tier::Signature.as_i64(), text, token_est])?;
            }

            // Insert raw_refs, mapping src_idx -> the new symbol id.
            let mut insert_ref = tx.prepare(
                "INSERT INTO raw_refs(src_id, target_name, kind, generation)
                 VALUES(?1, ?2, ?3, ?4)",
            )?;
            for r in refs {
                let src_id = *id_for_idx.get(r.src_idx).with_context(|| {
                    format!(
                        "raw_ref to '{}' has src_idx {} out of range ({} symbols)",
                        r.target_name,
                        r.src_idx,
                        symbols.len()
                    )
                })?;
                insert_ref.execute(params![src_id, r.target_name, r.kind.as_str(), generation])?;
            }
        }

        tx.commit()?;
        Ok(file_id)
    }

    // ── File reads ──────────────────────────────────────────────────────────

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRec>> {
        let rec = self
            .conn
            .query_row(
                "SELECT id, path, lang, purpose, size_bytes, content_hash, generation
                 FROM files WHERE path = ?1",
                params![path],
                row_to_file,
            )
            .optional()?;
        Ok(rec)
    }

    pub fn list_files(&self) -> Result<Vec<FileRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, lang, purpose, size_bytes, content_hash, generation
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], row_to_file)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ── Symbol reads ────────────────────────────────────────────────────────

    pub fn symbols_in_file(&self, file_id: FileId) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(&format!(
            "{SYMBOL_SELECT} WHERE file_id = ?1 ORDER BY start_byte ASC"
        ))?;
        let rows = stmt.query_map(params![file_id], row_to_symbol)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn symbol_by_key(&self, key: &str) -> Result<Option<Symbol>> {
        let sym = self
            .conn
            .prepare(&format!("{SYMBOL_SELECT} WHERE stable_key = ?1"))?
            .query_row(params![key], row_to_symbol)
            .optional()?;
        Ok(sym)
    }

    /// Substring/case-insensitive match on name or fqn, ordered by rank DESC.
    pub fn search_symbols(&self, query: &str, limit: i64) -> Result<Vec<Symbol>> {
        // Escape LIKE wildcards in the user query so `%`/`_` are literal, then
        // wrap in `%...%` for a substring match. ESCAPE clause makes `\` literal.
        let needle = format!("%{}%", escape_like(query));
        let mut stmt = self.conn.prepare(&format!(
            "{SYMBOL_SELECT}
             WHERE name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
                OR fqn  LIKE ?1 ESCAPE '\\' COLLATE NOCASE
             ORDER BY rank DESC, name ASC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![needle, limit], row_to_symbol)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn skeleton(&self, sym: SymbolId, tier: Tier) -> Result<Option<Skeleton>> {
        let skel = self
            .conn
            .query_row(
                "SELECT symbol_id, tier, text, token_est
                 FROM skeletons WHERE symbol_id = ?1 AND tier = ?2",
                params![sym, tier.as_i64()],
                |r| {
                    let tier_i: i64 = r.get(1)?;
                    Ok(Skeleton {
                        symbol_id: r.get(0)?,
                        tier: Tier::from_i64(tier_i).unwrap_or(Tier::Signature),
                        text: r.get(2)?,
                        token_est: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(skel)
    }

    // ── Graph queries ───────────────────────────────────────────────────────

    /// Reverse edges into `sym` (callers/referencers), ordered by rank DESC.
    /// `kinds` filters by edge kind; an empty slice means "any kind".
    pub fn who_calls(&self, sym: SymbolId, kinds: &[EdgeKind]) -> Result<Vec<Symbol>> {
        if kinds.is_empty() {
            let mut stmt = self.conn.prepare(&format!(
                "{SYMBOL_SELECT_PREFIX}
                 FROM edges e JOIN symbols s ON s.id = e.src_id
                 WHERE e.dst_id = ?1
                 ORDER BY s.rank DESC, s.name ASC"
            ))?;
            let rows = stmt.query_map(params![sym], row_to_symbol)?;
            return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }

        // Build an IN (...) list of quoted kind strings. Kind strings come from
        // the EdgeKind enum (a closed set), so there is no injection surface.
        let kind_list = kinds
            .iter()
            .map(|k| format!("'{}'", k.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "{SYMBOL_SELECT_PREFIX}
             FROM edges e JOIN symbols s ON s.id = e.src_id
             WHERE e.dst_id = ?1 AND e.kind IN ({kind_list})
             ORDER BY s.rank DESC, s.name ASC"
        ))?;
        let rows = stmt.query_map(params![sym], row_to_symbol)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Bounded BFS both directions via recursive CTE. Returns (symbol, depth),
    /// ordered by rank DESC. The center (depth 0) is excluded from results.
    pub fn neighborhood(&self, sym: SymbolId, depth: u8) -> Result<Vec<(Symbol, i64)>> {
        // Pattern from 02-data-model.md §5. We keep the MIN(depth) per node so a
        // symbol reachable via multiple paths reports its shortest distance.
        let mut stmt = self.conn.prepare(&format!(
            "WITH RECURSIVE nb(id, depth) AS (
                SELECT ?1, 0
                UNION
                SELECT e.dst_id, nb.depth + 1 FROM edges e JOIN nb ON e.src_id = nb.id
                  WHERE nb.depth < ?2
                UNION
                SELECT e.src_id, nb.depth + 1 FROM edges e JOIN nb ON e.dst_id = nb.id
                  WHERE nb.depth < ?2
             )
             SELECT {SYMBOL_COLS_S}, MIN(nb.depth) AS d
             FROM nb JOIN symbols s ON s.id = nb.id
             WHERE nb.id <> ?1
             GROUP BY s.id
             ORDER BY s.rank DESC, s.name ASC"
        ))?;
        let rows = stmt.query_map(params![sym, depth as i64], |r| {
            let symbol = row_to_symbol(r)?;
            // depth column is the last selected column (after the 15 symbol cols).
            let d: i64 = r.get(15)?;
            Ok((symbol, d))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ── Edge resolution ─────────────────────────────────────────────────────

    /// Build the `edges` table from `raw_refs` by name resolution.
    ///
    /// Resolution policy (documented decision):
    /// - For each raw_ref, find symbols whose `name == target_name`.
    /// - Exactly one match → insert a resolved edge (`resolved = 1`).
    /// - Multiple matches (ambiguous) → insert an edge to **every** candidate
    ///   with `resolved = 0`. We keep ambiguous edges (rather than dropping
    ///   them) so the graph stays connected for neighborhood/who_calls; the
    ///   `resolved = 0` flag lets consumers discount or filter them later.
    /// - Zero matches → skipped (nothing to point at).
    /// - Self-edges (src == dst) are skipped.
    ///
    /// Clears `edges` first so the build is idempotent. Returns the count of
    /// **resolved** (uniquely matched) edges.
    pub fn resolve_edges(&mut self) -> Result<usize> {
        let generation = self.generation()?;
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM edges", [])?;

        // Collect raw refs first to avoid holding a read statement open while we
        // run grouped lookups + inserts.
        let raw: Vec<(SymbolId, String, String)> = {
            let mut stmt = tx.prepare("SELECT src_id, target_name, kind FROM raw_refs")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, SymbolId>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut resolved_count = 0usize;
        {
            // UNIQUE(src_id, dst_id, kind) means duplicate refs collapse; OR
            // IGNORE makes that a no-op instead of an error.
            let mut insert_edge = tx.prepare(
                "INSERT OR IGNORE INTO edges(src_id, dst_id, kind, resolved, generation)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;

            for (src_id, target_name, kind) in raw {
                // Candidate def symbols sharing the referenced name.
                let candidates: Vec<SymbolId> = {
                    let mut stmt = tx.prepare("SELECT id FROM symbols WHERE name = ?1")?;
                    let rows = stmt.query_map(params![target_name], |r| r.get(0))?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };

                match candidates.as_slice() {
                    [] => {} // unresolved name, nothing to link
                    [dst] => {
                        if *dst != src_id {
                            let n = insert_edge
                                .execute(params![src_id, dst, kind, 1i64, generation])?;
                            // Only count it as resolved if a row was actually
                            // inserted (dedup may collapse repeats).
                            resolved_count += n;
                        }
                    }
                    many => {
                        for dst in many {
                            if *dst != src_id {
                                insert_edge
                                    .execute(params![src_id, dst, kind, 0i64, generation])?;
                            }
                        }
                    }
                }
            }
        }

        tx.commit()?;
        Ok(resolved_count)
    }

    /// Names of all symbols defined in a file (for incremental re-resolution).
    pub fn symbol_names_in_file(&self, file_id: FileId) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM symbols WHERE file_id = ?1")?;
        let rows = stmt.query_map(params![file_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Delete a file (and, via cascade, its symbols/skeletons/raw_refs/edges).
    /// Returns true if a row was removed. Used by `sync` for deleted files.
    pub fn delete_file(&mut self, path: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(n > 0)
    }

    /// Incrementally re-resolve edges after one file changed (H4/doc 10). Far
    /// cheaper than a full `resolve_edges`: only edges that could have changed
    /// are touched — outgoing edges from `file_id`'s symbols, plus any edge
    /// (from any file) whose target name is in `names` (the union of the names
    /// this file defined before and after the edit, so incoming edges to
    /// added/removed/renamed defs are rebuilt). Returns resolved-edge count.
    ///
    /// Note: the prior file row's symbols were cascade-deleted by `upsert_file`,
    /// so their old in/out edges are already gone; this only (re)inserts.
    pub fn resolve_edges_incremental(
        &mut self,
        file_id: FileId,
        names: &[String],
    ) -> Result<usize> {
        let generation = self.generation()?;
        let tx = self.conn.transaction()?;

        // Symbols currently in the dirty file (post-upsert, new ids).
        let affected: Vec<SymbolId> = {
            let mut stmt = tx.prepare("SELECT id FROM symbols WHERE file_id = ?1")?;
            let rows = stmt.query_map(params![file_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        // Drop outgoing edges from those symbols so a re-resolve is idempotent.
        for sid in &affected {
            tx.execute("DELETE FROM edges WHERE src_id = ?1", params![sid])?;
        }

        // Collect raw_refs to (re)resolve via INDEXED lookups (not a full scan):
        // (a) refs originating in the dirty file (idx_raw_refs_src), and
        // (b) refs targeting a name this file owns/owned (idx_raw_refs_target,
        // rebuilds incoming edges). Dedup the overlap. This keeps the edit hot
        // path O(refs touching this file) instead of O(all refs in the repo).
        let mut refs: Vec<(SymbolId, String, String)> = Vec::new();
        {
            let mut seen: std::collections::HashSet<(SymbolId, String, String)> =
                std::collections::HashSet::new();
            // (a) outgoing
            {
                let mut stmt =
                    tx.prepare("SELECT src_id, target_name, kind FROM raw_refs WHERE src_id = ?1")?;
                for &sid in &affected {
                    let rows = stmt.query_map(params![sid], |r| {
                        Ok((r.get::<_, SymbolId>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                    })?;
                    for row in rows {
                        let t = row?;
                        if seen.insert(t.clone()) {
                            refs.push(t);
                        }
                    }
                }
            }
            // (b) incoming (refs from anywhere targeting one of our names)
            {
                let mut stmt = tx.prepare(
                    "SELECT src_id, target_name, kind FROM raw_refs WHERE target_name = ?1",
                )?;
                for name in names {
                    let rows = stmt.query_map(params![name], |r| {
                        Ok((r.get::<_, SymbolId>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                    })?;
                    for row in rows {
                        let t = row?;
                        if seen.insert(t.clone()) {
                            refs.push(t);
                        }
                    }
                }
            }
        }

        let mut resolved_count = 0usize;
        {
            let mut insert_edge = tx.prepare(
                "INSERT OR IGNORE INTO edges(src_id, dst_id, kind, resolved, generation)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for (src_id, target_name, kind) in refs {
                let candidates: Vec<SymbolId> = {
                    let mut stmt = tx.prepare("SELECT id FROM symbols WHERE name = ?1")?;
                    let rows = stmt.query_map(params![target_name], |r| r.get(0))?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()?
                };
                match candidates.as_slice() {
                    [] => {}
                    [dst] => {
                        if *dst != src_id {
                            resolved_count +=
                                insert_edge.execute(params![src_id, dst, kind, 1i64, generation])?;
                        }
                    }
                    many => {
                        for dst in many {
                            if *dst != src_id {
                                insert_edge
                                    .execute(params![src_id, dst, kind, 0i64, generation])?;
                            }
                        }
                    }
                }
            }
        }

        tx.commit()?;
        Ok(resolved_count)
    }

    // ── Rank read/write ─────────────────────────────────────────────────────

    /// Overwrite symbols.rank from a map of symbol_id -> rank (one tx).
    pub fn write_ranks(&mut self, ranks: &HashMap<SymbolId, f64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE symbols SET rank = ?2 WHERE id = ?1")?;
            for (id, rank) in ranks {
                stmt.execute(params![id, rank])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// All edges (for PageRank in carto-core): (src_id, dst_id).
    pub fn all_call_edges(&self) -> Result<Vec<(SymbolId, SymbolId)>> {
        let mut stmt = self.conn.prepare("SELECT src_id, dst_id FROM edges")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All symbol ids (for PageRank seeding).
    pub fn all_symbol_ids(&self) -> Result<Vec<SymbolId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols ORDER BY id ASC")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ── Meta key/value (H3 head-rev bookkeeping etc.) ─────────────────────────

    /// Read a free-form meta value.
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Upsert a free-form meta value.
    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ── File co-change (H3) ───────────────────────────────────────────────────

    /// Replace the entire `file_cochange` table in one transaction. Producers
    /// (carto-git) recompute the full set from history, so this is a clean swap.
    pub fn replace_file_cochange(&mut self, rows: &[carto_model::FileCochangeRow]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM file_cochange", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO file_cochange(a_file, b_file, support, confidence, lift)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for r in rows {
                stmt.execute(params![r.a_file, r.b_file, r.support, r.confidence, r.lift])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Files that co-change with `file_id` at or above `min_lift`, strongest
    /// first, up to `limit`. Returns (file_id, lift).
    pub fn cochanging_files(
        &self,
        file_id: FileId,
        min_lift: f64,
        limit: i64,
    ) -> Result<Vec<(FileId, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT b_file, lift FROM file_cochange
             WHERE a_file = ?1 AND lift >= ?2
             ORDER BY lift DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![file_id, min_lift, limit], |r| {
            Ok((r.get::<_, FileId>(0)?, r.get::<_, f64>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Total co-change rows (0 = signal unavailable, e.g. no git history).
    pub fn file_cochange_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM file_cochange", [], |r| r.get(0))?)
    }

    /// All symbols defined in any of `file_ids`, ranked. Empty input → empty.
    pub fn symbols_in_files(&self, file_ids: &[FileId]) -> Result<Vec<Symbol>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = file_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql =
            format!("{SYMBOL_SELECT} WHERE s.file_id IN ({placeholders}) ORDER BY s.rank DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            file_ids.iter().map(|x| x as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), row_to_symbol)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Column list for `symbols`, aliased `s` (used inside joins). 15 columns in
/// `Symbol` field order; `row_to_symbol` reads indices 0..=14.
const SYMBOL_COLS_S: &str = "s.id, s.file_id, s.stable_key, s.name, s.fqn, s.kind, \
     s.signature, s.doc, s.start_byte, s.end_byte, s.start_row, s.end_row, \
     s.parent_id, s.rank, s.generation";

/// `SELECT <cols> FROM symbols s` prefix for join queries.
const SYMBOL_SELECT_PREFIX: &str = "SELECT s.id, s.file_id, s.stable_key, s.name, s.fqn, \
     s.kind, s.signature, s.doc, s.start_byte, s.end_byte, s.start_row, s.end_row, \
     s.parent_id, s.rank, s.generation";

/// Plain `SELECT ... FROM symbols` for single-table reads. We alias the table
/// `s` so `row_to_symbol` column ordering matches the join variants.
const SYMBOL_SELECT: &str = "SELECT s.id, s.file_id, s.stable_key, s.name, s.fqn, s.kind, \
     s.signature, s.doc, s.start_byte, s.end_byte, s.start_row, s.end_row, \
     s.parent_id, s.rank, s.generation FROM symbols s";

/// Map a `files` row (id..generation) into a `FileRec`.
fn row_to_file(r: &Row) -> rusqlite::Result<FileRec> {
    Ok(FileRec {
        id: r.get(0)?,
        path: r.get(1)?,
        lang: r.get(2)?,
        purpose: r.get(3)?,
        size_bytes: r.get(4)?,
        content_hash: r.get(5)?,
        generation: r.get(6)?,
    })
}

/// Map a symbol row (columns 0..=13, in `SYMBOL_COLS_S` order) into a `Symbol`.
/// `generation` is not selected by the shared column list; callers that need it
/// use full selects. For these read paths we re-derive generation as 0 is wrong,
/// so we fetch it explicitly below.
fn row_to_symbol(r: &Row) -> rusqlite::Result<Symbol> {
    let kind_str: String = r.get(5)?;
    Ok(Symbol {
        id: r.get(0)?,
        file_id: r.get(1)?,
        stable_key: r.get(2)?,
        name: r.get(3)?,
        fqn: r.get(4)?,
        kind: SymbolKind::from_str(&kind_str).unwrap_or(SymbolKind::Function),
        signature: r.get(6)?,
        doc: r.get(7)?,
        start_byte: r.get(8)?,
        end_byte: r.get(9)?,
        start_row: r.get(10)?,
        end_row: r.get(11)?,
        parent_id: r.get(12)?,
        rank: r.get(13)?,
        generation: r.get(14)?,
    })
}

/// Render the tier-1 skeleton text for a symbol: signature line plus an optional
/// trailing doc block. Falls back to the bare name when no signature is present.
fn render_skeleton(rs: &RawSymbol) -> String {
    let sig = rs.signature.as_deref().unwrap_or(&rs.name);
    match rs.doc.as_deref() {
        Some(doc) if !doc.is_empty() => format!("{sig}\n{doc}"),
        _ => sig.to_string(),
    }
}

/// Escape `%`, `_`, and `\` for a LIKE pattern using `\` as the escape char.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
