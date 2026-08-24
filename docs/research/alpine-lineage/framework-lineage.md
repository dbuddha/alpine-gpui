# Alpine GPUI framework lineage

## Direct capability comparison

This table is the declared 24-family accounting inventory. "Upstream" names a
conceptual influence or comparable mechanism, not copied implementation.

| ID | Capability family | Zed GPUI | Alpine state and source | Classification | Alpine change or boundary |
| --- | --- | --- | --- | --- | --- |
| F01 | Validated public values | Typed geometry and style values | Validated finite geometry, color, dimensions, counts, and structured errors in Alpine core and scene | `ALPINE-ORIGINAL` | Moves invalid-value rejection to narrow public constructors and preserves failure identity |
| F02 | Application lifecycle | Broad `App`, entity map, globals, windows, actions, executor | Narrow [`Application`](source-map.md#alpine-source-anchors) and `AppDelegate` | `INDEPENDENT-CONVERGENCE` | One window, synchronous main-thread mutation, bounded worker handoff, no globals or entities |
| F03 | Platform events | Cross-platform event and window abstractions | Typed macOS [`SurfaceEvent`](source-map.md#alpine-source-anchors) | `ADAPTED-CONCEPT` | Apple-first event identity, focus epochs, lifecycle generations, and explicit bounds |
| F04 | Demand invalidation | Dirty views/windows schedule work | Latest-wins dirty scene and paused display link at idle | `ADAPTED-CONCEPT` | Strict zero-idle default and observable omission/coalescing evidence |
| F05 | Frame ownership | Metal buffers reused after completion | Three completion-owned slots with stale-generation rejection | `ADAPTED-CONCEPT` | Hard bound, terminal status evidence, close/recovery ownership rules |
| F06 | Scene representation | Primitive-specific scene collections | Immutable SoA quads/glyphs/clips plus ordered operations | `ADAPTED-CONCEPT` | Smaller primitive vocabulary and no application/native handles in scenes |
| F07 | Painter order and clipping | Ordered batches and content masks | Explicit operation stream lowered into one ordered instance sequence | `INDEPENDENT-CONVERGENCE` | Stable semantic oracle and malformed-scene rejection are first-class |
| F08 | Primitive batching | Specialized primitive pipelines and reusable buffers | One instanced ordered draw for admitted quads and glyphs | `ADAPTED-CONCEPT` | Keeps one simple pipeline until measurement justifies specialization |
| F09 | Text shaping | Platform text system and font services | Alpine-owned CoreText shaping interface | `INDEPENDENT-CONVERGENCE` | Narrow native boundary with checked conversions and byte budgets |
| F10 | Glyph rasterization | Platform glyph rasterization feeding atlas | CoreText A8 rasterization with lookup-before-rasterize | `INDEPENDENT-CONVERGENCE` | Warm admission prevents CoreText entry; scratch-copy work remains a profiling target |
| F11 | Glyph atlas | Atlas storage, eviction, GPU resources | Indexed A8 atlas, deterministic eviction, byte accounting, row mutations | `ADAPTED-CONCEPT` | Explicit `None`, `Full`, or `Rows` publication and pressure behavior |
| F12 | Visible-range construction | Lists and editor build visible ranges | Bounded visible lines and overscan | `ADAPTED-CONCEPT` | No general list framework; editor-specific admission is explicit |
| F13 | Short-lived layout reuse | Current and previous frame line cache | Current/previous line layouts with collision-confirmed rope ranges | `ADAPTED-CONCEPT` | Byte ceiling and content confirmation prevent hash-only reuse |
| F14 | Background work | General async executor integrated with app context | Standard threads and bounded foreground/background queues | `ALPINE-ORIGINAL` | No Tokio or GPUI executor; results carry revision identity and admission bounds |
| F15 | Stale-result safety | Entity/version and application ownership mechanisms | Window, focus, document, workspace, request, and revision identities | `ALPINE-ORIGINAL` | Stale completions release resources but cannot publish current success |
| F16 | Accessibility | GPUI semantic/accessibility integration | Bounded snapshot, AppKit transport, text mappings, actions, notifications | `INDEPENDENT-CONVERGENCE` | Physical VoiceOver and AXObserver proof remains open |
| F17 | Clipboard and IME | Cross-platform platform integration | AppKit clipboard and marked-text lifecycle with focus epochs | `INDEPENDENT-CONVERGENCE` | Explicit byte caps, cancellation, and stale-composition rejection |
| F18 | Assurance and evidence | GPUI tests and renderer diagnostics | CPU oracle, semantic traces, mutation, Kani, TLA+, lifecycle reports, signposts | `ALPINE-ORIGINAL` | Stronger claim/evidence separation, but broad test cost is an active delivery risk |
| F19 | Retained entity graph | Central GPUI application model | Absent | `REJECTED` | Studio uses direct ownership; no compatibility graph will be added without product evidence |
| F20 | General element lifecycle | `request_layout`, `prepaint`, `paint` | No public `Element` layer | `DEFERRED` | Extract only after dogfood identifies repeated UI contracts |
| F21 | General style and flex layout | Rich style system and layout engine | Purpose-built editor geometry | `REJECTED` | Browser-style CSS/flex breadth is outside the prototype path |
| F22 | Animation and asset systems | GPUI animation and asset facilities | No generalized subsystem | `DEFERRED` | Static built-ins may be prepared offline; generalized animation is not justified |
| F23 | Multi-window and cross-platform runtime | macOS, Linux, Windows and multiple windows | Apple Silicon, single-window shipping scope | `DEFERRED` | M6 is explicitly outside the daily-driver critical path |
| F24 | Broad platform services and framework test runtime | Clipboard, menus, dialogs, URLs, executors, test contexts, many primitives | Only Studio-consumed services | `REJECTED` | Capability enters Alpine only with a concrete vertical-slice requirement |

## What was actually adapted from GPUI and Zed

Alpine directly credits these concept families:

- Demand-driven invalidation and frame coalescing.
- Ephemeral frame data separated from retained application state.
- Immutable, primitive-oriented scenes with painter order preserved.
- Direct Metal presentation with completion-owned reusable resources.
- Visible-range editor construction.
- Current/previous frame text-layout reuse.
- Glyph atlas reuse and eviction.
- Bounded batching and reusable upload storage.

Alpine did not import GPUI as a dependency and does not expose a compatible
API. It did not recreate `App`, `Entity<T>`, `View`, global registries, the
general element tree, the style/layout engine, the async executor, the full
primitive set, or cross-platform backends.

## What is new or materially modified in Alpine

| Mechanism | Difference | Current evidence | Qualification limit |
| --- | --- | --- | --- |
| Lifecycle generations and frame tokens | Completion may free stale resources but cannot publish stale success | Model, unit, mutation, and native lifecycle tests | No comparative latency claim |
| Hard three-slot frame admission | Queue and ownership cannot grow without bound | Deterministic accounting and failure-path tests | Physical residency and driver footprint remain open |
| Handle-free terminal evidence | Reports cannot retain scenes or native resources | API and lifecycle tests | Report overhead needs profiling |
| CPU pixel oracle plus versioned traces | Semantic admission is independent of Metal and comparator adapters | Eight-fixture composed E3 CPU, Alpine Direct Metal, and pinned GPUI Metal admission through PR #344 | Atlas lifecycle and recovery, timing, memory, and E4 qualification remain |
| Collision-confirmed line reuse | Reuse checks content in addition to range identity | Text-layout tests | Real editor cache-hit cost not physically profiled |
| Lookup-before-rasterize atlas admission | Warm glyphs avoid CoreText rasterization | 10,000-frame deterministic regression | CPU/GPU superiority over GPUI unqualified |
| Row-delta atlas publication and upload | Warm frames publish nothing; misses upload affected rows | Deterministic atlas and Metal regressions | Driver upload cost and texture alternative unqualified |
| Byte-budgeted product queues and caches | Search, files, LSP, settings, atlas, and layouts expose hard caps | Unit/property/mutation evidence | Whole-process physical footprint and user-visible degradation need dogfood |
| Formal assurance mix | TLA+ models lifecycle states; Kani checks bounded Rust transitions; mutation checks test sensitivity | CI effectiveness report | Assurance cost and missed production journeys remain risks |

## WGPU relationship

WGPU contributes no shipping code or runtime API. Its accepted E2 influence is
discipline, not implementation:

- Separate safe validation, core ownership, backend execution, and instance data.
- Treat surface outcomes and device loss as structured recoverable states.
- Keep resources alive until completion and reuse bounded staging memory.
- Split semantic validation from real-GPU backend qualification.

Alpine rejects WGPU's generalized WebGPU surface, backend portability matrix,
Naga/WGSL stack, broad trackers, and shipping abstraction cost for the Apple
v1 path. A WGPU backend may serve as a differential oracle only after realistic
trace semantics stabilize. See the [WGPU package](../wgpu/index.md).

## awesome-gpui relationship

awesome-gpui contributed workload discovery only. Editor and terminal entries
reinforce the need to test text, scrolling, IME, accessibility, focus, and
virtualization. Data clients suggest dense-list and memory-pressure cases.
Media and whiteboard entries suggest clipping, transforms, and embedded-surface
tests for future products. None of this proves that their architecture should
be copied, that GPUI is fast in those workloads, or that Alpine is faster.

## Framework conclusion

Alpine is recreating the minimum vertical path from macOS input through editor
state, immutable paint data, Direct Metal, and presentation. It is not
recreating most of GPUI as a general UI framework. That is the correct choice
until Studio dogfood demonstrates repeated layout, focus, and overlay contracts
that justify a small element layer.
