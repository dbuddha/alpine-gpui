# Source map

| ID | Source | Identity and authority | Used for | Not used for |
| --- | --- | --- | --- | --- |
| NIE-S01 | [Apple Energy Efficiency Guide: Best Practices](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/BestPractices.html) | Apple platform guidance | Idle means no polling, timers, or unnecessary work | A numeric Alpine threshold |
| NIE-S02 | [Apple Energy Efficiency Guide: Monitoring](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/MonitoringEnergyUsage.html) | Apple measurement guidance | Activity Monitor, Instruments, and idle CPU interpretation | Cross-device energy comparison |
| NIE-S03 | [Apple Energy Efficiency Guide: Graphics](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/UsingEfficientGraphics.html) | Apple graphics guidance | Avoid invisible and obscured updates | Proof of Alpine implementation behavior |
| NIE-S04 | Local `powermetrics(1)` manual | Tool contract on the qualifying OS image | Sampling interval, plist output, wakeup and power fields, limitations | Stable semantics across unpinned OS releases |
| NIE-S05 | Local macOS SDK `mach/task_info.h` | Public SDK contract for `TASK_POWER_INFO` and `TASK_POWER_INFO_V2` | Per-process interrupt and platform-idle wakeup counters | Private Energy Impact calculation |
| NIE-S06 | [Chromium process metrics at `2da39d93`](https://chromium.googlesource.com/chromium/src/+/2da39d93a2eebc354820daa86f28617036b3f267/base/process/process_metrics_mac.cc) | Pinned production implementation specimen | Public `TASK_POWER_INFO` use and interpretation of package-idle wakeups | Adoption of Chromium private power APIs |
| NIE-S07 | Alpine `SurfaceSnapshot` and native lifecycle artifacts | First-party revision-scoped evidence | Callback, submission, presentation, frame-slot, and retained ownership truth | System package power |

## Provenance rules

1. Every physical evidence bundle records the exact Alpine revision and hashes every raw artifact.
2. OS manuals and SDK headers are captured by OS build and SDK path because their contracts may change.
3. External implementation source is a pinned specimen. No source is copied into Alpine.
4. Apple guidance defines measurement intent, while Alpine counters and raw operating-system samples prove observed behavior.
