# AEP 0218: Bounded revision-safe Rust completion

- Status: Accepted
- Task: [#218](https://github.com/dbuddha/alpine-gpui/issues/218)
- Requirement: [#34](https://github.com/dbuddha/alpine-gpui/issues/34)
- Parent task: [#128](https://github.com/dbuddha/alpine-gpui/issues/128)
- Research: [#204](https://github.com/dbuddha/alpine-gpui/issues/204)

## Context

Alpine Studio needs useful local Rust completion without allowing a late or
oversized language-server result to mutate another editor revision, retain
unbounded JSON data, block the foreground, or create an idle redraw loop. The
merged local process, framing, JSON-RPC, rust-analyzer, wake, and diagnostic
boundaries already own transport and document synchronization. Completion must
compose those owners rather than introduce another process, queue, or document.

## Decision

Studio owns one private completion state inside its existing Rust language
session. Explicit completion input is admitted only for the active local Rust
document and captures workspace, document, buffer, selection, process
generation, process epoch, request, and LSP document-version identity. A newer
request cancels and locally revokes the older request before submission. Late
responses may release protocol ownership but cannot replace a newer result.

The foreground parses only the bounded response selected by the JSON-RPC peer.
It retains a compact list of Alpine-owned labels, optional documentation, and
one plain or insert-replace edit per item. Unsupported snippets, nonempty
additional edits, ambiguous edit shapes, malformed ranges, and oversized data
fail closed with bounded local status. Applying one selected item maps its LSP
UTF-16 range through the immutable Alpine buffer snapshot and performs one
revision-bound transaction through the existing editor undo path.

Completion uses the existing dirty-only scene builder. It adds one clip and at
most one background row plus bounded glyphs for each visible item. Focus loss,
document or selection change, supersession, cancellation, server restart, and
shutdown revoke pending and admitted completion before mutation or paint. No
accepted input or current response means no invalidation or frame submission.

No public Alpine API, dependency, process owner, async runtime, timer, polling
loop, network path, plugin boundary, AI path, cloud path, telemetry path, or
startup work is added.

## Locked limits

| Resource | Limit | Failure or pressure behavior |
| --- | ---: | --- |
| One completion JSON result | 1 MiB | Reject before item parsing |
| Admitted items | 64 | Retain the first bounded items and report omission |
| Visible rows | 8 | Project only the selected window |
| One label | 256 UTF-8 bytes | Reject the response item |
| One documentation value | 4 KiB | Reject the response item |
| One inserted replacement | 64 KiB | Reject the response item |
| Completion-owned retained bytes | 256 KiB | Stop before exceeding the budget |
| Active completion request | 1 | Cancel and revoke before supersession |
| JSON-RPC pending requests | Existing peer ceiling of 64 | Roll back failed admission |
| Cancellation tombstones | Existing peer ceiling of 64 | Fail visibly without growth |
| Scene clips | 1 | Omit the completion overlay on scene failure |
| Scene row quads | 8 | Paint only visible rows |

The existing process framing, queue, message, and retained-payload ceilings
remain authoritative. Completion does not widen them.

## Identity and cancellation

`LanguageIdentity` binds the workspace, document, buffer, and selection
revisions. The existing session adds process generation, process epoch, request
ID, URI, and LSP document version. Publication, row projection, accessibility,
and application all compare the complete current identity. Buffer revision zero
is valid for an unchanged newly opened buffer; owner and lifecycle identities
remain nonzero and monotonic.

Cancellation is locally authoritative even though LSP cancellation is advisory.
The peer removes the pending request, retains only a bounded cancelled-ID
tombstone, and classifies a later response as stale. A late cancelled response
cannot clear a newer admitted list because admitted ownership includes its exact
request ID.

## Edit and accessibility semantics

Plain insertion uses the current selection when the server omits a range.
`textEdit.range` and insert-replace edits use checked line-local UTF-16
conversion. Surrogate interiors, line overflow, reversed ranges, ambiguous
plain versus insert-replace shapes, snippets, and nonempty additional edits are
rejected before mutation. One accepted item is one atomic editor transaction
and one undo entry.

The completion list is a focused accessibility dialog. Its selected row owns
the bounded name and announcement. Up and Down change the selected item,
Return or Tab applies it, and Escape cancels it. When identity becomes stale or
focus leaves the window, the code editor regains accessibility focus and no
completion node is projected.

## Failure and lifecycle behavior

Malformed, unsupported, oversized, allocation-failed, stale, cancelled, and
unknown responses preserve document bytes and saving. Queue saturation rolls
back peer admission without blocking and exposes bounded local status. Process
crash or protocol failure uses the existing bounded restart path. Shutdown uses
the same production language-session drain invoked by `StudioApp::drop` and
releases pending requests, admitted items, retained bytes, and child ownership.

## Atomic claims and evidence contract

- **AEP-0218-C01:** Only the exact current Rust editor and process identity can
  publish or apply a completion; cancellation, supersession, focus loss,
  editor change, restart, and shutdown revoke stale work.
- **AEP-0218-C02:** Parsing, protocol ownership, retained completion values,
  visible rows, and scene additions remain within the locked ceilings, and
  saturation or pressure preserves editing and saving.
- **AEP-0218-C03:** Every accepted completion edit maps through checked Alpine
  coordinates and applies as one revision-bound, undoable transaction.
- **AEP-0218-C04:** The bounded list is keyboard and accessibility operable and
  one admitted completion frame returns to clean zero-idle state.

These claims require pure parser and coordinate controls, mock-process
supersession, pinned rust-analyzer compatibility, process saturation, retained
byte accounting, Studio scene, keyboard, focus, accessibility, atomic undo,
shutdown, and idle-frame controls. The finite completion-admission model proves
current-only publication, bounded items, close release, and current-only
application, with faulty late-publication and stale-application controls.
Changed-line coverage, viable mutation rejection, and hosted `ci-pass` remain
required.

## Explicit exclusions

Snippets, nonempty additional edits, command execution, resolve-on-selection,
completion caching across documents, automatic completion while typing, AI
completion, network language services, dynamic server discovery, plugins,
extension hosts, remote workspaces, telemetry, public framework completion APIs,
and cross-product performance claims are excluded.

## Reversal conditions

Reopen this decision if LSP or rust-analyzer requires a wider edit shape, if
dogfood shows explicit invocation is insufficient, if measured retained memory
invalidates a locked ceiling, or if Alpine changes the local-only process and
document ownership boundary. Any widening requires a superseding AEP and new
acceptance evidence.
