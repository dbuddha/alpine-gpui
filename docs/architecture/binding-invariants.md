# Binding invariants

1. Public behavior is specified independently of upstream implementations.
2. Safe crates deny unsafe code. Audited native boundaries are limited to
   `alpine-metal`, `alpine-platform-macos`, `alpine-text-layout` and the nonshipping
   `alpine-ax-client` tool, as enforced by `scripts/check-policy.sh`. Native FFI
   stays behind reviewed safe APIs, local safety arguments and focused tests.
3. Capabilities are queried at runtime and verified by behavior.
4. Unsupported capability, allocation failure, surface loss, and device loss
   become structured errors rather than panics.
5. The committed lockfile is validated with `--locked`; CI never updates it.
6. Git dependencies are prohibited in shipping manifests.
7. Architecture boundaries are added only when an implemented vertical slice
   needs them.
8. Accessibility is part of every future interactive component contract.
9. Performance claims require reproducible evidence, and blocking regression
   gates require qualified fixed hardware.
10. Update the relevant invariant when changed behavior makes its guidance wrong.
    Explain architectural rationale in the PR. A new boundary needs owner
    authorization unless already granted; no separate decision issue is required.

### Accessibility semantics (Task #130)

Studio derives a bounded semantic tree from the authoritative tab, focus,
status, selection, and immutable buffer state. Tree and action identities carry
both document and buffer revisions; stale assistive-technology actions fail
before mutation. Text remains in the copy-on-write snapshot and is materialized
only for an explicitly bounded UTF-16 range request. Existing AppKit UTF-16
conversion walks rope storage directly without allocating a whole-document
string. The model exposes stable roles for the window, tabs, active code editor,
file tree, transient search and command surfaces, and announcing status.

The public safe transport is owned by `alpine-platform-macos`: validated roles,
nodes, revisions, UTF-16 ranges, selections, requests, responses, errors, and
accounting contain no AppKit handle. One synchronous `SurfaceEvent` request
crosses the existing main-thread delegate boundary and `AppContext` admits at
most one exact response. Semantic snapshots retain at most 271 nodes, 4 KiB per
name, 256 KiB of referenced names, and no document text. Text is pulled against
an exact document and buffer revision and is materialized only after the mapped
UTF-8 range is within 64 KiB. Queries do not invalidate a clean scene. Selection
actions reuse Studio's checked UTF-16 conversion and dirty-only frame path.

The private `native_accessibility` adapter now translates these values into
stable, main-thread-only `NSAccessibilityElement` subclasses keyed by semantic
node identity and surface generation. The surface view owns a bounded cache;
each element keeps only a weak view reference, and close revokes the callback,
invalidates the generation, and releases every cached element. The adapter does
not retain Studio objects or expose native handles. It requests a fresh bounded
snapshot only after AppKit activates accessibility or accepted application
events change active semantics. Notifications are computed while reconciling
snapshots and posted only after the adapter borrow is released.

The code editor publishes character count and selection without materializing
the document. Selected text, bounded strings, logical lines, line ranges, and
grapheme ranges cross the exact revision-bound request path. Selection writes
carry the revision observed by the native element, and stale actions fail
without mutation. Unsupported text geometry selectors are explicitly denied.
The production native journey validates object identity, roles, labels, text
ranges, mapping selectors, stale writes, notifications, cache accounting, late
selector rejection, and owner drain. See AEP-0250 and AEP-0255.

Native text input carries one non-zero monotonic `InputEpoch` across the same
handle-free event boundary. Every IME start, update, commit, and cancellation is
tagged with the epoch active when AppKit produced it. On key-window loss,
occlusion, minimization, or close, the main-thread AppKit view first asks its
`NSTextInputContext` to discard marked text, suppresses reentrant `unmarkText`
commit, emits at most one cancellation for the old epoch, advances the epoch,
and only then publishes focus loss. Refocus reuses that newly established epoch
and cannot revive the discarded session.

Studio admits IME mutation only when the event epoch exactly matches its current
epoch and the window is focused. Stale and prematurely future events update
separate bounded counters but are otherwise atomic no-ops. Focus loss cancels
the one active composition owner among editor, find, quick open, command
palette, and project search before adopting the new epoch. Cancellation changes
frame demand only when visible composition or focus state changed; it performs
no GPU, filesystem, language-worker, clipboard, timer, or queue work. Native
handler revocation clears marked text and leaves input inactive even when no
application callback remains. See AEP-0268.

