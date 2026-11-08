---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# Zed Editor

Zed implements a Debug Adapter Protocol client. A developer can launch a target or attach to a process, set a breakpoint and inspect variables while stepping through source. [Zed debugger guide](https://zed.dev/docs/debugger)

For the stage 6 Rust scenario, share local tool paths with language configuration and start the debugger on an explicit action. A breakpoint and a hover can coexist with independent lifetimes.

**Proposed Alpine check.** Stop at a known line, inspect a local, step out and terminate the adapter during a request. Check that editing still works and that child resources are released. For editor comparisons, first verify both servers loaded the same workspace.
