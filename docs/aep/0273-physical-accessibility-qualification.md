# AEP 0273: Physical accessibility qualification

- Status: Accepted protocol, structural validator implemented, physical evidence pending
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
- Observe real focus, value, selection, layout, announcement, minimize,
  restore, destruction, and close delivery through `AXObserver` and process
  observation.
- Observe hide, show, actual sleep, and actual wake through `NSWorkspace`, not
  through injected Studio lifecycle events.
- Invoke a real AX action and prove that a retained stale element fails after
  its matching destruction observation.
- Retain exact tree, event, latency, residency, process, Inspector, VoiceOver,
  environment, revision, scenario, and binary identity.
- Keep all latency and residency results descriptive until A/A calibration
  activates a separately approved threshold.

## Non-goals

This AEP does not bypass macOS Accessibility permission, automate spoken or
braille output, add AccessKit, expose AX handles to Studio, add a shipping
dependency, introduce telemetry or network behavior, claim leak absence from a
finite capture, activate a performance budget, or qualify multi-window
behavior.

## Boundary

The future live AX client belongs to the non-shipping `alpine-assurance` tool.
Studio and the macOS platform crate continue to publish only their existing
production AppKit accessibility surface. The client owns every copied AX value
and observer registration, retains one stale-element control, and releases all
external ownership after the target closes.

The accepted client boundary uses target-only `objc2-application-services`
0.3.2 generated bindings for Apple's ApplicationServices framework, isolated
in the non-shipping `alpine-ax-client` crate under approved child Task #480.
Handwritten raw FFI and UI scripting remain rejected. The safe contract exposes
no native handles. Version 0.3.2 models `AXObserverGetRunLoopSource` as owned
even though Apple's Get rule returns borrowed ownership; the audited boundary
must suppress release of that generated temporary and establish exactly one
explicit Alpine retain before adding the source to the run loop.

## Evidence bundle

`alpine-assurance validate-ax-evidence <bundle>` accepts schema
`alpine-ax-evidence/v1`. `validate-ax-fixture` accepts only a manifest with
`fixture_only = true`; physical validation and reporting reject that same
manifest. A committed fixture can therefore prove parser and control behavior
without becoming physical evidence.

The manifest binds Task #273, a full repository revision, clean or
retained-diff state, ordered capture time, distinct Studio and harness PIDs,
normal Studio exit, macOS/SDK/Rust/hardware identity, locale, input source,
display, power, thermal state, Accessibility trust, real sleep/wake, separate
human VoiceOver and Inspector attestations, and post-close drain.

Every artifact is a bounded regular non-symlink file addressed by one unique
bundle-relative path and lowercase SHA-256. Every path component beneath the
bundle is checked so a symlinked parent cannot escape the evidence root.
Required artifacts are the exact Studio and harness binaries, scenario, AX
tree, event stream, latency samples, residency samples, stdout, stderr,
Inspector capture, human checklist, and the repository diff when the tree is
dirty. Serial numbers and credentials are prohibited. The bundle has an
aggregate byte ceiling, and text classes have narrower independent ceilings.

Tree, event, latency, and residency records are bounded JSON Lines with unknown
fields rejected. JSON escaping preserves real Unicode labels, commas, and line
breaks without an ambiguous comma-splitting protocol. Diagnostics are capped so
malformed bounded input cannot create an unbounded error vector.

## Tree and event contracts

A tree record binds contiguous sequence, depth, stable identifier, preceding
parent identifier, role, label, and focus. Exactly one `AXApplication` root,
one focused node, and application, window, and text-area roles are required.
Identifiers are unique and every child follows its retained parent at the next
depth.

An event record binds contiguous sequence, strictly increasing monotonic time,
source, kind, stable tree identifier, framework detail, and AX result. The
accepted source contracts are:

| Source | Required observations |
| --- | --- |
| `process` | Exact launch first and normal close last |
| `ax-observer` | Focus, value, selection, layout, announcement, minimized, restored, and destroyed |
| `ax-action` | A successful `AXPress`, `AXConfirm`, or `AXShowMenu` action |
| `ax-query` | A failed stale-element query after the matching destroyed element |
| `workspace` | Hide, show, `NSWorkspaceWillSleepNotification`, and `NSWorkspaceDidWakeNotification` |

The validator enforces hide before show, minimize before restore, actual sleep
before wake, destruction before the matching stale query, and stale query
before process close. Merely naming an injected Studio transition `sleep` does
not satisfy the source and framework-detail contract.

## Latency and residency contracts

Latency JSON Lines retain query, action, notification, stale-query, and close
intervals with exact identifiers and AX outcomes. Intervals cannot run backward.
No threshold, percentile gate, or performance conclusion is computed by this
slice.

Residency JSON Lines begin with one live startup sample, retain at least two
live finite steady samples, and end with exactly one post-close sample whose
process is absent and recorded process-owned bytes are zero. This is an exact
artifact-shape and observation contract. It is not proof of universal leak
absence, allocator behavior, framework autorelease timing, or favorable
long-session slope.

## Correctness and claim policy

The validator rejects absent AX trust, fixture substitution for physical
commands, synthetic lifecycle source substitution, missing human attestation,
unknown schema fields, path traversal, any internal symlink, hash mismatch,
duplicate artifact or tree identity, invalid hierarchy, multiple focus owners,
missing or reordered event classes, mismatched stale identity, non-monotonic or
oversized records, nonzero live or nonzero post-close contradictions, active
latency budgets, and performance claims.

Structural validation proves only that a bounded bundle satisfies this
protocol. Hashes prove retained bytes, not that the operator performed the
journey. Human and physical evidence remains independently reviewable. Current
machine preflight on the recovered branch reported `ax_trusted=false`, so no
physical qualification claim exists yet. A trusted run begins only after the
user grants Accessibility permission to the exact harness executable or its
stable parent application.

## Formal applicability

This AEP adds no shipping state machine. TLA+ does not model VoiceOver delivery,
macOS permission, elapsed time, or physical sleep. Kani does not prove FFI
behavior or external event delivery. Existing production lifecycle models and
mapping harnesses remain companions. This task adds bounded parsing, native
integration, negative controls, retained artifacts, and human attestation
instead of performative formal coverage.

## Atomic structural claims

- **AEP-0273-C01:** The non-shipping validator admits only one bounded,
  revision-bound, hash-complete tree and artifact set whose bundle-relative
  components cannot traverse symlinks; fixture and physical commands are
  mutually exclusive.
- **AEP-0273-C02:** The validator requires exact AXObserver, AX action, AX query,
  NSWorkspace, and process source contracts, ordered actual sleep/wake, one
  matching destroyed/stale-element control, and normal close.
- **AEP-0273-C03:** Latency and finite residency records are bounded,
  monotonic, descriptive, and post-close explicit while performance thresholds
  and claims remain structurally prohibited.
- **AEP-0273-C04:** Accessibility Inspector, human VoiceOver, and post-close
  attestations remain separate retained artifacts; fixture validation cannot
  produce a physical report.

These are validator claims. They do not claim that a physical run occurred or
that the referenced human statements are true.

## Acceptance and remaining work

The first slice is complete only when the fixture-only CLI, fail-closed unit and
integration controls, evidence registry, mutation, coverage, TLA applicability
review, and full exact-head CI pass. Task #273 remains open after that slice.

Remaining work is completion of the bounded generated-binding client in #480,
exact binary launch and artifact publication in #479, physical lifecycle run,
Inspector capture, human VoiceOver checklist, physical residency and latency
artifacts, negative controls, native mutation evidence, and revision-scoped
trusted-machine report.

Reopen the architecture decision if external AX cannot discover Alpine's stable
identifiers, physical VoiceOver requires a missing production action, observer
callbacks cannot be bounded without a general runtime, or retained ownership
fails to return to the accepted post-close baseline.
