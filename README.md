# Alpine Editor

Alpine Editor is a local code editor for Apple Silicon macOS, written in Rust on
Alpine GPUI, an independently written application framework with a Direct Metal
renderer. The goal is one editor its author uses every day: lower latency and
memory than Zed, a steady 120 Hz, and nothing loaded until it is needed.

Alpine GPUI's programming model is conceptually adapted from
[Zed GPUI](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui).
Alpine is an independent implementation, not a fork or a source-compatible
distribution, and is not affiliated with or endorsed by Zed Industries.

## Status

A prototype, not yet a daily driver. Performance has not been measured against
Zed yet; that is the next milestone.

Works today: opening files and folders from the File menu or at launch,
editing, atomic save, undo and redo, tabs, splits, a lazy file tree, session
restore, find and replace, quick open, project search, syntax highlighting for
nine languages, and language-server features (diagnostics, completion, hover,
definition, references, symbols, rename and format previews). Those are
verified for Rust; C++ shows clangd diagnostics; the other languages use the
same layer once their server is installed.

Not yet: git features, a file watcher, multiple windows, a design system, vim
mode and multi-cursor editing.

## Requirements

- An Apple Silicon Mac with macOS 15 or newer.
- Rust 1.97.1; `rust-toolchain.toml` selects it through rustup.
- Xcode Command Line Tools. Full Xcode is needed only to change
  `shaders/offscreen.metal`; the compiled library is checked in.
- `cargo-deny` for the full local check (`scripts/check.sh`).

## Build and run

```sh
cargo run --locked -p alpine-editor [file-or-folder]
scripts/build-alpine-editor-app.sh
scripts/launch-alpine-editor-app.sh <file-or-folder>
```

The build script installs an unsigned `~/Applications/Alpine Editor.app` from a
clean checkout.

## Languages

| Language | Extensions | Server Alpine looks for | Override |
| --- | --- | --- | --- |
| Rust | rs | rust-analyzer (from rustup toolchains) | `ALPINE_RUST_ANALYZER` |
| Python | py, pyi | pylsp, basedpyright, pyright-langserver | `ALPINE_PYTHON_LS` |
| C and C++ | c, cc, cpp, cxx, h, hh, hpp, hxx | clangd | `ALPINE_CLANGD` |
| Java | java | jdtls | `ALPINE_JDTLS` |
| TypeScript, JavaScript | ts, tsx, mts, cts, js, jsx, mjs, cjs | typescript-language-server (one shared server) | `ALPINE_TYPESCRIPT_LS` |
| Markdown, TOML, JSON | md, markdown, toml, Cargo.lock, json | none | none |

Highlighting never waits for a server. Alpine never downloads servers. It
searches the launch `PATH`, then `~/.cargo/bin`, `/opt/homebrew/bin`,
`/usr/local/bin` and `/opt/homebrew/opt/llvm/bin`, and finds rust-analyzer in
rustup toolchains. An app launched from the Dock gets a minimal `PATH`, so a
server installed elsewhere (pipx's `~/.local/bin`, an nvm prefix) needs a link
in one of those directories. Install the ones you want:

```sh
rustup component add rust-analyzer
pipx install python-lsp-server
npm install -g typescript-language-server typescript
xcode-select --install
```

The last command provides clangd. For Java, install a JDK and link Eclipse
`jdtls` into one of the searched directories. Override variables take the
full path of a server executable and apply to terminal launches such as
`cargo run`. To remove a language, write `disabled = ["java"]` to
`~/Library/Application Support/Alpine Editor/languages.overlay.toml`.

## Settings

Alpine reads local JSON only: compiled defaults, then
`~/Library/Application Support/Alpine Editor/settings.json`, then
`<workspace>/.alpine/settings.json`. Later layers override earlier ones and
missing files are ignored. A malformed layer rejects the whole reload and keeps
the previous settings. Each file is capped at 64 KiB.

```json
{
  "version": 1,
  "editor": { "font_name": "Menlo-Regular", "font_size": 15, "font_scale": 2,
              "line_height": 22, "tab_columns": 4 },
  "theme": { "background": [0.035, 0.04, 0.045, 1.0],
             "syntax": { "comment": [0.48, 0.60, 0.53, 1.0] } },
  "keymap": { "bindings": [ { "physical_key": 1, "modifiers": ["command"],
                              "action": "save_file", "label": "Cmd+S" } ] }
}
```

`editor` and `theme` are partial; theme colours are linear RGBA from 0.0 to
1.0, and `font_name` accepts only `Menlo-Regular`. A supplied keymap replaces
the defaults and holds at most 64 bindings. Reload with the command palette
action "Preferences: Reload Settings". Version 0 files (top-level `font_size`,
`font_scale`, `line_height`, `tab_columns`) migrate in memory; the file is never
rewritten.

## Data and recovery

Settings, the session and the recovery journal live in
`~/Library/Application Support/Alpine Editor`. Until it writes an
`.imported-from-alpine-studio` marker there, Alpine copies top-level files
from the pre-rename `Alpine Studio` folder that the new folder lacks. Existing
files win, and the old folder is never modified. Unsaved buffers are
journaled: at most 32 documents, 32 MiB each and 64 MiB in total.

## Limitations

The bundle is unsigned and built locally. Typing latency, 120 Hz presentation,
IME candidate placement and VoiceOver are not yet qualified. There is no
terminal, extension system, AI feature, account or telemetry; use an external
terminal and git.

## Ownership and license

Public visibility does not make Alpine open source. Alpine's independently
written source is proprietary under [LICENSE.md](LICENSE.md), which grants no
permission beyond viewing this repository and using GitHub's permitted
repository features. No Zed application source is in this repository. Zed's
`gpui` crate declares Apache-2.0 at the reviewed commit.
