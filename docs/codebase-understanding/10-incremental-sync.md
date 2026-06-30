# 10 — Incremental Sync

The engine room. Cartograph's #1 operating constraint
([README](./README.md) §"Hard operating constraints") is that the index resyncs
*as the developer types and switches branches*. This doc specifies the mechanism:
tree-sitter incremental reparse, the delta pipeline, branch-switch resync, and the
staleness window. Every hypothesis spec links here for its producer's `apply()`
contract — this is the canonical reference for incremental work.

**Target:** single-file edit p95 **≤ 200 ms** from on-disk change to committed
delta. Branch switch is **bounded and reported**, not held to 200 ms (see
[11-benchmark-harness.md](./11-benchmark-harness.md) SO gate).

---

## 1. Why this constraint disqualifies LSP/embeddings as core

The 200 ms budget is not a nicety; it is the line that selects the core
mechanism. Two rejected approaches ([README](./README.md) "Rejected / deferred")
fail it structurally:

- **LSP-bridged index.** One stateful server *per language*. Cold start is
  seconds-to-minutes (the server re-analyzes the project on launch); a branch
  switch forces a re-warm. Resync latency is owned by an external process we
  cannot bound. Disqualified as *core*; allowed only as an opt-in enrichment that
  never sits in the resync path.
- **Embedding / vector-RAG.** Every edit changes the chunk it lands in, forcing a
  re-embed (network or local model, tens-to-hundreds of ms each) plus an index
  rebuild. Worse, it retrieves *similar-looking* text, not *connected* code, so it
  buys nothing on the topology queries (H2/H3) that justify the index. Disqualified
  as core; kept only as a fuzzy NL→code fallback.

Tree-sitter incremental reparse (+ SQLite WAL) is the only substrate that
re-computes a single-file edit in the low tens of milliseconds with **no per-
language daemon and no cold-start penalty** — which is exactly the
language-agnostic, lightweight-daemon mandate.

### Latency budget — single-file edit

End-to-end path from save to committed delta. Numbers are rough per-stage budgets
on a ~100k-LOC repo, mid-range laptop; they sum well under 200 ms to leave p95
headroom for GC pauses, cold caches, and large files.

| # | Stage | What happens | Budget (ms) |
|---|---|---|---|
| 1 | watch event | `notify` delivers an fs event | ~0–2 |
| 2 | debounce | coalesce burst of saves into one Delta (§2) | ~50 (fixed wait) |
| 3 | hash check | blake3 the file bytes, compare `files.content_hash` | ~1–3 |
| 4 | reparse | tree-sitter incremental reparse of the edited region (§3) | ~2–15 |
| 5 | re-extract | run tag queries over changed subtree → symbols/edges/skeletons | ~3–20 |
| 6 | re-rank region | localized PageRank refresh over affected neighborhood (§5) | ~5–25 |
| 7 | diff | structural diff old↔new for touched symbols (§5, H4) | ~2–15 |
| 8 | commit | one SQLite write txn, bump `generation` | ~3–10 |
| | | **compute total (excl. debounce)** | **~20–90** |

The 50 ms debounce is a *fixed coalescing wait*, not compute, and is what keeps
editor autosave storms from triggering N resyncs. Even counting it, a single edit
lands in ~70–140 ms typical, comfortably inside the 200 ms p95 gate. The dominant
variable cost is re-extraction over the changed region and the localized re-rank;
both are bounded by *edit size*, not *repo size* — the property that makes the
budget hold as repos grow.

---

## 2. File watching

`carto serve` ([01-architecture.md](./01-architecture.md) §1) owns two watchers.

### fs watcher (the `notify` crate)

- A recursive watch over the repo root, honoring `.carto/config.toml`
  `[index].exclude` and `.gitignore` so generated/vendor trees never wake the loop.
- Raw events (`create`/`modify`/`remove`/`rename`) are noisy: editors write temp
  files, `fsync`, rename-into-place, and touch mtimes. We do **not** act per event.

### Debounce → one Delta

```rust
// Coalesce a burst of fs events into a single Delta. config: [sync].debounce_ms (default 50).
struct Debouncer {
    pending: HashMap<PathBuf, ChangeKind>,  // last-writer-wins per path
    timer:   Option<Instant>,               // resets on each new event
}
// On each event: upsert path→kind, (re)arm timer for debounce_ms.
// On timer fire: drain `pending` into one Delta and hand to the sync loop.
```

