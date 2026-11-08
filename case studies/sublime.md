---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# Sublime Text

Sublime separates shareable project definitions from a user-specific workspace file that includes open files and their modifications. [Sublime project documentation](https://www.sublimetext.com/docs/projects.html)

Stage 1 can test project identity separately from recovery of unsaved text. A restored tab label alone cannot establish that its edits survived.

**Proposed Alpine check.** With a disposable workspace, edit a file, replace its disk contents externally and restore the session. Assert the exact recovered bytes, the external bytes and the active document identity. Inject a failed recovery write before quit and verify the available route to save the edits.
