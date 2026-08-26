# Alpine Studio and Zed Editor lineage

## Product boundary

Zed is a broad production editor with collaboration, remote projects, accounts,
AI, extensions, terminal, Git, debugger, and multi-window or multi-platform
concerns. Alpine Studio is a local Apple-first editor and renderer proving
ground. Feature parity with all of Zed is neither expected nor desirable.

The comparison therefore has two views:

- Private daily-driver capabilities that Alpine accepts.
- Zed capabilities that Alpine intentionally excludes.

## Accepted daily-driver capability accounting

| ID | Capability | Zed source boundary | Alpine state through `e256405` | Lineage verdict |
| --- | --- | --- | --- | --- |
| S01 | Stable application launch | Zed app and platform crates | `.app` assembly, explicit launch/recovery composition, and revision-pinned Finder journey accepted in [#303](https://github.com/dbuddha/alpine-gpui/issues/303) | Independent product implementation; daily-driver and public release gates remain separate |
| S02 | Native keyboard, pointer, scroll, focus, clipboard, IME | GPUI platform plus editor event routing | Production AppKit events and focus-epoch IME exist | Shared platform requirement; Alpine adds narrow identity and caps |
| S03 | Local text buffer and snapshots | Zed text retains replica and operation state | Ropey-backed local `Buffer`, revisions, snapshots, transactions, undo/redo | Deliberate local simplification, not a Zed text port |
| S04 | Unicode coordinate conversion | Zed editor/text/language conversions | Byte, grapheme, line/column, AppKit UTF-16, and LSP UTF-16 boundaries | Independent convergence with explicit checked conversion |
| S05 | Atomic save and external change handling | Zed buffer/project persistence | Atomic save, external-change detection, close policy | Independent local implementation |
| S06 | Folder and virtualized file tree | Zed workspace/project | Bounded lazy workspace inventory and file tree | Behavioral adaptation with explicit path and retained-byte caps |
| S07 | Tabs and active editor | Zed panes/items | Bounded pane tabs and active-editor history | Behavioral adaptation without GPUI entities |
| S08 | Splits | Zed pane groups | Purpose-built row/column split state | Behavioral adaptation with narrower layout model |
| S09 | Session and dirty recovery | Zed workspace persistence | Checksummed bounded restoration and dirty-buffer journal | Independent implementation with crash-safe local boundary |
| S10 | Quick open | Zed project search and picker | Lazy bounded inventory and ranked quick open | Behavioral adaptation with fixed query/path/result budgets |
| S11 | Command palette | Zed actions and command palette | Static typed commands and authoritative shortcuts | Deliberately rejects runtime plugin registration |
| S12 | Find and replace | Zed editor search | Bounded in-file find/replace | Independent editor behavior |
| S13 | Project search | Zed project search | Streaming local search with inventory/read/result/batch caps | Narrow local adaptation with graceful truncation |
| S14 | Built-in syntax | Zed language registry and Tree-sitter ecosystem | Compiled Rust, Markdown, TOML, JSON, and plain-text cohort | Smaller static implementation; no grammar extension API |
| S15 | Typed settings, themes, keymaps | Zed settings and dynamic registrations | Central compiled schema and deterministic layering | Implemented core; reload and migration remain open in #222 |
| S16 | Local language process | Zed project/language/LSP stores | Bounded child process, framing, JSON-RPC, lifecycle, pinned rust-analyzer | Narrow independent transport; no remote server or extension host |
| S17 | Diagnostics | Zed editor/project language integration | Revision-safe bounded Rust diagnostics | Behavioral subset implemented |
| S18 | Completion | Zed editor completion and language stores | Revision-safe bounded Rust completion | Behavioral subset implemented |
| S19 | Hover and source navigation | Zed editor hover and navigation | Bounded hover, definition, references, local-path admission, revision-safe supersession, overlays, and accessibility merged in [PR #345](https://github.com/dbuddha/alpine-gpui/pull/345) | Behavioral subset implemented with stricter local path and result bounds |
| S20 | Rename and formatting | Zed project transactions and editor edits | Strict bounded response admission and immutable off-thread preparation merged in [PR #348](https://github.com/dbuddha/alpine-gpui/pull/348); local #220 work adds revision-bound preview, bounded checksummed Preparing/Prepared/Committed publication, fail-closed rollback/startup recovery, loaded-tab admission, active-document undo, and an exact release-pinned rust-analyzer journey that admits and prepares real rename and formatting responses | Local implementation and local pinned-server evidence are staged; hosted exact-head, complete fault-injection, mutation, and coverage evidence remain before an E3 or completion claim |
| S21 | Document and workspace symbols | Zed project symbol search | Bounded current-only requests, ranking, keyboard and IME picker, scene, accessibility, and checked local navigation merged in [PR #350](https://github.com/dbuddha/alpine-gpui/pull/350) | Behavioral subset with explicit result, hierarchy, label, query, retained-byte, and visible-row bounds |
| S22 | Native accessibility | GPUI accessibility plus editor semantics | Snapshot, AppKit transport, text mappings, actions, notifications | Implementation present; physical VoiceOver/AX proof remains open |
| S23 | Local diagnostic evidence | Zed diagnostics/profiling facilities | Frame, cache, residency tools, event-to-present correlation and signposts | Alpine-specific claim discipline; external physical capture pending |
| S24 | Sustained repository dogfood | Zed is a mature production editor | Open #224 and #238 through #242; visible typing defect #304 | Blocking qualification, not a polish item |

Twenty rows implement their selected behavior. S15, S20, and S22 have
production implementation but incomplete reload/migration, edit publication,
or physical qualification, and S24 remains incomplete. Twenty-three rows
therefore have some production implementation. These counts are an inventory,
not readiness; the verdict remains "working prototype, not trusted daily
driver."

## Intentionally excluded Zed scope

| Zed subsystem | Alpine decision | Performance and delivery effect | Correctness cost avoided |
| --- | --- | --- | --- |
| Collaboration and replicated buffers | Reject | Avoids replica clocks, operation history, network scheduling, presence rendering, and retained remote state | No conflict resolution, reconnect, authorization, or replica-consistency state machine |
| Hosted AI and agent UI | Reject | Avoids model clients, context indexing, prompt UI, token storage, and background traffic | No provider, privacy, streaming, cancellation, or generated-edit trust boundary |
| Cloud account and synchronization | Reject | Avoids startup network/account state and sync residency | No authentication, sync conflict, or server lifecycle |
| Extension host and marketplace | Reject | Avoids dynamic loading, extension processes, registration graphs, and compatibility surface | No untrusted extension isolation or evolving plugin ABI |
| Remote development | Reject | Avoids remote filesystem/process protocol and duplicated local/remote identity | No disconnect, remote authority, or transport consistency |
| Integrated terminal and tasks | Defer until after dogfood | Removes terminal emulation and process orchestration from M5 | No PTY, shell integration, task cancellation, or escape-sequence surface |
| Git UI | Defer until after dogfood | External Git remains available without product weight | No repository mutation, credential, or index-lock state machine |
| Debugger | Reject for v1 | Avoids DAP, process control, breakpoint, and debug rendering | No debuggee lifecycle or privileged process boundary |
| Telemetry | Reject | No background upload, queue, schema, or privacy cost | No consent, redaction, retention, or transport behavior |

These exclusions are not evidence that Alpine is faster. They are a smaller
product contract whose CPU, memory, startup, and latency effect must still be
measured in normalized and stock-product comparisons.

## Source-derived differences that matter

### Text ownership

Zed's text model exposes replica identity and operation-oriented structures
because collaboration is a first-class requirement. Alpine's local buffer uses
monotonic revisions, immutable snapshots, compact transactions, and local
undo/redo. This is a substantive simplification that should reduce state and
failure modes, but physical memory savings remain unqualified.

### Runtime and state

Zed editor behavior is composed through GPUI entities, views, contexts,
actions, subscriptions, and async tasks. Alpine uses direct
`StudioApp -> Workspace -> Editor -> Buffer` ownership and bounded channels.
This removes framework machinery but can concentrate complexity in Studio and
increase manual routing. The large Studio source surface is now a maintenance
risk, so extraction should target repeated measured contracts rather than a
general GPUI clone.

### Rendering

Both construct visible editor content, shape text, use glyph atlases, batch
primitives, and present through Metal on macOS. Alpine narrows primitives to
quads, clips, and monochrome glyphs, uses explicit painter-order operations,
and records no/full/row-delta atlas publication. The warm deterministic path
avoids rasterization and uploads, but no matched code-viewport E4 comparison
exists.

### Language intelligence

Zed has a mature project/language architecture supporting many servers,
languages, dynamic registrations, remote projects, worktrees, and richer editor
features. Alpine has one bounded local JSON-RPC/LSP transport and qualifies only
rust-analyzer. This is directionally correct for delivery and memory. Hover,
definition, references, and bounded symbols are implemented; rename and
formatting publication, restart behavior, and sustained real-server use must
complete before daily-driver acceptance.

### Settings and extensibility

Zed supports a broad evolving settings and extension ecosystem. Alpine compiles
one typed schema, static command set, built-in syntax cohort, themes, and
keymaps. The smaller model prevents runtime registration weight, but safe reload,
migration, diagnostics, and project override behavior still need #222.

## Fair product comparison

Alpine Studio versus Zed must be reported in two lanes:

1. Normalized local-editor journeys with accounts, AI, collaboration, telemetry,
   extensions, remote development, and plugins disabled where configurable.
2. Stock-product footprint, reported separately as a product-scope observation.

Every journey must retain exact files, settings, fonts, language server,
hardware, OS, display mode, power, thermal state, revision, cold/warm status,
and semantic output. Alpine may claim only the named metric and journey whose
confidence interval passes the accepted comparator protocol.

## Studio conclusion

Alpine has recreated a substantial local editing vertical slice, not Zed Editor
as a whole. The remaining work is concentrated and high impact: fix typing
latency, finish physical M4 qualification, complete the four Rust/config gaps,
dogfood safely, measure residency, and then compare matched journeys. Adding
terminal, Git, plugins, AI, collaboration, remote, or broad framework machinery
now would move the project in the wrong direction.