Semantic nodes now carry finite view-local rectangles plus explicit enabled and
activation capability bits. AppKit exposes a stable identifier composed from
surface, native-element-instance, and semantic identity; unchanged nodes retain
their native object, while removed, replaced, and surface-revoked instances can
never become valid again. Rectangle conversion to screen coordinates occurs
only in the private AppKit adapter. `Activate` carries the exact observed
document/buffer revision and node identity. Studio rebuilds its bounded current
projection, rejects stale, missing, unsupported, or disabled targets before
mutation, and routes only to existing tab, file-row, command, diagnostic,
save, and dirty-close authorities. A state-changing action then enters the same
document-authority advancement, language synchronization, bounded worker,
recovery publication, semantic-revision, and one-frame coalescing path as
keyboard and pointer input. Queries and unchanged actions remain dirty-neutral.
No `SetFocus`, arbitrary text geometry, callback registry, second semantic tree,
or generic element framework is introduced. See AEP-0270.

Accessibility refresh now returns one bounded native dispatch batch after the
adapter borrow ends. Removed and semantically replaced instances leave the
current identity set before their destruction posts. Layout posts target the
current root and carry `NSAccessibilityUIElementsKey` with only current bounded
affected elements; announcement posts carry bounded text and medium priority
under AppKit's required keys. Close invalidates every identity, posts destruction
outside the borrow while reentrant access fails closed, releases the batch, and
only then revokes the application handler. No post is admitted afterward.

Posting evidence records six per-kind invocation counters, a bounded
observer-facing protocol prefix, payload bytes, temporary retained bytes,
omissions, invalid user-info, and post-after-revocation violations. These values
prove AppKit call shape, not delivery to assistive technology. Notification work
requests no frame and touches no GPU, worker, timer, channel, or filesystem
authority. See AEP-0271.

The non-shipping native validation boundary can now inspect one complete current
AppKit element set and press one exact role-and-label target while returning
only bounded, handle-free evidence. The real Studio process journey uses that
boundary over one local workspace to expand the lazy file tree, open two Rust
files, navigate tabs, admit and activate a mock-server diagnostic, edit, execute
the existing save command, reject dirty close, save, close, and drain every
native and runtime owner. Stable accessibility queries submit no frame; each
accepted visible action enters the existing latest-frame coalescing path at most
once. The deterministic language fixture runs in a separate child process and
does not enter shipping code, startup, discovery, or dependencies. See AEP-0272.

Each validation query or named action performs one semantic refresh. It does not
preflight through `accessibilityChildren` and then refresh the same tree again.
This avoids redundant main-thread work and removes an artificial generation
race between target discovery and activation.

The production native accessibility request bridge rejects every clipboard and
close side effect. Snapshot, text, selection, and mapping queries must also
return no frame. One revision-valid action may carry one frame when its typed
result is `Applied` or `Unchanged`; that frame enters the same latest-scene
admission and display-link directive path as keyboard and pointer input. An
`Unchanged` action may only expose dirty work that was already pending, while a
failed action cannot submit a frame. This closes the fixture-hidden gap where
Studio mutated correctly but AppKit reported visible actions as rejected.

If a bounded worker result arrives immediately before an accessibility query,
runtime leaves it queued so the query observes the last complete state and
projection without attaching a frame. The already requested wake drains the
bounded result, builds one latest scene, and publishes projection identity only
after `rendered_lines` is final. An accessibility action is ordered before a
concurrent bounded drain, so its revision remains paired with the complete tree
that supplied the target. The drain then runs and action plus background effects
coalesce into at most one frame. The state/projection revision guard remains a
fail-closed recovery boundary, and the native adapter retains its previous
complete tree on a transient mismatch without reconciliation or notification.
This prevents new identity from accompanying old geometry, current native
controls from being rejected by unrelated concurrent publication, query-caused
frames, extra action frames, and idle redraw.

Native diagnostic validation never inspects the AppKit tree before complete
process-bound language authority is observable. It inspects only after that
authority on a newer complete semantic revision or a real requested frame.
Superseded language wakes are accepted only when their count is bounded by
actual foreground or latch observations; exact current-generation publication,
observation, empty pending state, and no restart remain mandatory. This proves
stale work was rejected without treating valid supersession as a qualification
failure or adding unconditional tree polling.

After the application accepts close, the native owner revokes accessibility
before publishing final focus loss. Runtime has already rejected new work at
that point, so this order prevents a focus callback from synchronously refreshing
an active semantic adapter against a shutting-down application.
The accepted close response itself also skips semantic refresh before returning
to `windowShouldClose`; a cancelled close still refreshes so its blocking status
is observable. This preserves `Allow` until AppKit enters `windowWillClose`.
Hosted validation invokes the same idempotent native close authority directly
after `windowShouldClose` accepts because headless AppKit may terminate its run
loop without delivering `windowWillClose`. Physical qualification still requires
the real AppKit callback.