A 50 ms quiet window collapses a save-storm (temp write → rename → mtime touch)
into **one** Delta carrying the final state per path. Last-writer-wins per path
means a file created-then-modified inside the window emits a single `Modified`.

### content_hash gate

Before any parse, each dirty path is blake3-hashed and compared to
`files.content_hash` ([02-data-model.md](./02-data-model.md) §4). **No-op saves
are dropped here** — formatter-on-save that produces identical bytes, an editor
"touch" with no content change, or a rename that round-trips. Dropping at the hash
gate is the cheapest possible exit (one hash, no parse, no txn) and is the first
defense against debounce storms (§9).

```rust
fn gate(path: &Path, store: &Store) -> Option<DirtyFile> {
    let bytes = fs::read(path).ok()?;
    let h = blake3::hash(&bytes);
    if store.content_hash(path) == Some(h) { return None; }  // no-op save: drop
    Some(DirtyFile { path: path.into(), bytes, hash: h })
}
```

### git HEAD watcher

A second watch on `.git/HEAD` (and the packed-refs path) detects branch switches,
`pull`, `rebase`, `reset`, and `checkout`. A HEAD move produces a special Delta
variant (`DeltaKind::HeadMove`, §6) rather than a per-file list; the pipeline
diffs the two trees itself. Controlled by `[sync].watch_git_head` (default true).

---

## 3. Tree-sitter INCREMENTAL reparse

This is the core enabler of US (Update Speed) and the reason the budget in §1
holds. Tree-sitter can reparse a file in time proportional to the **size of the
edit**, not the size of the file, by reusing the unchanged subtree of the prior
syntax tree.

### How it works

1. We keep the prior `Tree` for each open/recently-touched file in an in-memory
   LRU cache (`carto-parse`), keyed by `file_id`, alongside the prior source bytes.
2. On an edit we compute the byte-range delta (old text vs new text) and feed it to
   the old tree via `Tree::edit(&InputEdit { … })`. This shifts node positions and
   marks the spanning nodes dirty.
3. We call `Parser::parse(new_bytes, Some(&old_tree))`. Tree-sitter **reuses every
   subtree outside the edited span** and only re-parses the dirty region, producing
   a new `Tree` cheaply.

```rust
// Map a byte-range edit to a tree-sitter InputEdit.
fn to_input_edit(old: &[u8], new: &[u8]) -> InputEdit {
    let (start, old_end, new_end) = byte_range_diff(old, new); // common-prefix/suffix trim
    InputEdit {
        start_byte:   start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position:   byte_to_point(old, start),
        old_end_position: byte_to_point(old, old_end),
        new_end_position: byte_to_point(new, new_end),
    }
}

fn reparse(file: &mut CachedTree, new_bytes: &[u8]) -> Tree {
    let edit = to_input_edit(&file.bytes, new_bytes);
    file.tree.edit(&edit);
    let tree = file.parser.parse(new_bytes, Some(&file.tree)).expect("parse");
    file.bytes = new_bytes.to_vec();
    file.tree  = tree.clone();
    tree
}
```

We do **not** need the editor's keystroke-level edits. We reconstruct one
synthetic `InputEdit` by trimming the common prefix and suffix between old and new
bytes — a single contiguous changed span that is a safe over-approximation. Tree-
sitter still reuses everything outside that span. (When an editor *does* hand us
fine-grained edits, e.g. via a future LSP-style transport, we can feed them
directly for an even tighter dirty region; v1 uses the synthetic span.)

### Re-extraction is scoped to changed nodes

After reparse we walk `tree.changed_ranges(&old_tree, &new_tree)` and run the
language's tag query ([H1](./04-h1-recursive-repo-map.md) /
[H2](./05-h2-symbol-rank-topology.md)) only over the changed ranges plus their
enclosing top-level definitions. Symbols whose body span did not intersect a
changed range keep their rows untouched (only `generation` bumps if the file row
rewrites). This is what makes stage 5 in §1 bounded by edit size.

### What forces a full file reparse

- **No prior tree** — first sight of the file, cache eviction, or cold start.
- **Encoding / size change** — file flips encoding, becomes binary, or crosses
  `[index].max_file_bytes`; we re-detect and parse fresh (or drop to Tier-0).
