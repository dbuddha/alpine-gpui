---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# Solo

Solo describes a project manifest that gives multiple clients access to the same long-running processes. Its stated purpose includes avoiding duplicate instances of project services. [Solo workspace overview](https://soloterm.com/)

The analogous Alpine question is who owns a language server when two tabs use it and one closes. Use the existing bounded LSP pool for the experiment.

**Proposed Alpine check.** Attach two documents in one workspace to one language server. Close the first, verify the second retains its unsaved overlay, then close the second and check idle shutdown, process count and retained memory.