### Pane document ownership (Task #127)

Pane leaves retain a stable document-tab identity and pane-local view state. The global document-tab store remains the sole owner of document payloads and buffers; panes never clone an editor or buffer. Scene construction resolves each pane identity to an immutable snapshot, while focus activates that identity through the existing checked tab transition. Selection state follows the document revision and is synchronized across panes showing the same tab, while scroll remains pane-local. Closing a tab retargets every referencing pane to the replacement active tab before the next scene is admitted.

The top-level tab strip controls the focused pane in this slice. Pane-local tab strips, duplicated document stores, GPUI-compatible entities, collaboration clocks, and a general reactive component graph are intentionally excluded. This keeps tab/pane composition bounded and local while leaving a narrow path to independent pane tab groups without changing buffer ownership.

### Local session persistence (Task #127)

Studio owns one private binary session manifest under the user's macOS application-support directory. Version 2 writes at most 32 tabs, four panes, seven split-tree nodes, 256 expanded directory identities, one selected file-tree identity, 4 KiB per path, 64 KiB of aggregate path bytes, and 128 KiB for the complete file. Runtime tab, pane, worker, cache, and filesystem identities are never serialized. Stable tab indices, fixed split nodes, active focus, directional selections, pane-local scroll, and strictly ordered UTF-8 root-relative tree paths are validated as one graph before publication. Version 1 remains readable and migrates to an empty tree snapshot so an upgrade cannot strand an existing recovery journal.

The payload carries a CRC-32 corruption check and is written through a unique mode-0600 temporary file, flush, file synchronization, atomic rename, and parent-directory synchronization. Restore occurs before native surface creation. Only the active tab and tabs visible in restored panes are opened before the first scene, with at most four unique visible documents under the fixed pane bound. Inactive non-visible tabs retain only validated path and view metadata until checked activation. Expanded tree paths restore as empty dormant nodes and the selected path remains an identity until checked directory results make it visible. No restored directory is enumerated before explicit tree activation, and then the existing one-request serial loader repopulates immediate directories under the same cache and byte ceilings. Missing or changed directories fail visibly, cannot open an unrelated row, and discard an unresolved selected identity after all admitted restoration work becomes terminal. A failed deferred document load leaves the active document and tab identity unchanged and the target deferred. These bounded active and visible file reads are still synchronous before surface creation; background restoration enrichment is not claimed by this slice. Missing, incompatible, corrupt, stale, or structurally invalid state cannot mutate files and falls back to a clean application with a bounded local diagnostic. Session capture occurs only after the event loop releases `StudioApp`, so persistence performs no typing or rendering work.

The clean session manifest remains unchanged while any tab is dirty. A separate private version-1 recovery journal retains the same validated session graph plus exact accepted-base and local UTF-8 bytes for at most 32 dirty documents. Revision-dirty documents remain journaled even when undo makes their local bytes equal the accepted base. Each base or local document is capped at 32 MiB, aggregate retained text is capped at 64 MiB, and an over-budget document degrades visibly rather than being truncated. Foreground event handling clones copy-on-write buffer snapshots and replaces one latest pending request. It never materializes text, waits for file I/O, or creates an unbounded queue. One owned worker materializes snapshots and performs mode-0600, checksummed, file-synchronized atomic replacement. Structurally equivalent session state and unchanged monotonic buffer revisions suppress redundant writes, including caret-only and scroll-only churn. Shutdown publishes the latest state and joins the worker before attempting the clean session manifest. An explicit file or folder launch refuses to replace an unresolved dirty or corrupt journal; launching Studio without a path remains the recovery entry point.

Recovery compares the exact retained base bytes with the current file, not a collision-prone summary. An unchanged file is reopened through the normal `Editor` authority, receives the recovered transaction, and retains its existing external-change and atomic-save protection. A modified, unreadable, invalid-UTF-8, or deleted file restores the local bytes into an explicitly conflicted document whose save operation fails closed; external bytes are never replaced or recreated. During dirty recovery, an unavailable prior workspace does not hide recoverable document tabs, and an unrelated clean active or visible file that became unavailable is represented by an empty, clean, save-blocked placeholder. Normal clean-session restoration remains strict, and non-visible unavailable clean tabs still fail checked activation. An unexpected failure while restoring a valid dirty journal aborts startup without replacing that journal. The local status reports recovered, conflicted, and unavailable counts. Atomic journal consistency is guaranteed, but asynchronous publication does not claim preservation of a keystroke when the process or machine fails before the corresponding generation reaches durable storage. Conflict-resolution commands and generation visibility in the diagnostic overlay remain follow-up daily-driver work.
