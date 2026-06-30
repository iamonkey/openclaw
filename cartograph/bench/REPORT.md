# Cartograph Benchmark Report (H1-H5)

Corpus: `/home/user/openclaw/cartograph/bench/corpus`

## Corpus and index

| metric | value |
|---|---|
| files (TypeScript) | 40 |
| lines of code | 2178 |
| full-corpus tokens (est) | 16946 |
| symbols indexed | 211 |
| edges resolved | 612 |
| index build time | 249.0 ms (full build) |

## Headline: token cost vs naive grep + full-file

- **Overall token reduction: 94.8%** (195387 baseline -> 10237 Cartograph tokens across 14 tasks)
- Mean per-task reduction: 94.9%
- Context precision: Cartograph reads 5.2% of what naive reads

## Task resolution (localization via H5 planner)

Answer present in the H5 plan's budget-constrained context (budget 2000 tokens):

- Cartograph symbol-localization: 12/14
- Cartograph file-localization: 14/14
- Baseline file-localization (grep): 14/14

## Sync overhead (SO)

| metric | value | gate |
|---|---|---|
| query latency p50 | 1.81 ms | - |
| query latency p95 | 3.46 ms | informational |
| full index build | 249.0 ms | (incremental apply is a later milestone) |

## Per-task detail

| task | class | baseline tok | carto tok | reduction | sym hit | file hit |
|---|---|---|---|---|---|---|
| where-pagination | where-is | 7161 | 243 | 96.6% | yes | yes |
| where-config-load | where-is | 7949 | 338 | 95.7% | yes | yes |
| where-validation | where-is | 14933 | 467 | 96.9% | yes | yes |
| where-repo-create | where-is | 12551 | 783 | 93.8% | yes | yes |
| where-status-transitions | where-is | 14372 | 708 | 95.1% | yes | yes |
| where-search-index | where-is | 15179 | 649 | 95.7% | yes | yes |
| where-error-status-map | where-is | 14816 | 468 | 96.8% | yes | yes |
| where-router-dispatch | where-is | 12750 | 565 | 95.6% | yes | yes |
| impact-paginate-callers | what-breaks | 16062 | 650 | 96.0% | yes | yes |
| impact-task-interface | what-breaks | 16946 | 1997 | 88.2% | yes | yes |
| impact-result-type | what-breaks | 15404 | 1679 | 89.1% | yes | yes |
| impact-domain-error | what-breaks | 13958 | 306 | 97.8% | yes | yes |
| bug-pagination-offbyone | bug-localize | 16554 | 470 | 97.2% | no | yes |
| bug-pagination-via-list | bug-localize | 16752 | 914 | 94.5% | no | yes |

> Token estimates use ceil(bytes/4), identical on both sides, so ratios are apples-to-apples.
