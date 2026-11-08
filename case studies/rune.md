---
role: case study
author: Deepak Buddha
updated: 2026-11-08
authority: research input to project.md
related: [README.md, ../project.md]
---

# Rune

Rune’s pinned Linux CI builds the binaries, runs a bounded display-free startup probe and checks that its headless process reaches the authentication step. Its broader tests run under a virtual display. [Rune CI at 6365c87](https://github.com/unstablebuild/rune/blob/6365c87be1ce8213cae2e74235eae2b8c697013e/.github/workflows/test-linux.yml)

Rune also documents a networked headless workspace service and a separate macOS bundle build. Alpine’s applicable question is how much local editing and recovery can be verified without acquiring the display. [Rune README at the same revision](https://github.com/unstablebuild/rune/blob/6365c87be1ce8213cae2e74235eae2b8c697013e/README.md#headless)

**Proposed Alpine check.** Use scripts/build-alpine-editor-app.sh with `--output "$scratch/Alpine Editor.app"`, where `$scratch` is a fresh directory outside target/. Inspect the build stamp before launch. Drive edit, failed save, quit decision and recovery reload through the editor library using a disposable directory. Verify bytes and child teardown. Reserve window visibility, display timestamps, Dock launch and IME delivery for approved Mac sessions.
