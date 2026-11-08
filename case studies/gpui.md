---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# GPUI

GPUI combines retained state with immediate rendering in a GPU-backed Rust framework. Its render example constructs elements from view state. [GPUI framework overview](https://gpui.rs/)

Alpine already recorded a costly synchronous GPU wait and moved to completion-owned frame slots. Preserve that finding when designing the stage 5 comparison. [Alpine research history](../ARCHIVE.md#research-conclusions)

**Proposed Alpine check.** Render an identical scrolling viewport in both frameworks. Freeze glyphs, visible row count and invalidations. Compare CPU preparation time, retained bytes, missed refreshes and idle submissions. Verify painter order and the actual visible rows before comparing timings.
