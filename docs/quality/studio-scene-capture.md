# Studio production-scene capture

This validation-only diagnostic supplies production geometry for
[Task #577](https://github.com/dbuddha/alpine-gpui/issues/577). It does not itself
produce an admitted `alpine-scene-trace/v2` workload, benchmark sample, physical
presentation result, or milestone acceptance.

## Boundary and scope

The hook observes a successful `StudioApp` implementation of
`AppDelegate::frame`, after its real scene builder returns. It preserves logical
geometry, ordered paint operations, clip references, glyph sampling rectangles,
colors, scene revision, and the editor's rendered-line count. It is compiled
only with `alpine_native_validation`; ordinary shipping builds have neither the
hook nor the exporter. There is no new dependency or public runtime API.

Set `ALPINE_STUDIO_NATIVE_SCENE_CAPTURE_DIR` to a new private directory before
launching a validation build. Unset means no capture. The process admits at most
16 capture attempts, including failed attempts. Each completed observation has
`scene-PID-NNNN.json` and, when a glyph atlas exists, `scene-PID-NNNN.a8`.
The JSON is an internal `alpine-studio-scene-capture/v1` raw-observation envelope,
not a new renderer protocol. It explicitly declares
`renderer_trace_admitted = false` and `timing_invalidated = true`.

The A8 sidecar contains the current image: base pixels with **all cumulative row
patches** applied. It does not export only the latest delta or silently return
an older base. Rows are streamed with bounded scratch space, rather than cloning
the full image or expanding each byte into a JSON number. The exporter retains
no scene or native handle after the call.

## Resource and failure bounds

| Resource | Bound |
| --- | ---: |
| Capture attempts per process | 16 |
| Paint operations per capture | 65,536 |
| Clips per capture | 4,096 |
| Cumulative atlas row patches | 64 |
| A8 sidecar per capture | 16 MiB |
| Geometry JSON per capture | 32 MiB |
| Buffered output per file | 32 KiB |

The directory must be an experiment-owned private directory. The exporter does
not create or clear it. Files use create-new semantics and owner-only Unix
permissions. Bytes are flushed and synchronized before hard-link publication,
which cannot overwrite an existing capture. Failed writes/publications leave
`.incomplete` files, and failed geometry publication may leave an A8 sidecar.
Retain these as rejected observations, not successful traces. The writer does
not claim crash-durable directory metadata or protection against an attacker
who can replace the parent directory.

A fallback clears the editor's rendered-line projection and is rejected by the
capture admission check. Consumers must still validate every JSON/sidecar pair,
count, identity and omission. File presence alone is not acceptance. Once the
16-attempt bound is reached, later frames are not captured; this does not stop
or change ordinary application rendering.

## Safe collection

Use a clean, revision-pinned checkout and an isolated Cargo target. Record the
exact source, Rust flags, compiler, final bundle and executable digests in the
external capture manifest. Build with `RUSTFLAGS='--cfg alpine_native_validation'`
and the existing `scripts/build-alpine-studio-app.sh --executable PATH` assembly
path. Never use the fixture-revision override to relabel an uncommitted build.

Before visible collection, confirm the permitted unlocked desktop. Use only
disposable copies of the selected Alpine repository documents and a private
HOME. Bind the child PID, process start, executable and native window; an app
name or shared bundle identifier does not establish ownership. Do not interact
with or close other Studio instances, particularly recovered dirty sessions.
Use the production close path for the owned child and retain failures.

Capture normal code, dense code, selection/caret, a small scroll delta and
resize from an explicitly recorded interaction sequence. Record which frame
was selected and why. Do not automatically use an arbitrary last frame, combine
different processes, or call a synthetic reconstruction a native capture.

This code synchronously writes diagnostic files during frame construction.
**Do not collect timing, energy or residency qualification from an enabled
capture build.** Produce the immutable workloads first; ordinary, instrumented
and comparison runs have their own identities and calibration.

## Admission still required

`Scene` intentionally contains no native window, backing-scale or font-file
identity. Those fields are explicit omissions, not guessed from a display's
advertised refresh rate or the window's decorated frame bounds. The external
manifest must bind actual backing scale, viewport pixel dimensions, font and
glyph inputs, document identity, interaction history, source and executable
identities, and hashes of both raw files before workload admission.

Convert selected observations using the existing renderer protocol and its
canonical hash rules. Preserve painter order and all materialized atlas bytes.
Retain the historical 64-by-32, seven-glyph miniature trace unchanged, including
its unfavorable full submit-readback result. Require independent CPU oracle,
Alpine Metal and pinned GPUI semantic/pixel equivalence and negative controls
before any timing. Decoding and adaptation remain separate endpoints.

Task #577 remains open until the complete representative workload package and
correctness/provenance disposition are retained and exact-head/exact-main CI
pass. Residency belongs to #471 and comparative qualification to #472; their
four-window/twenty-paired-run minimum is unchanged. See the
[comparator protocol](comparator-protocol.md).

## Regression gates

Unit controls cover bounded admission, streamed byte limits, cumulative row
materialization, exact geometry/order/clip serialization, retained partial
failures, non-overwrite publication, and rejection of fallback projections.
The native CoreText test checks real glyph output and independently reconstructs
the expected atlas from its base and cumulative patches.

The existing native shipping-process journey enables a private capture and
requires a valid PID-bound geometry/A8 pair from the real frame hook. Removing
the hook, changing its process identity, losing the atlas, corrupting counts or
turning raw observations into a claimed qualification makes that gate fail.
The native fixture is a correctness control, not a representative editor-scale
workload or physical-performance qualification.
