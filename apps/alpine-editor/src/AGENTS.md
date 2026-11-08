---
scope: editor source under apps/alpine-editor/src/
role: editor subsystem contracts
author: Deepak Buddha
authority: ../../../project.md
related: [../AGENTS.md, ../../../AGENTS.md, ../../../project.md, ../../../notes.md]
parent: ../AGENTS.md
updated: 2026-11-03
---

# Editor subsystems: language intelligence and git

Contracts for language workflows (stage 3) and local Git (stage 4).
Targets below are unimplemented unless a "Today" line says otherwise.
Read ../../../project.md and ../../../notes.md; the project contract owns
scope and acceptance, and root instructions own operating rules.

## Language intelligence

Today: off-main-thread discovery, one server per workspace and language (cap
5), idle shutdown of unattached slots, local-lexer highlighting, plus
../AGENTS.md `known_defects`. A stop sends `shutdown` and `exit`, then TERMs
and KILLs the server's process group within about 0.8 s. Eviction and idle
stops never wait on the main thread; quit waits 1.5 s at most. rust-analyzer
takes 1 to 4 GB on a real crate; a first query waits on its indexing.

Rules:
- Lifetime: start a server on the first visible file of its language; folder
  open starts nothing. Shut idle servers down after the TTL, and at once on
  macOS memory pressure. Eviction and shutdown never run on the main thread.
- OS hooks (memory pressure, FSEvents, process priority) live in
  alpine-platform-macos behind an alpine-platform trait; alpine-editor has no
  unsafe code. Each new binding, crate feature or unsafe file is `ask_first`.
- Requests only on intent: completion on trigger characters, a typing pause or
  a key, one request in flight, cancelled on supersede; hover on a key or a
  deliberate mouse rest; signature help on `(` and `,`; references and code
  actions only when asked. Diagnostics render after a typing pause, capped,
  visible range only. Inlay hints off by default; never semantic tokens.
  While a server indexes, suppress optional requests.
- Completion stays local after the first list: filter as the prefix grows,
  re-request only when the server marked the list incomplete, resolve only the
  selected item.
- Per keystroke: incremental didChange from edit transactions, flushed at most
  once per event-loop turn through a bounded queue that merges when full.
  Typing never waits on a server. Scroll and pointer moves trigger no
  language work.
- Main thread: decode JSON on reader threads into typed structs, admit a
  bounded amount per turn by document revision, keep only the newest
  diagnostics per file, drop stale versions. Cancellation is advisory; local
  revocation by request ID is authoritative.
- Positions: advertise `["utf-8", "utf-16"]` and use the encoding the server
  picks.
- Server settings: keep only those the bench shows to help. Memory candidates:
  rust-analyzer without cache priming, clangd with capped index threads, jdtls
  with a JVM heap cap, typescript-language-server's `maxTsServerMemory`
  (tsserver is its child), pylsp with unused plugins off. rust-analyzer's own
  check target directory avoids cargo lock waits.
- Energy: run servers at background priority only if the bench shows no
  response cost.
- No downloads: discover servers and show the install command when one is
  missing.

Implement serially: wire format (Cow JSON fix, string IDs, server requests,
processId, capabilities, UTF-8 negotiation and event-driven waits); supervisor
without 2 ms polling; pool/discovery; incremental sync and didSave (16 open
documents/server); off-main decoding; completion; hover/signature help;
diagnostics; code actions/applyEdit; seven-language memory qualification.
Recheck implemented behavior before starting each slice. Every target language
needs real-server evidence; settings and background priority are experiments.
Pool eviction prefers the least recently used server without a visible tab;
use a deadline thread, sync only real changes and show missing-tool install hints.

CI counter tests assert: zero requests while typing within the pause, zero
JSON parsing on the main thread, zero language work on scroll or pointer
moves, and incremental sync (a one-character edit on a 1 MiB file sends one
didChange under 512 bytes). T2, dev Mac bench: no language work over 1 ms on
the main thread under load.

## Local Git

Rules:
- Detect the repository without spawning; start nothing until a feature needs
  it. Resolve a `.git` file to its git directory (linked worktrees,
  submodules).
- Branch: read HEAD in the resolved git directory. One FSEvents stream,
  started by the first git feature, covers the workspace and that directory
  and drives refreshes; ignore build output such as `target/`; never poll.
- Contents: one long-lived `git cat-file --batch-command`: `info` gives the
  object id, `contents` runs only on a cache miss. Cache HEAD blobs by object
  id in an LRU under a byte cap; skip diffs for files above a size cap.
- Gutter diff: computed in process on a worker after a typing pause.
- Blame: `git blame --incremental --contents=- -L <visible range> -- <path>`
  on the open buffer, on demand, cancelled on scroll. Entries arrive unordered
  and without line text, which the buffer has.
- Status: `git --no-optional-locks status --porcelain=v2 -z
  --ignore-submodules=dirty`, debounced and capped; skip the untracked scan on
  huge repositories.
- Stage and commit are explicit. At most two git processes: the long-lived
  cat-file, plus one at a time for status, blame or commit, each with a
  timeout, at background priority if measured harmless.

T2, dev Mac bench: typing latency unchanged while `git status` runs on a
large repository.

## Planned interfaces

Follow project.md for preferences, preview tabs, structural parsing, Markdown
and debugger scope. Preserve settings layers and rejection of malformed reloads.
The debugger manager is separate from LSP, bounded and explicit-intent only;
shared workspace/language configuration does not make it an LSP server.
