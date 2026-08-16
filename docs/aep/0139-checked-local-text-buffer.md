# AEP 0139: Checked local text buffer and one-file editor

- Status: accepted 2026-08-15
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#33](https://github.com/dbuddha/alpine-gpui/issues/33)
- Task: [#125](https://github.com/dbuddha/alpine-gpui/issues/125)
- Decision: [#139](https://github.com/dbuddha/alpine-gpui/issues/139)
- Research: [#118](https://github.com/dbuddha/alpine-gpui/issues/118)

## Motivation and boundary

Alpine Studio needs a correct local editing state before shaping, rendering,
native input, syntax, or language services can consume it. Crop 0.4.3 was the
preferred candidate because it is byte-indexed and copy-on-write, but it failed
the accepted corpus: a nested slice can return unrelated text and its UTF-16
conversion accepts an offset inside a surrogate pair despite documenting that
case as invalid. Shipping Git dependencies are prohibited, so the unreleased
fix cannot be selected.

This AEP places exact-version Ropey 1.6.1 and Unicode Segmentation 1.13.3 behind
one safe `alpine-text` boundary. It does not expose third-party values, build a
custom rope, render text, map native events, or add collaboration, replica,
language-service, plugin, AI, cloud, or remote state.

## Atomic claims

- **AEP-0139-C01:** Byte offsets are canonical. A transaction applies only to
  its exact revision, validates every UTF-8 boundary and non-overlapping base
  range before mutation, and either publishes all text and selection changes
  under one newer revision or leaves the exact prior state intact.
- **AEP-0139-C02:** Immutable snapshots use Ropey's O(1) copy-on-write clone.
  Undo and redo are deterministic, always advance the live revision, and retain
  no more than the configured entry and changed-byte ceilings.
- **AEP-0139-C03:** One local UTF-8 file is fingerprinted at open and after each
  accepted save. A changed or deleted file is reported as a conflict. On the v1
  Unix platform family, save writes and synchronizes a same-directory temporary
  file, rechecks the accepted fingerprint, and atomically replaces the target;
  any pre-replacement failure leaves target bytes unchanged and cleans the
  temporary file.

## Coordinate and ownership rules

`Buffer` owns one Ropey tree, local monotonic revision, normalized selections,
and bounded history. `BufferSnapshot` owns a cheap immutable rope clone and its
captured revision. `Transaction` contains replacements in one base snapshot's
byte coordinates and an optional post-edit selection set. Replacements are
sorted, validated for overlap, and applied to a private copy from highest to
lowest byte offset before publication.

Every byte-to-character conversion must round trip to the original byte before
Alpine treats it as a boundary. AppKit uses a global UTF-16 code-unit offset;
LSP uses line-local UTF-16; line-column uses content bytes; grapheme conversion
uses extended grapheme clusters. Offsets inside UTF-8 code points, UTF-16
surrogate pairs, grapheme clusters, or CRLF are structured errors. Alpine does
not call a panicking third-party conversion with untrusted input.

## Dependency and unsafe boundary

Ropey 1.6.1 is MIT, has `smallvec` and `str_indices` as direct dependencies,
and contains third-party unsafe tree storage behind its safe public API. Alpine
disables default features, enables CR line metrics in every build, and enables
SIMD scanning in every non-Miri build. Miri exercises the same safe API through
Ropey's scalar path because the interpreter cannot execute the selected SIMD
intrinsics. Filesystem tests remain native because Miri isolation forbids their
syscalls.
Unicode Segmentation 1.13.3 is MIT OR Apache-2.0, has no required dependencies,
and supplies UAX #29 grapheme boundaries. Both support the workspace Rust
version. No unsafe code is permitted in `alpine-text`, and no dependency type
crosses its public API.

## Formal model and implementation mapping

[`LocalTextBuffer.tla`](../../formal/tla/aep-0139/LocalTextBuffer.tla)
models valid and rejected transactions, bounded undo and redo, monotonic live
revisions, external disk divergence, accepted save, and conflict rejection.
`RejectedIsAtomic`, `RevisionNeverDecreases`, `HistoryIsBounded`, and
`ConflictPreservesAcceptedDisk` map to `Buffer::apply`, `Buffer::undo`,
`Buffer::redo`, and `Editor::save`. `Faulty.cfg` changes content during a
rejected transaction and overwrites external disk identity during a conflict;
at least one safety invariant must fail. This is model checking of the finite
ownership abstraction, not a refinement proof of Ropey, Unicode, Rust, or a
physical filesystem.

Kani covers bounded selection transformation. The dynamic companion covers
multi-edit endpoint affinity and overlap rejection. A deterministic 1,000-edit
String oracle covers broad Unicode and line-ending sequences. Filesystem
integration covers successful atomic replacement, external modification, and
injected pre-replacement failure.

## Correctness, performance, and memory

Correctness gates precede editor latency claims. Snapshots avoid full-document
copies and edits retain Ropey's logarithmic tree behavior, but this AEP makes no
superiority claim. Coordinate conversion currently materializes text where a
simple independent implementation reduces risk; Task #126 may add measured
line and viewport caches without changing these semantics.

History exposes current entries, redo entries, retained changed bytes, and both
ceilings. The byte count describes changed payload retained by history, not
Ropey's allocator footprint. Large-file tests prove snapshot independence and
bounded history accounting, while fixed-hardware residency remains a later
qualification gate.

## Accessibility and platform scope

Explicit AppKit and LSP coordinate values preserve the identities required by
later input and accessibility tasks, but this AEP does not claim native IME,
VoiceOver, or text-range qualification. Text state is host portable. Atomic
replacement is implemented for the Apple-first Unix platform family; Windows
returns a structured unsupported result until a reviewed native replacement
boundary exists.

## Failure and reversal conditions

Stale revisions, invalid bounds, overlap, ambiguous Unicode coordinates,
revision exhaustion, invalid UTF-8 files, disk conflict, deletion, write,
flush, synchronization, and replacement failures are structured. A rejected
transaction cannot mutate text, selections, revision, or history. A failed
pre-replacement save cannot modify the accepted target.

Re-evaluate Crop after a crates.io release includes its slice fix and a
non-panicking UTF-16 contract and passes the same corpus. Re-evaluate Ropey if
dogfooding exposes a correctness defect, an unbounded residency slope, or a
measured conversion cost that dominates accepted editor journeys. Any
replacement must preserve this Alpine API and evidence corpus.
