# Ruleset performance

These metrics use the existing authenticated `/metrics` endpoint and
`SCRYER_METRICS=1` recorder. They cover every active release-scoring ruleset,
including built-in TRaSH, community packs, custom rules and plugin policies.
They do not cover request or maintenance policy execution.

## What is measured

| Metric | Meaning |
|---|---|
| `scryer_rule_load_seconds` | Each ruleset's validation, source parsing, wrapper loading and reference analysis. |
| `scryer_rule_evaluation_seconds` | Wall time for a ruleset invocation, including output decoding, no matches and failures. |
| `scryer_rule_evaluations_total` | Invocations by outcome: `ok`, `no_match`, `error`, `skipped` (facet), or `held` (unavailable facts in policy families supporting holds). Skips and holds have no evaluation latency sample. |
| `scryer_rules_stage_seconds` | Timing for the stages below. |
| `scryer_scoring_batch_candidates_total` | Input candidates in scored search batches, including candidates filtered before scoring. |

Stage labels:

- `engine_build`: the complete captured-policy build, including configuration,
  baseline validation, package rewriting, loading and failures. It does not
  include fetching the policy snapshot from storage.
- `snapshot_wait`: acquisition of the active engine's read lock when resolving
  canonical context; `snapshot_clone`: cloning that snapshot.
- `evaluator_create`: cloning an evaluator from its engine. A search batch reuses
  it across candidates; standalone scoring and previews can create new ones.
- `input_prepare`: serializing and installing the Rego input for each phase.
- `rules_apply`: constructing the typed input, executing all rules/phases and
  converting their results into canonical scoring contributions. Rule failures
  set `outcome="error"` even when other rules still contribute.
- `scoring_release`: the complete canonical scoring call, including mandatory
  requirements and final score aggregation. An analyzed release can evaluate
  both announced and file-probed evidence. This excludes evaluator creation.
- `scan_batch`: search-result processing, including context lookup, parsing,
  filtering, scoring and sorting. It excludes upstream indexer network fetches.
  It is elapsed wall time, including async waits, not CPU time.

Stages nest: **do not add their durations together**. Canonical scoring can run
outside searches too (for example imports and incumbents); its count is not
necessarily the search-batch candidate count. Blocked releases are normal scoring
results, not evaluation errors. See `rules_apply` and per-rule outcomes for errors;
`scoring_release` measures completion of the scoring call, including partial results.

`purpose="live"` separates normal work from explicit `purpose="preview"` tests.
Live rule metrics use stable `rule_set_id` values, which match the rules API's IDs.
Preview IDs are collapsed to `rule_set_id="preview"` to avoid an unbounded series
for unsaved drafts. Disabled rules are absent, not zero-cost samples. Facet-skipped
rules have a counter but do not dilute evaluation percentiles.

`phase="baseline"` and `phase="additional"` distinguish execution phases.
`temperature="first"` means the first actual invocation of that rule in that
evaluator; subsequent invocations are `warm`. It does **not** mean first since
startup or first following a rule update. Shared lazy compilation may occur on
the first invocation, so source-load timing alone is not the full build cost.

Cross-rule references and shared caches charge work to the invocation that
triggers it. Consequently, per-rule timing identifies expensive observed calls;
it does not predict precisely how much disabling that rule would save.
Histograms have microsecond buckets; p95 is an estimate within those buckets.
Metric recording adds overhead, included in enclosing stages but excluded
from each rule's own reported duration. Rule IDs survive updates, so use a time
window covering the configuration you want to measure. Idle series expire after
24 hours. No release names, titles, rule source, versions or errors become labels.

## PromQL examples

Use your instance selector in place of `instance="scryer:8080"`.

Average warm milliseconds per ruleset:

```promql
1000 * sum by (rule_set_id) (
  rate(scryer_rule_evaluation_seconds_sum{instance="scryer:8080",purpose="live",temperature="warm"}[15m])
) / sum by (rule_set_id) (
  rate(scryer_rule_evaluation_seconds_count{instance="scryer:8080",purpose="live",temperature="warm"}[15m])
)
```

Warm p95 milliseconds per ruleset (use `temperature="first"` for first calls):

```promql
1000 * histogram_quantile(0.95, sum by (le, rule_set_id) (
  rate(scryer_rule_evaluation_seconds_bucket{instance="scryer:8080",purpose="live",temperature="warm"}[15m])
))
```

Average milliseconds for all scoring work per release:

```promql
1000 * sum(rate(scryer_rules_stage_seconds_sum{instance="scryer:8080",purpose="live",stage="scoring_release"}[15m]))
/ sum(rate(scryer_rules_stage_seconds_count{instance="scryer:8080",purpose="live",stage="scoring_release"}[15m]))
```

Use `stage="rules_apply"` to isolate the rule portion of that call, or
`stage="engine_build"` for engine build durations. Source load per ruleset is
available from `scryer_rule_load_seconds`. Change `purpose` to inspect previews.

Average source-load milliseconds per ruleset since process startup (also shows
loads that occurred before the first scrape, which `rate()` cannot recover):

```promql
1000 * sum by (rule_set_id) (scryer_rule_load_seconds_sum{instance="scryer:8080",purpose="live"})
/ sum by (rule_set_id) (scryer_rule_load_seconds_count{instance="scryer:8080",purpose="live"})
```

This averages retained observations across rebuilds; it is not the most recent
build alone. The same `_sum / _count` approach works for `stage="engine_build"`.

Amortized batch milliseconds per input candidate, including preparation and filtering:

```promql
1000 * sum(rate(scryer_rules_stage_seconds_sum{instance="scryer:8080",purpose="live",stage="scan_batch"}[15m]))
/ sum(rate(scryer_scoring_batch_candidates_total{instance="scryer:8080",purpose="live"}[15m]))
```

Counts by rule and outcome in the last hour:

```promql
sum by (rule_set_id, outcome) (
  increase(scryer_rule_evaluations_total{instance="scryer:8080",purpose="live"}[1h])
)
```

Low-volume first calls and builds may need a longer range. Missing series mean
no observations in the selected window, not a measured zero.
