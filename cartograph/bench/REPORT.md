# Cartograph Benchmark Report (H1+H2 vertical slice)

Corpus: `/home/user/openclaw/cartograph/bench/corpus`

## Corpus and index

| metric | value |
|---|---|
| files (TypeScript) | 40 |
| lines of code | 2178 |
| full-corpus tokens (est) | 16946 |
| symbols indexed | 211 |
| edges resolved | 612 |
| index build time | 288.4 ms (full build) |

## Headline: token cost vs naive grep + full-file

- **Overall token reduction: 96.6%** (195387 baseline -> 6566 Cartograph tokens across 14 tasks)
- Mean per-task reduction: 96.8%
- Context precision: Cartograph reads 3.4% of what naive reads

## Task resolution (localization)

- Cartograph symbol-localization@10: 8/14
- Cartograph file-localization@10: 12/14
- Baseline file-localization (grep): 14/14

## Sync overhead (SO)

| metric | value | gate |
|---|---|---|
| query latency p50 | 1.74 ms | - |
| query latency p95 | 2.45 ms | informational |
| full index build | 288.4 ms | (incremental apply is a later milestone) |

## Per-task detail

| task | class | baseline tok | carto tok | reduction | sym hit | file hit |
|---|---|---|---|---|---|---|
| where-pagination | where-is | 7161 | 153 | 97.9% | yes | yes |
| where-config-load | where-is | 7949 | 259 | 96.7% | yes | yes |
| where-validation | where-is | 14933 | 273 | 98.2% | no | no |
| where-repo-create | where-is | 12551 | 236 | 98.1% | no | yes |
| where-status-transitions | where-is | 14372 | 273 | 98.1% | no | yes |
| where-search-index | where-is | 15179 | 273 | 98.2% | no | no |
| where-error-status-map | where-is | 14816 | 254 | 98.3% | yes | yes |
| where-router-dispatch | where-is | 12750 | 253 | 98.0% | yes | yes |
| impact-paginate-callers | what-breaks | 16062 | 547 | 96.6% | yes | yes |
| impact-task-interface | what-breaks | 16946 | 1701 | 90.0% | yes | yes |
| impact-result-type | what-breaks | 15404 | 1218 | 92.1% | yes | yes |
| impact-domain-error | what-breaks | 13958 | 642 | 95.4% | yes | yes |
| bug-pagination-offbyone | bug-localize | 16554 | 231 | 98.6% | no | yes |
| bug-pagination-via-list | bug-localize | 16752 | 253 | 98.5% | no | yes |

> Token estimates use ceil(bytes/4), identical on both sides, so ratios are apples-to-apples.