- **Grammar change** — language re-detected (extension/shebang change).
- **Tree desync** — defensive: if `changed_ranges` looks implausible (e.g. the
  whole file), fall back to a full parse. Full parse of one file is still single-
  digit-to-tens of ms at typical sizes, so this is a safe floor, not a cliff.

---

## 4. The Delta and the serialized sync loop

All `apply()` calls are serialized through **one writer task**
([01-architecture.md](./01-architecture.md) §5) so producer ordering holds and the
write transaction is coherent.

```rust
/// One coalesced unit of change handed to the pipeline.
pub struct Delta {
    pub generation_base: u64,        // generation this delta was computed against
    pub kind: DeltaKind,
    pub dirty: Vec<DirtyFile>,       // post-hash-gate; never empty for FileEdit
    pub removed: Vec<PathBuf>,       // deleted/renamed-away paths
}

pub enum DeltaKind {
    FileEdit,                        // normal edit/create/delete burst (§1–3)
    HeadMove { from: Oid, to: Oid }, // branch switch / pull / rebase (§6)
}

pub struct DirtyFile {
    pub path:  PathBuf,
    pub bytes: Vec<u8>,
    pub hash:  blake3::Hash,
    pub tree:  Option<Tree>,         // reused prior tree for incremental reparse, if cached
}
```

### The loop

```rust
// Single-threaded writer. Producers run in Stage order: Parse → Rank → History → Diff.
fn sync_loop(rx: Receiver<Delta>, store: &Store, producers: &[Box<dyn Producer>]) {
    for delta in rx {                                  // one delta at a time, in order
        let mut txn = store.begin_write();             // ONE write txn per delta (WAL)
        for p in producers.iter() /* sorted by stage() */ {
            p.apply(&delta, &mut txn)?;                 // §5 per-producer work
        }
        let g = txn.bump_generation();                 // monotonic; published on commit
        txn.commit();                                  // readers see g atomically (§7)
        record_staleness(&delta, g);                   // for the benchmark (§7)
    }
}
```

One delta → one transaction → one `generation` bump. Readers never observe a
partially-applied delta (§7–8). Producer dispatch is just the `Producer::apply`
trait ([01-architecture.md](./01-architecture.md) §3) called in `Stage` order.

---

## 5. Per-producer incremental work

This is the canonical reference the hypothesis docs link to for `apply()`. Each
runs inside the single write transaction of §4, in `Stage` order.

### Parse.apply (Stage 0, [H1](./04-h1-recursive-repo-map.md) + [H2](./05-h2-symbol-rank-topology.md))

Per dirty file:

1. Incremental reparse (§3) using the cached prior tree when present.
2. **Delete-and-reinsert** the file's `symbols`, `edges` (where `src` ∈ file), and
   `skeletons` ([02-data-model.md](./02-data-model.md) §4). `ON DELETE CASCADE`
   cleans children.
3. **Preserve stable_key handles.** Re-insert reuses each symbol's `stable_key`
   ([02-data-model.md](./02-data-model.md) §3), so the integer `symbols.id` may
   change but the agent-facing handle survives. A symbol present before and after
   keeps its key; a new symbol gets a fresh key; a vanished symbol's key disappears
   (and is flagged to Diff as `removed`).
4. Recompute and write `files.content_hash`, `size_bytes`, and the row
   `generation`.

For `removed` paths: cascade-delete the `files` row; its symbols/edges/skeletons
go with it.

### Rank.apply (Stage 1, [H2](./05-h2-symbol-rank-topology.md))

A full PageRank over 50k–200k edges per keystroke is far too slow for the §1
budget (iteration over the whole graph is tens-to-hundreds of ms). v1 uses a
**localized refresh with periodic full reconciliation**:

- **On each delta:** recompute rank approximately over the **affected
  neighborhood** — the changed symbols plus their in/out neighbors out to a small
  bounded radius (default 2 hops) — holding the rest of the graph's ranks fixed as
  boundary conditions. This is a few hundred nodes at most, so it fits the ~5–25 ms
  stage-6 budget. Symbols outside the neighborhood keep their prior `symbols.rank`.
- **Mark-dirty:** the delta records that the global ranks are now approximate.
- **On idle** (no pending deltas for a short window): run a full PageRank to
  reconcile drift, in the background, committed as its own delta with a
  `generation` bump.

