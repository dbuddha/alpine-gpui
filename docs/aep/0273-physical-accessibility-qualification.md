# AEP 0273: Physical accessibility qualification

- Status: Accepted protocol, physical evidence pending
- Decision: [#268](https://github.com/dbuddha/alpine-gpui/issues/268)
- Research: [#267](https://github.com/dbuddha/alpine-gpui/issues/267)
- Implementation: [#273](https://github.com/dbuddha/alpine-gpui/issues/273)
- Requirement: [#37](https://github.com/dbuddha/alpine-gpui/issues/37)

## Motivation

Hosted AppKit tests prove selector behavior and internal post intent. They do
not prove that an external accessibility client receives notifications, that
VoiceOver can operate the real editor, that actual sleep and wake preserve the
journey, or that native accessibility ownership drains on physical hardware.
This AEP defines the trusted-machine evidence boundary without introducing a
shipping dependency or another semantic tree.

## Goals

- Launch an exact Alpine Studio binary and attach externally by PID.
- Query bounded stable identifiers, roles, labels, focus, values, selection,
  actions, and hierarchy through `AXUIElement`.
- Observe real focus, value, selection, layout, announcement, hide, show,
  minimize, restore, destruction, and close delivery through `AXObserver`.
- Distinguish actual sleep and wake from injected lifecycle transitions.
- Retain exact tree, event, latency, residency, process, Inspector, VoiceOver,
  environment, revision, scenario, and binary identity.
- Keep all latency and residency results descriptive until A/A calibration
  activates a separately approved threshold.

## Non-goals

This AEP does not bypass macOS Accessibility permission, automate spoken or
braille output, add AccessKit, expose AX handles to Studio, add a shipping
dependency, introduce telemetry or network behavior, claim leak absence from a
finite soak, activate a performance budget, or qualify multi-window behavior.

## Boundary

The AX client belongs to the non-shipping `alpine-assurance` tool. Studio and
the macOS platform crate continue to publish only their existing production
AppKit accessibility surface. The client owns every copied AX value and
observer registration, retains one stale-element control, and releases all
external ownership after the target closes.

The preferred implementation uses target-only generated bindings for Apple's
ApplicationServices framework. Handwritten raw FFI or UI scripting is rejected
unless a separate review demonstrates that generated bindings cannot express
the required API. Adding the binding remains owner-gated and is not implied by
this protocol acceptance.

## Evidence bundle

`alpine-assurance validate-ax-evidence <bundle>` accepts schema
`alpine-ax-evidence/v1`. The manifest must bind Task #273, a full repository
revision, clean or retained-diff state, ordered capture time, Studio PID,
macOS/SDK/Rust/hardware identity, locale, input source, display, power,
thermal state, Accessibility trust, real sleep/wake, human VoiceOver and
Inspector attestations, and post-close drain.

Every retained artifact is a bounded regular non-symlink file addressed by a
bundle-relative path and lowercase SHA-256. Required artifacts are the exact
Studio and harness binaries, scenario, AX tree, event stream, latency samples,
residency samples, stdout, stderr, Inspector capture, human checklist, and the
repository diff when the tree is dirty. Serial numbers and credentials are
prohibited.

The tree format has one bounded row per current node and exactly one focused
node. The event format uses contiguous sequence numbers and strictly increasing
monotonic timestamps. Successful evidence must include focus, value, selection,
layout, announcement, hidden, shown, minimized, restored, actual sleep, actual
wake, destroyed-element, and close observations. Latency intervals may not run
backward. Residency samples retain physical-footprint and private-dirty bytes
without converting a finite distribution into a universal leak claim.

## Correctness and claim policy

The validator rejects absent AX trust, synthetic lifecycle substitution,
missing human attestation, unknown schema fields, path traversal, symlinks,
hash mismatch, duplicate tree identity, multiple focus owners, missing event
classes, non-monotonic samples, active latency budgets, and performance claims.
Structural validation proves only that the bundle satisfies this protocol.
Human and physical evidence remains reviewable evidence, not a cryptographic
proof that the operator performed each action.

Current-machine preflight on the implementation branch reported
`ax_trusted=false`. Therefore no physical qualification claim exists yet. The
trusted run begins only after the user grants Accessibility permission to the
actual harness executable or its stable parent application.

## Formal applicability

This AEP adds no shipping state machine. TLA+ does not model VoiceOver delivery,
macOS permission, elapsed time, or physical sleep. Kani does not prove FFI
behavior or external event delivery. Existing production lifecycle models and
mapping harnesses remain companions; this task adds native integration,
negative controls, retained artifacts, and human attestation instead of
performative formal coverage.

## Atomic claims

- **AEP-0273-C01:** One exact external harness attaches to one exact Studio PID
  only after macOS reports Accessibility trust and retains bounded current tree
  identity without exposing native handles to product code.
- **AEP-0273-C02:** Required external events, real sleep/wake, stale-element
  rejection, actions, close, and post-close drain are recorded with monotonic
  identity and exact artifact hashes.
- **AEP-0273-C03:** AX latency, physical footprint, and private-dirty samples
  remain descriptive and cannot activate a threshold or performance claim
  before approved A/A calibration.
- **AEP-0273-C04:** Accessibility Inspector and human VoiceOver attestations
  remain separate from automated AX evidence and cannot be synthesized by the
  validator.

## Acceptance and remaining work

Protocol parsing and fail-closed structural controls are the first slice. Task
#273 remains open until the external client, observer ownership, action and
stale-element controls, exact binary launch, physical lifecycle run, Inspector
capture, human VoiceOver checklist, residency/latency artifacts, negative
controls, native mutation evidence, and full repository gates are complete.

Reopen the architecture decision if external AX cannot discover Alpine's
stable identifiers, physical VoiceOver requires a missing production action,
observer callbacks cannot be bounded without a general runtime, or retained
ownership fails to return to the accepted post-close baseline.
