# AEP 0141: Bounded text layout and monochrome glyph atlas

- Status: accepted 2026-08-16
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Requirement: [#33](https://github.com/dbuddha/alpine-gpui/issues/33)
- Task: [#126](https://github.com/dbuddha/alpine-gpui/issues/126)
- Follow-up: [#299](https://github.com/dbuddha/alpine-gpui/issues/299)
- Defect: [#294](https://github.com/dbuddha/alpine-gpui/issues/294)
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
- **AEP-0141-C06:** Studio probes the atlas before native rasterization, retains
  both visible and empty raster outcomes with their copied bearings, and uses a
  monotonic pixel-content revision for publication. After cold admission, an
  unchanged viewport performs zero native glyph rasterizations, zero atlas
  publications, and zero GPU atlas uploads.
- **AEP-0141-C07:** Retained glyph lookup uses a deterministic Alpine-owned
  power-of-two index at no more than 50 percent load. Index slot capacity is
  included in the atlas metadata budget, collision probing never scans the
  atlas entry vector, swap-removal repairs entry identity without allocation,
  and pressure releases the complete index.
- **AEP-0141-C08:** Atlas pixel mutations retain at most 64 sorted, disjoint
  dirty-row ranges without heap allocation. Overlapping and adjacent ranges
  coalesce, saturation merges the smallest deterministic gap, and a compatible
  consumer receives only complete changed rows with exact byte evidence.
  Initialization, growth, source-revision mismatch, and explicit full-dirty
  state produce one full replacement. A stale acknowledgement cannot discard
  newer dirty evidence.
- **AEP-0141-C09:** Every immutable scene atlas is self-contained as one shared
  full base plus cumulative, sorted complete-row overrides. A dropped scene
  therefore cannot break update ancestry, and a new or recovered renderer can
  reconstruct current pixels without application or native handles. Direct
  Metal retains a compatible private atlas allocation and blits only the
  override rows; incompatible ancestry, dimensions, or storage force one
  checked full replacement.

## Ownership and cache generations

`BufferSnapshot` exposes checked line byte ranges, deterministic streaming
fingerprints, and exact streaming range equality without exposing Ropey.
`LineLayoutCache` owns current and previous vectors of immutable copied layouts.
Beginning a frame drops the old previous generation and moves the current
generation into previous. A confirmed previous hit moves one entry back to
current; a miss alone materializes and shapes the line. The combined layout
payload and Alpine vector-capacity metadata remain below the configured ceiling.

`GlyphAtlas` owns one tightly packed square A8 pixel vector, removable positive
and metadata-only empty entries, a deterministic open-addressed key index, free
rectangles, monotonic use and pixel sequences, and exact vector-capacity
evidence. The index remains at or below 50 percent load and resolves a retained
key through bounded collision probing without a linear entry scan. A lookup hit
returns the retained rectangle and native bearings before CoreText is entered.
An absent lookup is counted as a miss only when its native raster outcome is
admitted.
Allocation reserves fallible metadata capacity before mutating free-space or
entry ownership. Failure therefore cannot consume an untracked tile. Eviction
repairs the index without allocation, clears pixels, returns the exact rectangle,
coalesces adjacent free regions, and then records the terminal counter.

## Correctness and formal applicability

Streaming fingerprints are admission filters, never proof of equality. Exact
range comparison is the collision guard. Unit tests force a matching candidate
fingerprint against different same-length text and require rejection. Two-frame
tests require pointer-identical layout reuse and exactly one shaping call.

The layout cache and atlas are synchronous local owners without independent
agents, fairness, or temporal progress, so a TLA+ model would be ceremonial.
Kani instead proves the compiled pure rectangle coalescing rule over all two
adjacent widths and one height in the bounded `u8 + 1` domain, plus index probe
containment for every start and probe in power-of-two tables through 128 slots.
Two additional compiled harnesses prove bounded sorted dirty-row transitions
and the saturated merge path. Their three required covers distinguish
multi-range retention, capacity-preserving insertion, and overlap reduction.
A deterministic dynamic model exercises collisions, membership, use ordering,
swap-removal, eviction, byte ceilings, and pressure drain across mixed
operations. This is not a proof of hash distribution, `Vec`, Ropey, allocation
success, or unbounded geometry.

## Performance and memory

The design removes contiguous line allocation and native shaping from a true
cache hit, but no latency superiority is claimed. Visible-range admission and
line ceilings prevent document size from directly determining per-frame work.
Cache snapshots report current bytes, peak bytes, budget, generation entries,
hits, misses, evictions, and shaped lines. Atlas snapshots separately report
pixel bytes, metadata bytes, peak, budget, entries, hits, misses, evictions,
pressure events, and exact pixel-content revision.

The default layout ceiling is 32 MiB. The default atlas ceiling is 16 MiB, but
the atlas starts empty and grows only on demand. These are Alpine policy limits,
not statements about Zed or Sublime memory behavior.

Studio publishes immutable atlas pixels only when the pixel-content revision
changes; it does not compare or copy the complete image on a warm frame. It
acknowledges a full publication as the retained base, then carries cumulative
bounded row overrides without cloning that base after each glyph miss. Direct
Metal retains one current private A8 buffer independently of the three bounded
presentation slots. Compatible revisions allocate only bounded staging bytes
and encode one blit per changed row range. Initialization, growth, recovery,
incompatible ancestry, or dimensions allocate and upload one reconstructed full
image. An unchanged frame reports zero atlas upload bytes. A pending command
retains its own buffer references, so pressure can remove the cache immediately
without freeing in-flight resources. Native snapshots expose current and peak
atlas bytes, allocations, uploads, reuses, and pressure releases. Parent task
#297 retains physical-device and end-to-end typing qualification.

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
