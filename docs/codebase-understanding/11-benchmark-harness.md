# 11 — Benchmark Harness and Evaluation

This is how we prove Cartograph works. The harness lives in the `carto-bench`
crate ([01-architecture.md](./01-architecture.md) §2) and is stood up **first**,
before any hypothesis lands, so every change is measured against the grep
baseline from day one. A hypothesis is only "in" when it moves the frontier
defined here.

The project's whole claim is "maximum accuracy, minimum tokens, interactive
resync." This doc operationalizes those three into measurable gates: **TRA**
(accuracy), **TC** (tokens), **SO** (sync). The first two define a frontier; the
third is a hard gate on it.

---

## 1. Purpose and the headline view: the efficiency frontier

We do not report a single score. We report a **frontier**: Task Resolution
Accuracy (TRA, higher better) on the y-axis against Token Cost (TC, lower
better) on the x-axis. Each point is one *configuration* (baseline, or a
Cartograph ablation — see §7). The frontier is the upper-left envelope: the set
of configs that no other config beats on both accuracy and cost simultaneously.

```
TRA ▲
1.0 │                          ● H1+H2+H3+H5  (target: up-and-left of baseline)
    │                  ● H1+H2
    │           ● H1
    │      ○ baseline (grep + full-file)
    │   ✗ H1+H2+H3+H5 (FAILS SO gate: greyed out, excluded from frontier)
    └───────────────────────────────────────────▶ TC (input tokens / resolved task)
        less tokens ◀                    ▶ more tokens
```

**Synchronization Overhead (SO) is an operational GATE, not a frontier axis.**
A configuration that wins the TRA/TC frontier but cannot incrementally resync a
single-file edit under the p95 budget **fails the interactive requirement and is
excluded from the frontier entirely** — it is plotted greyed-out and never
counted as winning. This is non-negotiable: per [README](./README.md) hard
constraint #1, anything that cannot recompute at interactive speed is
disqualified as a *core* mechanism. The frontier ranks only SO-passing configs.

```
config wins frontier  ⟺  (no other config dominates it on TRA∧TC)
                          AND  SO_p95_single_edit ≤ 200ms        ← the gate
```

A headline result is a triple per config: `(TRA, TC, SO_pass)`. We publish all
three; the marketing number is "Cartograph resolves X% of tasks at Y% of the
baseline's token cost while resyncing in <200ms."

---

## 2. The control: "Naive Grep + Full-File" baseline

Every Cartograph number is reported **both** as an absolute and as a ratio
against this baseline. It must be reproducible and a *fair* comparison: same
agent, same model, same tasks, same turn budget — only the tool layer differs.

The baseline agent has exactly two tools and **no index**:

| Tool | Definition | Notes |
|---|---|---|
| `read_file(path, [start_line], [end_line])` | Returns raw file bytes. With no range, returns the **whole file** (this is the "full-file" cost driver). | Range read allowed but agent is not prompted to use it — it discovers ranges only via grep output. |
| `grep(pattern, [path_glob])` | `ripgrep` over the corpus: `rg --line-number --no-heading --color=never <pattern> [glob]`. Returns matching lines with `path:line:text`. | Fixed `rg` version pinned in `carto-bench` (vendored or `--locked` install) so output format is stable. |

Baseline rules (frozen in `carto-bench/baseline.rs`):

- Same system prompt scaffold as the Cartograph agent **minus** the
  Cartograph-tool descriptions, **plus** the two tools above. The task text is
  byte-identical across arms.
- Same model, same temperature (0), same max turns, same per-turn token cap.
- `grep` returns at most `N` lines (default 200) then `truncated`; the agent may
  refine. This mirrors a real coding agent and prevents a single mega-grep from
  dumping the repo into context.
- No symbol awareness, no ranking, no cross-file graph — the agent reconstructs
  structure by reading files. This is the honest "what a competent agent does
  today without us" control.

The baseline is itself a config on the frontier (the `○` point). Cartograph
must land **up and to the left** of it.

---

## 3. Corpus

One **real, polyglot, OSS monorepo, ≥100k LOC, with a passing test suite**. The
passing suite is load-bearing: it makes bug-fix tasks machine-gradeable
(pass@1) and forces genuine language-agnosticism because the tests span the
repo's languages.

Selection criteria:

1. **Polyglot, ≥100k LOC.** At least 3 languages with first-class tree-sitter
   grammars (exercises [carto-parse](./01-architecture.md) across grammars and
   the unknown-language degradation path).
