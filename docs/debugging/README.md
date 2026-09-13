# Debugging procedures

Use the procedure relevant to the observed failure:

- [Studio capture](../quality/studio-dogfood-capture.md).
- [Release profiling](../quality/studio-release-profiling.md).
- [Residency](../quality/studio-residency.md) and [idle energy](../quality/native-idle-energy.md).
- [Renderer stage attribution](../quality/renderer-stage-attribution.md).
- [Performance claim boundaries](../quality/claim-readiness.md).

Preserve raw failures and source/hardware identity. Measure observer overhead and
use disposable fixtures. Capture-owned process cleanup must not close unrelated apps.

For worktree inventory use `git worktree list` and inspect each checkout's status.
Do not remove a dirty checkout or unique commits. The optional
`scripts/check-worktrees.sh --plan-remove /absolute/path` is a read-only planning
helper; no worktree count or cleanup is required for ordinary development.