**Drift tradeoff (stated):** between full recomputes, centrality of nodes far from
recent edits can be slightly stale — acceptable because (a) ranking is used to
*order/truncate* results, not for correctness, and (b) the edits that move a node's
rank most are exactly the local ones the neighborhood refresh already catches. The
idle full pass bounds accumulated drift. This is the **chosen v1 approach**; a
true incremental-PageRank algorithm is an open question (§10).

### History.apply (Stage 2, [H3](./06-h3-impact-radius.md))

Append-only ([02-data-model.md](./02-data-model.md) §4) — `cochange` accrues,
never rewritten in place.

- **Normal edit:** the working tree changed but no commit was created, so there is
  **no new history to walk** — History.apply is a no-op on a `FileEdit` delta.
- **New commits** (commit/amend, or `pull` fast-forward): walk only the commits
  **between `meta.head_rev` and the new HEAD** — never the full log. For each new
  commit, attribute its changed hunks to symbols and accumulate co-change pairs,
  then advance `meta.head_rev`.
- **Branch switch:** walk the **symmetric difference** of commits between old and
  new HEAD (`git rev-list old...new`) so co-change reflects the commits unique to
  the branch now checked out, without re-mining shared history.

### Diff.apply (Stage 3, [H4](./08-h4-semantic-diff-hydration.md))

Structural diff of **old vs new snapshot for the touched symbols only**:

- Compare each touched symbol's prior AST shape (from the cached prior tree /
  prior rows) against the new one and emit `ast_diffs` rows
  (`added`/`removed`/`renamed`/`signature`/`body`/`moved`,
  [02-data-model.md](./02-data-model.md) §2). This is what `diff_context`
  ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md)) reads — H4 resends a
  structural delta instead of re-sending file text.
- Rename detection here ([02-data-model.md](./02-data-model.md) §3) emits the
  key-remap row so a `removed` old key + `added` new key that are structurally the
  same symbol are reconciled, and co-change history follows the rename.

---

## 6. Branch switch

A branch switch (HEAD move) can change hundreds of files at once. The pipeline
treats it as one large Delta, not many small ones.

1. The HEAD watcher (§2) emits `DeltaKind::HeadMove { from, to }`.
2. `carto-git` computes the changed path set with one diff
   (`git diff --name-status from to`); those paths become the `dirty`/`removed`
   sets.
3. **Bounded-parallel reparse.** Parsing is per-file independent, so dirty files
   are reparsed (and re-extracted) across a bounded worker pool
   (`min(num_cpus, N)`), producing per-file row batches. **No prior tree is reused
   across a branch switch** — content can differ arbitrarily — so these are full
   parses.
4. **Single serialized write.** The per-file batches are applied in **one write
   transaction** through the same single writer (§4), preserving producer ordering
   and atomicity. Only the parse/extract fan-out is parallel; the commit is serial.
5. History.apply walks the symmetric-difference commits (§5); Rank.apply does one
   neighborhood refresh over the union of changed symbols (and schedules an idle
   full pass, since a branch switch is a large perturbation).

**Latency:** larger than a single edit and proportional to the number of changed
files. It is **bounded and REPORTED, not required to be ≤ 200 ms** — the SO
(switch overhead) gate in [11-benchmark-harness.md](./11-benchmark-harness.md)
measures and bounds it. During the switch, tools keep serving the *previous*
committed generation (§8) until the new delta commits atomically.

---

## 7. The staleness window

**Definition:** the staleness window is the elapsed ms between a file changing on
disk and the delta that incorporates it committing (its `generation` becoming
visible to readers). It is the debounce wait (§1 stage 2) plus compute (stages
3–8).

- **Observable.** Every tool response carries `generation`
  ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md) design rules). A client (and
  the benchmark) can compare the `generation` it received against the current head
  to quantify how stale a read was, and `record_staleness` (§4) logs the per-delta
  window for the harness.
- **WAL snapshot guarantee — reads never tear.** Because the writer commits one
  delta per transaction in WAL mode ([02-data-model.md](./02-data-model.md),
  [01](./01-architecture.md) §5), a reader always sees a *whole* generation: either
  fully before or fully after a given delta, never a half-applied mix of a file's
  old symbols and another file's new ones.

The staleness window is the honest cost of the design — we trade a small, bounded,
*measured* lag for never blocking the editor or the reader. The 200 ms target *is*
the p95 ceiling on this window for a single-file edit.

---

## 8. Consistency for readers

