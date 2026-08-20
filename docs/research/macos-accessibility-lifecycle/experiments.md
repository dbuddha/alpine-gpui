# macOS accessibility qualification experiments

## Lane 1: portable deterministic CI

Prove semantic identity, focus, actions, revisions, IME epoch ordering, dirty
neutrality, hard bounds, every focus owner, stale callback rejection, and
idempotent cancellation. Use unit/property fixtures, Kani, mutation, Miri, and
coverage. This lane makes no AppKit or VoiceOver claim.

## Lane 2: hosted Apple Silicon AppKit

Exercise production selectors, native marked-text discard, input loss/refocus,
actions, notification payload construction, destruction/revocation, injected
lifecycle ordering, the real Studio process, mutation, and owner drain.
Injected sleep or wake is labeled as injection. Direct selectors and internal
post counts do not prove external delivery or spoken output.

## Lane 3: trusted physical Apple Silicon

Use an Alpine assurance client built on `AXUIElement` and `AXObserver`, plus
human VoiceOver and Accessibility Inspector attestation. Exercise actual hide,
minimize, sleep, wake, action, focus, announcement, destruction, latency,
residency, and post-close drain journeys.

## Retained artifact identity

Retain repository and harness revisions, clean state, binary and harness
SHA-256, scenario hash, macOS/Xcode/SDK/Rust identity, hardware identity without
serial number, VoiceOver/Inspector/input-source identity, locale, display,
power, thermal and trust state, raw AX tree, notification stream, latency and
residency samples, stdout/stderr, Inspector captures, human checklist,
timestamps, and checksums.

Experiment on trusted hardware with notification ordering and coalescing,
rectangle sufficiency, accessibility-on residency delta, and calibrated AX
latency. Do not activate a blocking budget before A/A calibration.
