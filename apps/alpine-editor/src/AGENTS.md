---
scope: editor source under apps/alpine-editor/src/
parent: ../AGENTS.md
updated: 2026-10-02
---

# Editor subsystems: language intelligence and git

Design rules for M4 (language intelligence) and M7 (git), and the M4 slices;
M7 slices are planned at its kickoff. All of it is a target unless a "Today"
line says otherwise. The root and editor AGENTS.md files own the budgets,
invariants and product rules.

## Language intelligence (M4)

Today: one server per workspace and language, hard cap 5, idle shutdown of
unattached slots, local-lexer highlighting, and the `known_defects` in
../AGENTS.md. rust-analyzer alone is 1 to 4 GB on a real crate. A big
project's first query still waits on the server's own indexing.

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

Slices, serial:
1. Wire format: the `Cow<str>` JSON fix, string ids, answers to server
   requests, `processId`, capabilities, UTF-8 negotiation, event-driven test
   waits instead of fixed 5 s loops.
2. Shutdown worker: a process group per server, shutdown then exit then kill,
   an event-driven supervisor (no 2 ms polling), recovery published before
   server shutdown on quit.
3. Pool and discovery: evict the least recently used server with no visible
   tab, a deadline thread, memory-pressure shutdown, discovery off the main
   thread, sync only on real changes, install hints. Closes 2.1 (cold path)
   and 2.3.
4. Incremental sync and `didSave`; up to 16 open documents per server.
5. Decoding off the main thread: typed structs, a per-turn budget, newest
   diagnostics only, progress-aware suppression, caps that truncate.
6. Quiet completion with local filtering and auto-import edits.
7. Hover on mouse rest, and signature help.
8. Inline diagnostics after a pause, visible lines only.
9. Code actions, server `applyEdit`, inlay hints off by default.
10. Per-server memory and the five-language close: server settings and
    background priority as measured experiments, the Zed memory bench.
    Closes 2.2.

CI counter tests assert: zero requests while typing within the pause, zero
JSON parsing on the main thread, zero language work on scroll or pointer
moves, and incremental sync (a one-character edit on a 1 MiB file sends one
didChange under 512 bytes). T2, dev Mac bench: no language work over 1 ms on
the main thread under load.

## Git (M7)

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
