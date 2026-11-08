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

**Alpine result, November 8.** On e482b9f, a disposable test replaced the original file externally and forced the journal writer to report a write failure. The normal close event allowed the first Quit. The candidate fix cancels that close and offers Save As. A file read verifies that the saved copy contains the edits and the original retains the external contents. The deleted-file case verifies that losing focus disarms Quit and a second consecutive close permits exit without saving. Another case restores a missing file, edits it, switches tabs and verifies that Quit selects those edits for Save As. These checks exercise the editor library. [Regression tests](../apps/alpine-editor/src/editor_coverage_tests.rs)