- **Snapshot reads.** Each tool call opens a WAL read transaction at the latest
  committed generation ([01-architecture.md](./01-architecture.md) §5). An in-flight
  resync (writer mid-transaction) is invisible: the reader sees the prior
  generation until the writer commits, then the next read sees the new one.
- **Pinned plans.** A long [H5](./09-h5-adaptive-context-budgeter.md) plan
  (`plan_retrieval`) holds **one** read transaction open for the whole multi-step
  retrieval, pinning a single generation. Every `outline`/`neighborhood`/`expand`
  in that plan reads the same consistent snapshot, so the hydrated context cannot
  mix symbols from two different index states even if the developer keeps typing.
  The plan reports the generation it was pinned to.
- **No reader ever blocks the writer**, and the writer never blocks a reader — WAL
  gives the single-writer/many-reader split for free.

---

## 9. Failure handling

- **Parse error on a dirty file.** A syntactically broken file (mid-edit) still
  parses under tree-sitter as a tree containing `ERROR` nodes. **v1 decision: keep
  prior symbols for the broken regions.** We extract every symbol whose subtree
  parsed cleanly and, for regions under an `ERROR` node, **retain the prior rows**
  rather than deleting them — so a half-typed function does not make its callers
  vanish from the graph mid-keystroke. Only if a file becomes wholly unparseable
  (or its language is unknown) does it **drop to a Tier-0 entry**
  ([01-architecture.md](./01-architecture.md) §7): path + size + best-effort
  purpose, no symbols. The next clean save re-extracts normally.
- **Watcher overflow.** `notify` can drop/overflow events under heavy churn (large
  `git checkout`, bulk codegen). On an overflow signal we **fall back to a periodic
  rescan**: walk the tree, hash every file, and synthesize a Delta from the hash
  diff against `files.content_hash`. Slower but correct and self-healing; the next
  steady state returns to event-driven sync.
- **Debounce storms (bulk operations).** `git rebase`, `git checkout`, or a
  formatter sweeping the repo fire thousands of events. Coalescing strategy, in
  layers: (1) the 50 ms debounce collapses bursts per path; (2) the content_hash
  gate (§2) drops every file the bulk op left byte-identical; (3) a HEAD move is
  recognized as **one** `HeadMove` delta (§6) instead of N `FileEdit`s, so a branch
  switch is a single batched resync, not a storm; (4) if events still outrun the
  loop, the overflow path above takes over. The net effect: bulk operations
  collapse to at most one large, batched, reported delta.

---

## 10. Open questions / risks

- **True incremental PageRank.** v1's neighborhood-refresh + idle-full
  reconciliation (§5) is pragmatic but approximate. A streaming/incremental
  centrality algorithm (e.g. push-based personalized PageRank, or maintaining a
  residual) could remove the drift window — worth prototyping if rank staleness
  shows up in [11-benchmark-harness.md](./11-benchmark-harness.md) accuracy
  numbers.
- **Synthetic InputEdit precision.** Reconstructing one contiguous span from a
  prefix/suffix trim (§3) over-approximates multi-region edits (e.g. a
  find-and-replace across a file), forcing a larger dirty region than necessary.
  Usually fine; pathological cases degrade toward a full file parse, which is still
  cheap. Real editor edit-streams would tighten this.
- **Tree cache memory.** The prior-`Tree` LRU (§3) trades memory for reparse speed.
  Cache size vs. hit-rate under realistic multi-file editing needs tuning; a cold
  cache turns an incremental reparse into a full one (still within budget for one
  file, but it widens the §1 distribution).
- **ERROR-node retention correctness.** Keeping prior symbols for broken regions
  (§9) can briefly show a stale signature for a function the developer is actively
  changing. Acceptable for a transient mid-edit state, but the boundary between
  "broken region" and "intentionally deleted" is heuristic — needs validation
  against real editing traces.
- **Branch-switch tail latency.** SO is bounded but unbounded *in principle* for a
  switch that rewrites the whole repo (e.g. a generated-code branch). The
  bounded-parallel parse (§6) helps, but the worst case is a near-cold rebuild;
  [11-benchmark-harness.md](./11-benchmark-harness.md) must report the tail, and we
  may cap reported SO and finish the rest in the background.
- **Rename vs. delete+add ambiguity.** Diff.apply's rename reconciliation (§5)
  relies on structural similarity; a rename that also heavily edits the body may be
  misclassified as delete+add, resetting a symbol's co-change history. Tunable, but
  a known sharp edge.
