# Metrics and reporting

Count leaf tasks, not parents and children together. Keep scope additions, removals, completed work, and not-planned work separate. Preserve snapshot time and filter identity. GitHub Project historical insights do not track archived or deleted items, so archive policy is part of metric identity.

| Measure | Question answered | Failure prevented |
| --- | --- | --- |
| Burn-up | Is completion outpacing scope growth? | Hidden scope expansion |
| Frozen-scope burn-down | Are we consuming a committed cohort? | Moving-target forecasts |
| Scope trend | What entered or left after commitment? | Artificial velocity |
| Leaf completion | Which acceptance units are done? | Inflated parent counters |
| Throughput | How many accepted leaves close per window? | Activity mistaken for value |
| Cycle time | How long from In Progress to Done? | Slow review hidden by starts |
| Blocker age | How long has critical work been blocked? | Stale blockers |
| Work in progress | How much is active concurrently? | Fragmentation |
| Critical path | What currently controls the outcome? | Optimizing non-blocking work |

## Burn-up protocol

Choose a stable cohort or explicit filter. Plot total in-scope leaves and completed leaves. Annotate requirement changes and task additions. Report not-planned closures separately. Explain whether a widening gap is discovery, correction, or drift.

## Burn-down protocol

Preserve a frozen list of issue node IDs and estimates. Plot remaining accepted estimate. Additions after the snapshot are a separate scope-change series.

## Forecasting

Forecast only from comparable completed leaves. Give a range and confidence, list assumptions, and name the dominant blocker. Do not turn relative estimates into hours. If samples are weak or scope unstable, state that no forecast is defensible.

## Status report

Report objective and snapshot identity, accepted leaves completed, critical path and next gate, blockers and unblock conditions, scope changes, exact PR and CI state, milestone leaf completion, forecast or its limits, and next smallest uncompromised action.

GitHub's default historical burn-up visualizes completed and remaining work: https://docs.github.com/en/issues/planning-and-tracking-with-projects/viewing-insights-from-your-project/about-insights-for-projects
