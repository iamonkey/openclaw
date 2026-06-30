# Cartograph Long-Horizon / Freshness Report

Corpus: `/tmp/claude-0/-home-user-openclaw/05d9dc16-f32f-561a-9d08-9b47f1a4c9ce/scratchpad/openclaw-src-copy`

## Scale (large real codebase)

| metric | value |
|---|---|
| TS files | 4776 |
| symbols indexed | 28386 |
| edges resolved | 63228 |
| cold full build | 21446 ms |
| edits sampled | 300 |

## 1. Freshness — incremental single-file resync latency (SO gate)

| metric | value | gate |
|---|---|---|
| resync p50 | 14.65 ms | - |
| resync p95 | 37.67 ms | **PASS** (<= 200 ms) |
| resync p99 | 86.66 ms | - |
| resync max | 126.81 ms | - |
| scoped-diff compute (mean) | 1.74 ms | - |

Cold rebuild was 21446 ms for the whole repo; an incremental edit resyncs in the latencies above — the ratio is the point: a single edit touches ~1/4776 of the work.

## 2. Cache economics — context refresh per edit

To keep the agent's context true to the code after each edit:

| approach | tokens/edit (mean) | total over run | notes |
|---|---|---|---|
| Naive resend changed file | 1778 | 533413 | large; **invalidates cache** from edit point |
| Cartograph H4 structural diff | 10.0 | 2990 | tiny; **append-only, cache prefix preserved** |

- **Per-edit refresh tokens cut ~178x** (resend 1778 -> diff 10.0).
- Illustrative refresh cost over 300 edits at $3/M input tokens: naive $1.6002 vs diff $0.0090 (input-only, conservative — excludes the cache-invalidation penalty the resend also incurs).

> The diff number is the load-bearing one for prompt caching: structural diffs append a handful of tokens and leave the cached context prefix intact, so subsequent turns stay cache-warm. Resending changed files both costs more tokens and breaks the cache from the change point.

> Caveats: single repo (openclaw TS); token estimate is ceil(bytes/4); the cache-cost figure is illustrative with a stated per-MTok rate, not a quote; the cache-invalidation penalty on the naive side is not modeled, so the real gap is larger than shown.