2. **Green test suite at the pinned commit**, runnable in a pinned Docker image
   in <30 min, with a stable per-test pass/fail signal (enables pass@1 grading
   and flaky-test quarantine, §9).
3. **Deep, active git history** (thousands of commits, many merged bugfix PRs).
   This is the ground-truth mine for tasks (§4) *and* the substrate H3 co-change
   needs ([06-h3-impact-radius.md](./06-h3-impact-radius.md)); a shallow repo
   starves both.
4. **Permissive license** (Apache-2.0 / MIT) so we can vendor a pinned snapshot.

Candidate properties (pick one; do not over-fit the harness to its quirks):

| Candidate profile | Languages | Why it fits |
|---|---|---|
| Large cloud-native Go monorepo with TS/Python tooling | Go, TypeScript, Python, shell | Huge, fast-moving git history; rich PR-with-fix corpus; well-tested. |
| Polyglot data/ML platform | Python, Java/Scala, TS, SQL | Cross-language edges stress H2; strong CI test gate. |
| Editor/IDE-class app monorepo | TypeScript, Rust, C++ | Multi-language with real cross-boundary calls; very active history. |

**Pin a specific commit.** Record in `carto-bench/corpus.lock`:

```toml
[corpus]
name        = "example-monorepo"
repo        = "https://github.com/org/example-monorepo"
commit      = "9f3c1ab2e7d4..."   # exact SHA, never a branch/tag
loc         = 142_318             # measured via `tokei` at this commit, checked in
languages   = ["go", "typescript", "python", "shell"]
docker_image = "ghcr.io/org/example-monorepo-ci@sha256:..."  # frozen test env
test_cmd    = "make test"
test_runtime_p50_min = 18
```

All tasks (§4) are mined against **this exact SHA** so ground truth is
deterministic. Bumping the corpus commit is a deliberate, reviewed event (it
re-mines tasks and re-baselines the frontier).

---

## 4. Metric 1 — Task Resolution Accuracy (TRA) [PRIMARY]

TRA is the headline accuracy number: fraction of held-out tasks the agent
resolves correctly. Tasks fall in three classes, each graded by an objective,
ground-truth scorer. TRA is reported overall **and** per class.

### Task sourcing (ground truth from real history)

Tasks are *mined*, not authored, so the answer key is real:

- **Class A (bug localize-and-patch)** — mined from **merged bugfix PRs** at or
  before the pinned SHA: the PR's pre-fix parent is the task state; the PR's
  changed test files give the grading tests; the issue/PR body (sans the fix and
  sans file/line hints) is the task prompt. SWE-bench style.
- **Class B (where is X implemented)** — mined from PRs/issues whose resolution
  touched a known symbol set; the task asks "where is `<behavior>` implemented",
  ground truth = the set of files/symbols the PR actually touched.
- **Class C (what breaks if I change Y)** — mined from PRs where a change to one
  symbol forced changes elsewhere in the **same** PR; ground truth = the impacted
  symbol/file set the PR co-modified (optionally widened by the static call
  graph at the pinned SHA, recorded separately).

All mining is scripted (`carto-bench mine`), reviewed, then **frozen** into
`tasks.jsonl` with the answer key. A held-out split is reserved; tuning never
touches it (§9).

```jsonc
// tasks.jsonl row
{
  "id": "A-0421",
  "class": "patch",                 // patch | localize | impact
  "prompt": "Pagination returns a duplicate row when page_size == 1.",
  "base_commit": "9f3c1ab2e7d4...",
  "grading": {
    "tests": ["pkg/page/page_test.go::TestPageSizeOne"],   // class A
    "gold_symbols": ["pkg/page/page.go#Paginator/next:method"], // class B/C
    "k": 5
  }
}
```

### Scoring per class

| Class | Metric | Definition |
|---|---|---|
| **A — patch** | **pass@1** | Apply the agent's single patch to `base_commit`; run the PR's grading tests in the frozen Docker image. Pass = all listed tests green **and** no previously-green test regresses. 1 attempt, no test feedback loop. |
| **B — localize** | **localization@k** | The agent emits a ranked list of symbols/files. Score = 1 if any gold symbol is in the top-k, else 0. Report @1 and @5. |
| **C — impact** | **precision/recall@k** | Agent emits a predicted impacted set; compare to gold set. `recall@k = |pred∩gold| / |gold|`, `precision@k = |pred∩gold| / |pred|`, reported with F1@k. |

```
TRA_overall = mean over all tasks of class-normalized score
            = mean( passA ∪ loc@5_B ∪ F1@5_C )   # each ∈ [0,1]
```

