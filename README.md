# Alpine Editor

Alpine Editor is a local code editor for Apple Silicon macOS, written in Rust on
Alpine GPUI, an independently written application framework with a direct Metal
renderer. The goal is one editor its author can use every day, with a memory
footprint the alternatives cannot match, owned end to end and understandable
without a plugin API.

Alpine GPUI's programming model is conceptually adapted from
[Zed GPUI](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui).
Alpine is an independent implementation, not a fork or a source-compatible
distribution, and is not affiliated with or endorsed by Zed Industries. Upstream
source is not copied, vendored, or linked.

## Current state

Alpine Editor is a prototype. It builds, launches, renders, and edits files, and
it is not yet a daily driver. Being accurate about the gap matters more than the
feature list, so:

**Works today**

- Opening a file or folder passed at launch, editing, atomic save, undo and redo
- Tabs, bounded splits, a virtualized file tree, session restore
- Find and replace, quick open, project search
- Syntax highlighting for Rust, Markdown, TOML and JSON
- Rust language support against a pinned `rust-analyzer`: diagnostics,
  completion, hover, go to definition, references, document and workspace
  symbols, and rename and format previews
- Unicode, IME composition, clipboard, and accessibility semantics

**Not built yet**

- No menu bar, so every command is keyboard-only
- No open or save dialog, so files and folders can only be chosen at launch
- One window per process
- No git integration, no file watcher, no terminal, no extensions
- No language support beyond the four above
- No design system, which is why surfaces are not yet visually consistent

**Not planned for version 1**

Intel Macs, Linux, Windows, web, mobile, GPUI source compatibility, AI features,
and multiplayer editing.

## Performance

The premise is lower memory use than comparable editors. That premise is
**unverified**. An early comparison measured only idle footprint after opening a
folder, which is not a like-for-like test because Alpine does not read file
contents until a file is opened. No performance claim is currently supported by
evidence, and an earlier renderer measurement favored pinned Zed GPUI by about
12 percent at one stage on one workload.

## Build and run

```sh
cargo run --locked -p alpine-editor              # run against the current tree
cargo run --locked -p alpine-editor path/to/file # open a file or folder
scripts/check.sh                                 # full local gate
```

Build a local application bundle:

```sh
scripts/build-alpine-editor-app.sh
scripts/launch-alpine-editor-app.sh path/to/file-or-folder
```

The bundle is local dogfood infrastructure. Signing, notarization and
distribution are later work.

Native execution is separate from the ordinary test suite:
`scripts/check-native.sh physical shipping` runs the shipping smoke. Neither
workspace tests nor hosted CI prove physical presentation.

Development is PR-first: problem and outcome, change, then verification and
remaining risks. See [CONTRIBUTING.md](CONTRIBUTING.md) and
[ARCHITECTURE.md](ARCHITECTURE.md).

## Ownership and license

Public visibility does not make Alpine open source. Alpine's independently
written source is proprietary under [LICENSE.md](LICENSE.md), which grants no
permission beyond viewing this repository and using GitHub's permitted
repository features. Zed's `gpui` crate declares Apache-2.0 at the reviewed
commit, and that license governs Zed source, which is kept in a separate GPL
comparison repository and never in this one.
