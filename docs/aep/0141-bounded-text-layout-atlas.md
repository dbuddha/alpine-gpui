# AEP 0141: Bounded text layout and monochrome glyph atlas

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#33](https://github.com/dbuddha/alpine-gpui/issues/33)
- Task: [#126](https://github.com/dbuddha/alpine-gpui/issues/126)
- Decision: [#141](https://github.com/dbuddha/alpine-gpui/issues/141)
- Research: [#27](https://github.com/dbuddha/alpine-gpui/issues/27), [#118](https://github.com/dbuddha/alpine-gpui/issues/118)

## Motivation and boundary

Alpine must shape and render only the visible editor viewport without importing
GPUI entities, a retained component tree, collaboration state, or unbounded
text caches. The implementation owns a safe portable layout and A8 atlas,
copies CoreText output into Alpine values, emits immutable scene arrays, and
samples the atlas through Direct Metal. This AEP makes no comparative
performance claim from functional and resource-bound evidence alone.

## Atomic claims

- **AEP-0141-C01:** A line-layout candidate is keyed by font, scale, wrap, byte
  length, and a streaming two-lane fingerprint, then confirmed by exact rope
  range comparison before reuse. A true current-frame or previous-frame hit
  performs no contiguous text materialization and no shaping call.
- **AEP-0141-C02:** Scroll, viewport height, fixed line height, document line
  count, and bounded overscan determine the only lines admitted for layout and
  paint preparation. Invalid scroll and arithmetic overflow fail structurally,
  and explicit byte and glyph ceilings bound one pathological line.
- **AEP-0141-C03:** The monochrome A8 atlas starts with zero pixel allocation,
  grows geometrically only within its total byte budget, reuses exact glyph
  keys, removes least-recently-used entries deterministically, and releases all
  pixel and metadata storage under explicit pressure.
- **AEP-0141-C04:** Direct Metal retains at most one cached GPU atlas per
  backend, reuses it only when revision, dimensions, and immutable pixel storage
  identity match, accounts instance and atlas uploads separately, and releases
  cached ownership under pressure without invalidating an in-flight command.
  Validation rejects dimensions beyond the Metal 3 guarantee and payloads over
  16 MiB before native allocation.
- **AEP-0141-C05:** One production-composed Studio process routes the real
  AppKit keyboard, text-input, IME, pointer, and scroll selectors through the
  bounded application runtime. The exact seven-event input sequence produces
  six and only six monotonically revised immutable frames, leaves no residual
  dirty frame, and atomically persists the expected Unicode document before
  the clipboard and close journey begins.

## Ownership and cache generations

`BufferSnapshot` exposes checked line byte ranges, deterministic streaming
fingerprints, and exact streaming range equality without exposing Ropey.
`LineLayoutCache` owns current and previous vectors of immutable copied layouts.
Beginning a frame drops the old previous generation and moves the current
generation into previous. A confirmed previous hit moves one entry back to
current; a miss alone materializes and shapes the line. The combined layout
payload and Alpine vector-capacity metadata remain below the configured ceiling.

`GlyphAtlas` owns one tightly packed square A8 pixel vector, removable entries,
free rectangles, a monotonic use sequence, and exact vector-capacity evidence.
Allocation reserves fallible metadata capacity before mutating free-space or
entry ownership. Failure therefore cannot consume an untracked tile. Eviction
clears pixels, returns the exact rectangle, coalesces adjacent free regions, and
then records the terminal counter.

## Correctness and formal applicability

Streaming fingerprints are admission filters, never proof of equality. Exact
range comparison is the collision guard. Unit tests force a matching candidate
fingerprint against different same-length text and require rejection. Two-frame
tests require pointer-identical layout reuse and exactly one shaping call.

The layout cache and atlas are synchronous local owners without independent
agents, fairness, or temporal progress, so a TLA+ model would be ceremonial.
Kani instead proves the compiled pure rectangle coalescing rule over all two
adjacent widths and one height in the bounded `u8 + 1` domain. Dynamic tests
exercise geometric growth, reuse, eviction, byte ceilings, and pressure drain.
This is not a proof of `Vec`, Ropey, allocation success, or unbounded geometry.

## Performance and memory

The design removes contiguous line allocation and native shaping from a true
cache hit, but no latency superiority is claimed. Visible-range admission and
line ceilings prevent document size from directly determining per-frame work.
Cache snapshots report current bytes, peak bytes, budget, generation entries,
hits, misses, evictions, and shaped lines. Atlas snapshots separately report
pixel bytes, metadata bytes, peak, budget, entries, hits, misses, evictions,
and pressure events.

The default layout ceiling is 32 MiB. The default atlas ceiling is 16 MiB, but
the atlas starts empty and grows only on demand. These are Alpine policy limits,
not statements about Zed or Sublime memory behavior.

The native backend retains one current R8 texture independently of the three
bounded presentation slots. Replacement allocates and uploads once, while an
unchanged frame reports zero atlas upload bytes. A pending command retains its
own texture reference, so pressure can remove the cache immediately without
freeing in-flight resources. Native snapshots expose current and peak atlas
bytes, allocations, uploads, reuses, and pressure releases.

## Platform, accessibility, and later integration

The ownership, cache, viewport, and atlas contracts are portable safe Rust. A
private Apple Silicon CoreText and CoreGraphics module copies shaped glyphs and
A8 bitmaps into Alpine-owned values. Scene glyph operations preserve painter
order and clipping, and Metal sampling is checked against the independent CPU
pixel oracle. The process journey composes native input and editor integration
with those independently qualified renderer contracts; it does not claim a
formal refinement from AppKit callbacks to pixels. Accessibility remains
outside this claim and is not implied by renderer or input evidence.

## Failure and reversal conditions

Invalid text ranges, line indices, scroll, metrics, glyph counts, bitmap sizes,
arithmetic, allocation, sequence advancement, budget, and atlas saturation are
structured failures. Rejected work never enters a cache generation or atlas
entry table. Pressure deliberately removes all atlas entries and storage.

Revisit the representation if fixed-hardware evidence shows linear cache probes
or full-atlas growth dominate accepted viewport journeys, if color fonts become
approved, or if native fixtures require a different copied-value boundary. Any
replacement must preserve collision-safe reuse, visible-range admission,
explicit ceilings, removable atlas ownership, and exact evidence.