Pass/fail is binarized per task for the frontier's y-axis (A: pass; B: hit@k;
C: F1@k ≥ τ, τ pinned e.g. 0.6). The continuous scores are kept for analysis.

### Statistical signal

Target **≥150 tasks** total, roughly balanced (~60 A / ~50 B / ~40 C). At n=150
a 10-point TRA difference clears a two-proportion test at α=0.05 with adequate
power; we report **95% Wilson confidence intervals** on every TRA number and
**bootstrap CIs** on the per-task-paired baseline delta (same tasks both arms →
paired test, McNemar for the binarized A/B classes). No headline claim without
non-overlapping CIs or a significant paired test.

---

## 5. Metric 2 — Token Cost per resolved task (TC)

TC is the x-axis. It is the cost we are trying to crush.

**Definition:** total **input** tokens fed to the model to reach the first
correct resolution for a task, **including the tokens of failed turns** along
the way. Failed exploration is part of the cost — a tool that needs five wrong
turns is not cheap just because the final turn is small.

```
TC(task) = Σ over all turns until first-correct  ( input_tokens(turn) )
         = system_prompt + tool_schemas + Σ (prior tool results + model msgs)
```

- Tasks the agent never resolves are **not** averaged into the resolved-task TC
  (that would reward giving up); instead unresolved tasks count against TRA and
  their token spend is reported separately as `wasted_tokens`.
- Report `TC_mean` and `TC_median` over **resolved** tasks, plus the
  baseline-relative ratio `TC_ratio = TC_carto / TC_baseline` (paired per task).

**Context Precision** — how much of what we hauled in was actually used:

```
context_precision = tokens_used / tokens_retrieved
  tokens_retrieved = Σ token_est of all tool payloads returned to the agent
  tokens_used      = tokens of retrieved spans the agent cited / edited / that
                     overlap the gold answer span
```

Cartograph's lazy-hydration thesis predicts high context precision (we fetch the
skeleton, not the file); the baseline's full-file reads predict low precision.
This is the metric that most directly validates "minimum tokens."

### Deterministic token instrumentation

Token counts must be reproducible, not estimated post-hoc:

- The model is pinned (§7). Token counts come from the **provider's exact
  tokenizer** for that model, computed by `carto-bench` on every request/response
  *before* sending, and cross-checked against the API's returned `usage` field.
  Any mismatch fails the run (guards against silent tokenizer drift).
- `carto-bench` logs a per-turn ledger: `{turn, role, input_tokens,
  output_tokens, tool, retrieved_est, used_est}`. TC and context precision are
  recomputed from the ledger, never hand-rolled.
- Temperature 0 + seeded harness (§7) make the turn sequence — and thus TC —
  replayable bit-for-bit for a given model snapshot.

---

## 6. Metric 3 — Synchronization Overhead (SO) [THE GATE]

SO proves the interactive-resync constraint. It is measured on the live
[`carto serve`](./01-architecture.md) daemon over the pinned corpus, independent
of any agent. See [10-incremental-sync.md](./10-incremental-sync.md) for the
mechanism being measured.

| Sub-metric | What | How measured | Gate |
|---|---|---|---|
| **Single-edit latency** | Time from a one-file content change hitting the watcher to the delta committed and the bumped `generation` visible to readers. | Scripted edit harness applies a realistic edit (insert/modify a function body) to a sampled set of files spanning all languages; timestamp `fs_event → commit`. Report **p50 / p95 / p99** over ≥500 edits. | **p95 ≤ 200ms** ← frontier gate |
| **Branch-switch latency** | Full incremental resync after `git checkout` between two commits N apart, across the whole corpus. | Check out commit A, warm; `git checkout B`; measure HEAD-watch → fully-resynced. Report p50/p95 for small (≈10 files), medium (≈200), large (≈2000) diffs. | Bounded + reported (no hard cap; must not wedge). |
| **Daemon peak RAM** | Resident set of `carto serve` at steady state and at branch-switch peak. | RSS sampled @10Hz during the SO run; report steady + peak. | Reported; budget e.g. ≤1.5GB at 100k LOC. |
| **Staleness window** | Milliseconds during which a tool call can return an answer inconsistent with on-disk source (file changed, delta not yet committed). | Use the `generation` counter ([01](./01-architecture.md) §5): mutate a file, immediately poll a tool touching it, record the window where the response's `generation` predates the edit's committed delta. Report p50/p95 and **max**. | Reported; max must be < single-edit p99. |

```
SO_pass(config) ⟺ single_edit_p95 ≤ 200ms
```

