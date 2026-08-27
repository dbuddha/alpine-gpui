# Renderer atlas lifecycle qualification

## Purpose

Task [#353](https://github.com/dbuddha/alpine-gpui/issues/353) extends the
immutable prepared-scene ladder with one companion protocol for resource
lifecycle correctness. It does not widen `alpine-scene-trace/v1` or v2 and does
not authorize timing, memory superiority, or dominance claims.

The canonical `alpine-scene-trace-sequence/v1` manifest contains six ordered
steps:

1. Full admission of one bounded A8 atlas.
2. Exact compatible reuse with zero atlas upload.
3. Same-capacity content replacement with a newer revision and content hash.
4. Capacity replacement with a newer revision and distinct dimensions.
5. Deterministic renderer teardown with zero terminal retained bytes.
6. Full resynchronization of the latest atlas under the next logical owner generation.

## Identity and ownership

Every visible step binds the exact scene path, workload hash, atlas resource,
content hash, dimensions, content revision, logical renderer generation,
expected upload bytes, and expected compact CPU image bytes. Teardown carries
no scene or atlas identity. Reconstruction must use the exact latest accepted
atlas and must perform one full upload.

The pure `alpine-trace` state contract rejects stale revision, noncontiguous
steps, incompatible reuse, partial identity, generation drift, nonzero terminal
retention, and arithmetic overflow. The assurance boundary additionally rejects
absolute, parent-relative, symlinked, missing, oversized, or non-v2 scene
references before decoding each scene through the independent CPU oracle.

## Native execution

`alpine-assurance render-trace-sequence-native` invalidates prior output before
validation, retains one Direct Metal backend across the first four visible
steps, shuts it down, creates a clean owner for resynchronization, and compares
every Metal image with its CPU oracle before publishing evidence. It records:

- Logical and backend generations.
- Submission identity.
- Compact CPU bytes.
- Native allocated bytes.
- Exact atlas upload bytes.
- Terminal retained bytes.
- Maximum observed CPU-oracle channel delta.
- Semantic and pixel-equivalence status.

The safe offscreen report does not expose native atlas handles or allocation
identities. The evidence therefore records that omission explicitly rather than
inferring allocation counts from upload activity. GPUI resource identity and
allocation counts remain the isolated lab's responsibility.

## Assurance map

- Rust unit and property tests cover every transition and identity axis.
- Kani selects whether compatible reuse changes content identity and proves the
  bounded validator rejects exactly that fault.
- Existing AEP 0028 TLA+ invariants continue to require lifecycle and resource
  equivalence before measurement.
- The CLI integration test proves the committed sequence reaches all five CPU
  oracle images through the public non-shipping binary.
- Physical Apple Silicon Direct Metal and pinned GPUI execution remain required
  before Task #353 reaches E3.

## Claim ceiling

The committed manifest and hosted validation establish protocol and CPU-oracle
correctness only. They do not prove GPU allocation identity, device-loss
recovery, elapsed time, memory footprint, presentation, input latency, 120 Hz
behavior, or Alpine superiority. Clean teardown and reconstruction are not a
simulated hardware device-loss claim.
