---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# Neovim

Neovim composes operators with motions. Counts on the operator and motion multiply, and linewise and characterwise motions select different ranges. [Neovim motion reference](https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/motion.txt)

Build stage 3 command cases from the agreed Vim subset. Record input, initial buffer and cursor, register contents, resulting bytes and undo state.

**Proposed Alpine check.** Use a buffer with seven words and a second line. From the first word, apply 2d3w, then undo. Check the six-word range, register and restored buffer. Add a linewise deletion and a Unicode boundary case before claiming the operator works.