The single-edit p95 gate is the **frontier gate** from §1. The other three are
reported and budgeted but do not gate the frontier (a config can be slow on
branch switch and still be "interactive" for typing). If a hypothesis makes the
index unsyncable within budget, it is rejected as a *core* mechanism regardless
of TRA/TC — exactly per the hard constraints.

---

## 7. Harness design

`carto-bench` isolates the single variable — **the tool layer** — and holds
everything else fixed.

```
┌──────────────────────── carto-bench run ─────────────────────────┐
│  fixed: model snapshot, temperature=0, seed, max_turns, prompt    │
│  swap:  TOOL LAYER                                                 │
│                                                                   │
│   tasks.jsonl ──▶ Agent (pinned model + fixed harness loop)       │
│                    │                                              │
│        ┌───────────┴───────────┐                                 │
│        ▼ arm = baseline        ▼ arm = cartograph                 │
│   [ read_file, grep ]      [ outline, expand, get_symbol,         │
│                              who_calls, neighborhood, impact,     │
│                              diff_context, plan_retrieval, … ]    │
│        │                        │  (carto serve over stdio MCP)   │
│        ▼                        ▼                                 │
│     resolution ──▶ class scorer ──▶ ledger (TRA, TC, ctx-prec)    │
└──────────────────────────────────────────────────────────────────┘
```

- **Same agent, same tasks, swap the tools.** The agent loop, model, system
  scaffold, and turn budget are identical across arms (§2). The *only* difference
  is which tools are registered. This is what makes the comparison causal.
- **Cartograph arm** drives the real `carto serve` MCP server
  ([03-mcp-tool-surface.md](./03-mcp-tool-surface.md)) over stdio JSON-RPC — the
  exact production path, not a mock — so we benchmark the shipping surface.
- **Determinism / replay:** temperature 0, a fixed RNG seed for any harness-side
  sampling, a pinned model snapshot ID recorded in the run manifest. For H5 runs,
  the planner's emitted **plan is recorded** ([H5](./09-h5-adaptive-context-budgeter.md))
  so a run is auditable and a regression can be diffed plan-by-plan. Provider
  nondeterminism is bounded by re-running each task `r` times (default r=3) and
  reporting the distribution; flaky resolutions are flagged.
- **Pinned model:** one provider model snapshot is the fixed agent for all
  frontier comparisons. Record exact snapshot ID in the run manifest; a model
  change re-baselines the whole frontier (do not compare across models). See the
  `claude-api` skill for current snapshot IDs when wiring the agent.

### Ablation matrix (attribute the gain per hypothesis)

The frontier is drawn from a ladder of configs so each hypothesis's contribution
is isolated. Each row is a full TRA/TC/SO run:

| Config | Tools enabled | Question it answers |
|---|---|---|
| `baseline` | grep + full-file | The control. |
| `H1` | `outline`, `expand`, `search_symbols` | Does tiered repo-map hydration alone beat grep? |
| `H1+H2` | + `get_symbol`, `who_calls`, `neighborhood` | Does the topology graph add accuracy/precision? |
| `H1+H2+H3` | + `impact` | Does co-change impact help Class C / patch localization? |
| `H1+H2+H3+H5` | + `plan_retrieval` (budgeter) | Does the planner improve tokens-per-task at equal TRA? |

(H4 `diff_context` is exercised by the SO and edit-driven task variants rather
than the static ablation ladder.) Each added stage must **move the frontier**
(up, left, or both) to justify its complexity — a stage that does not is
flagged for cut. H5's job specifically is to push **left** (same TRA, fewer
tokens).

---

## 8. Reporting

Every run emits a self-contained report (`carto-bench report`) plus machine JSON
for CI. Required artifacts:

1. **Frontier plot** — TRA vs TC scatter, one point per config, SO-failing
   configs greyed and annotated `SO FAIL`. The frontier envelope is drawn.
2. **Per-metric tables:**

   | config | TRA | TRA 95% CI | TC_med | TC_ratio | ctx_prec | SO p95 | SO gate |
   |---|---|---|---|---|---|---|---|
   | baseline | 0.41 | ±0.08 | 14.2k | 1.00 | 0.18 | — | n/a |
   | H1+H2 | 0.58 | ±0.08 | 6.1k | 0.43 | 0.51 | 88ms | PASS |
   | H1+H2+H3+H5 | 0.67 | ±0.07 | 3.9k | 0.27 | 0.63 | 142ms | PASS |

   (illustrative numbers, not results)

3. **Ablation table** — marginal ΔTRA and ΔTC for each added hypothesis vs the
   prior rung, with paired CIs.
