# Alpine Enhancement Proposals

An AEP is required when a Capability needs substantial ownership, state,
lifecycle, cross-platform, accessibility, performance, or formal-design work.
Small changes remain entirely in a GitHub Requirement.

Every major AEP defines stable atomic claim IDs, maps meaningful transition
properties into TLA+, identifies Rust and native boundaries, and specifies the
evidence required before its Capability can complete. An AEP is reviewed before
its Capability and Requirements receive `owner:approved`. The initial AEPs are
bootstrap exceptions authorized directly by the owner's approved plan.

An accepted AEP is historical. Later design creates a superseding AEP and links
the old one instead of rewriting the original rationale. Current implemented
truth always moves into `ARCHITECTURE.md` and rustdoc.

Accepted AEPs:

- [AEP 0009: Multi-layer assurance](0009-multi-layer-assurance.md)
- [AEP 0016: Portable value contracts](0016-portable-value-contracts.md)
- [AEP 0025: Direct Metal offscreen renderer](0025-direct-metal-offscreen.md)
- [AEP 0028: Zed golden qualification](0028-zed-golden-qualification.md)
- [AEP 0064: Native macOS presentation](0064-native-macos-presentation.md)
- [AEP 0120: Bounded asynchronous Metal presentation](0120-bounded-asynchronous-presentation.md)
- [AEP 0137: Bounded single-window Studio runtime](0137-bounded-studio-runtime.md)
- [AEP 0139: Checked local text buffer and one-file editor](0139-checked-local-text-buffer.md)
- [AEP 0141: Bounded text layout and monochrome glyph atlas](0141-bounded-text-layout-atlas.md)
- [AEP 0153: Bounded clipboard and close responses](0153-bounded-clipboard-close-response.md)
- [AEP 0160: Bounded local workspace foundation](0160-bounded-local-workspace-foundation.md)
- [AEP 0165: Bounded in-file find](0165-bounded-in-file-find.md)
- [AEP 0168: Bounded lazy workspace inventory and quick open](0168-bounded-lazy-workspace-inventory.md)
- [AEP 0171: Lazy bounded workspace file tree](0171-lazy-bounded-workspace-file-tree.md)
- [AEP 0177: Bounded static command palette](0177-bounded-static-command-palette.md)
- [AEP 0180: Bounded streaming local project search](0180-bounded-streaming-project-search.md)
- [AEP 0218: Bounded revision-safe Rust completion](0218-bounded-rust-completion.md)
- [AEP 0250: Bounded native accessibility transport](0250-bounded-native-accessibility-transport.md)
- [AEP 0255: Bounded native accessibility text mapping](0255-bounded-native-accessibility-text-mapping.md)
- [AEP 0268: Bounded native input and accessibility lifecycle](0268-bounded-native-input-accessibility-lifecycle.md)
- [AEP 0270: Bounded accessibility actions and geometry](0270-bounded-accessibility-actions.md)
- [AEP 0271: Bounded accessibility notifications and destruction](0271-bounded-accessibility-notifications.md)
- [AEP 0272: Real Studio native accessibility process journey](0272-studio-native-accessibility-process.md)
- [AEP 0273: Physical accessibility qualification](0273-physical-accessibility-qualification.md)

Proposed AEPs:

- None.

Required sections are motivation, journeys, goals, non-goals, atomic claims,
model, Rust ownership, correctness, accessibility, performance, memory, failure,
platform scope, evidence, mapping, risks, and reversal conditions. If no
meaningful states, actions, invariants, or progress properties exist, decompose
the work or keep it issue-only rather than creating a ceremonial model.
