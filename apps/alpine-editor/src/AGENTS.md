---
scope: editor source under apps/alpine-editor/src/
parent: ../AGENTS.md
updated: 2026-10-02
---

# Editor subsystems: language intelligence and git

Design rules and slice plans for M4 (language intelligence) and M7 (git). All
of it is a target unless a "Today" line says otherwise. The root and editor
AGENTS.md files own the budgets, invariants and product rules.

## Language intelligence (M4)

Today: one server per workspace and language, hard cap 5, idle shutdown of
unattached slots, local-lexer highlighting, and the `known_defects` in
../AGENTS.md. rust-analyzer alone is 1 to 4 GB on a real crate. The first query
in a big project still waits for the server's own indexing; that time is the
server's, not Alpine's.

Rules:
- Lifetime: start a server on the first visible file of its language; folder
  open starts nothing. Shut idle servers down after the TTL, and at once on
  macOS memory pressure. Eviction and shutdown never run on the main thread.
- Requests only on intent: completion on trigger characters, a typing pause or
  a key, one request in flight, cancelled on supersede; hover on a key or a
  deliberate mouse rest; signature help on `(` and `,`; references and code
  actions only when asked. Diagnostics render after a typing pause, capped,
  visible range only. Inlay hints off by default; never semantic tokens.
  While a server indexes, suppress optional requests.
- Completion stays local after the first list: filter as the prefix grows,
  re-request only when the server marked the list incomplete, resolve only the
  selected item.
- Per keystroke: incremental didChange from edit transactions, at most once
  per frame, through a bounded queue that merges when full. Typing never
  waits on a server. Scroll and pointer moves trigger no language work.
- Main thread: decode JSON on reader threads into typed structs, admit a
  bounded amount per frame by document revision, keep only the newest
  diagnostics per file, drop stale versions. Cancellation is advisory; local
  revocation by request ID is authoritative.
- Positions: offer UTF-8 where the server supports it, UTF-16 otherwise.
- Server memory: configure each server and keep only settings the bench shows
  help. Candidates: rust-analyzer without cache priming and with its own check
  target directory; clangd with capped index threads; jdtls with a JVM heap
  cap; Node-based servers with a heap cap; pylsp with unused plugins off.
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
3. Pool and discovery: evict by visibility, a deadline thread, memory-pressure
   shutdown, discovery off the main thread, sync only on real changes, install
   hints. Closes 2.1 and 2.3.
4. Incremental sync and `didSave`; up to 16 open documents per server.
5. Decoding off the main thread: typed structs, a per-frame budget, newest
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
moves, and bytes sent per edit (10 keystrokes on a 1 MiB file under 2 KiB).
Gate: no language work over 1 ms on the main thread under load.

## Git (M7)

Rules:
- Detect the repository without spawning; start nothing until a feature needs
  it. Handle a `.git` file that points elsewhere (linked worktrees,
  submodules).
- Branch: read `.git/HEAD` directly. One FSEvents stream per workspace drives
  refreshes; never poll.
- Contents: one long-lived `git cat-file --batch`. Cache HEAD blobs by object
  id under a byte cap; skip diffs for files above a size cap.
- Gutter diff: computed in process on a worker after a typing pause.
- Blame: `git blame --porcelain --incremental --contents=- -L <visible range>`
  on the open buffer, on demand, cancelled on scroll.
- Status: `git --no-optional-locks status --porcelain=v2 -z
  --ignore-submodules=dirty`, debounced and capped; skip the untracked scan on
  huge repositories.
- Stage and commit are explicit. At most two git processes, each with a
  timeout, at background priority if measured harmless.

Gate: typing latency unchanged while `git status` runs on a large repository.