4. **Per-task-class breakdown** — TRA split into patch / localize / impact, so a
   config strong on lookup but weak on patching is visible.
5. **SO panel** — single-edit p50/p95/p99, branch-switch by diff size, peak RAM,
   staleness p95/max, with the 200ms gate line drawn.

### CI integration

- A reduced **smoke set** (~25 tasks, single replicate) runs on PRs touching
  `carto-core`/producers/tools; the full ≥150-task suite runs nightly and on
  release tags.
- **Regression gates** fail the build if, vs the last green main run on the same
  pinned corpus+model: TRA drops > 3pts (outside CI), TC_ratio worsens > 10%, or
  **SO single-edit p95 > 200ms** (hard fail — the interactive gate is also a CI
  gate). Results are posted to the PR.
- The pinned corpus SHA, model snapshot, and `rg` version are asserted at run
  start; a mismatch aborts rather than silently producing incomparable numbers.

---

## 9. Threats to validity and mitigations

| Threat | Risk | Mitigation |
|---|---|---|
| **Train/test leakage** — base model memorized a public repo's fixes. | Inflates *both* arms' TRA; can mask that retrieval did nothing. | Prefer tasks from PRs after the model's training cutoff; run a **no-context probe** (model answers with zero tool access) — high no-context pass = memorized, quarantine that task. Report the leakage-adjusted TRA. |
| **Overfitting tool params to the corpus** — tuning `pagerank_iterations`, `min_lift`, budgets to this one repo. | Frontier looks great here, fails elsewhere. | Strict held-out split: tune on dev tasks only, report on held-out only. Freeze config before the held-out run. A second small corpus is used only as a generalization spot-check (§10), never for tuning. |
| **Grading flakiness** — flaky tests flip pass@1. | Noise read as signal; CI gate trips spuriously. | Quarantine flaky tests: each grading test must pass `Q` consecutive runs on the unmodified gold patch to be eligible; flaky ones are dropped from the task's grading set or the task is excluded. Replicate (r=3) and require majority. |
| **Single-corpus generalization** — one repo is not the world. | Headline claim over-generalizes. | Frame all headline numbers as "on `<corpus>@<sha>`." Keep a second, smaller polyglot corpus wired (no tasks tuned to it) as a directional generalization check; report it separately, never blended into the primary frontier. |
| **Baseline strawman** — an unfairly weak control inflates the win. | Overstates Cartograph's value. | Baseline gets the same model, prompt, turn budget, and a competent `rg` tool with refinement; reviewed as "what a good agent does today." Range-read is available to it. |
| **Token-count drift** — provider tokenizer changes silently. | TC numbers become incomparable over time. | Pin model snapshot; cross-check harness token count against API `usage` per turn; abort on mismatch (§5). |

---

## 10. Open questions and risks

- **Corpus selection is load-bearing and not yet pinned.** The §3 criteria are
  set but the exact repo+SHA is TBD; a poor pick (thin history, flaky suite)
  undermines H3 and grading. This is the single highest-leverage open decision.
- **Class C ground truth is fuzzy.** "What breaks" mined from co-modified PR
  files conflates *had to change* with *happened to change in the same PR*.
  Widening with the static call graph helps but may over-credit. Needs a
  human-audited gold subset to calibrate τ.
- **Patch grading is harsh (pass@1, no feedback).** Real agents iterate against
  test output. A pass@k or test-feedback variant may be a fairer secondary
  metric; decide whether to add it without letting it become the headline.
- **H5 plan auditing vs cost.** Recording every plan is great for audit but adds
  tokens; confirm the recorded-plan overhead is excluded from TC (it is a harness
  artifact, not agent input) and that dry-run plans are scored separately.
- **Model dependence.** The frontier shifts with the agent model. We pin one, but
  the claim "Cartograph helps" must be spot-checked on a second model class
  before any general statement, even if not part of the gated frontier.
- **Branch-switch tail.** Large-diff branch switches may blow the staleness
  window even while single-edit p95 passes the gate; need to decide whether a
  bounded "resyncing, generation N-1" response (per [10](./10-incremental-sync.md))
  is acceptable UX or itself a gate.

---

Related: [README](./README.md) · [01-architecture.md](./01-architecture.md) ·
[02-data-model.md](./02-data-model.md) ·
[03-mcp-tool-surface.md](./03-mcp-tool-surface.md) ·
[10-incremental-sync.md](./10-incremental-sync.md) ·
[09-h5-adaptive-context-budgeter.md](./09-h5-adaptive-context-budgeter.md) ·
[12-roadmap.md](./12-roadmap.md)
