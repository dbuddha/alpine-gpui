# Alpine mechanism evidence ledger

## Reading the ledger

- Origin level describes the source finding.
- Alpine level describes implementation or reproduction evidence.
- Perf level is separate and never inherits from correctness evidence.
- `Pending E4` means no comparative dominance claim is allowed.

## Framework and renderer mechanisms

| ID | Alpine mechanism | Origin and lineage | Alpine modification | Evidence | Claim status and next gate |
| --- | --- | --- | --- | --- | --- |
| ALG-001 | Immutable SoA [`Scene`](source-map.md#alpine-source-anchors) | ZGP-S002, `ADAPTED-CONCEPT`, E2 | Narrow quads, clips, glyphs, ordered ops; no app/native handles | Scene validation, painter-order, CPU oracle, Metal readback; Alpine E3 | Semantic architecture proven for current primitives; realistic viewport comparator pending |
| ALG-002 | Demand-driven native surface | Zed invalidation and Apple display-link contracts, `ADAPTED-CONCEPT`, E2 | Latest-wins dirty state, pause at idle, explicit omission evidence | Lifecycle/model/native idle tests; Alpine E3 | Zero-idle invariant supported; energy and 120 Hz latency pending |
| ALG-003 | Three async frame slots | ZGP-S005 plus Apple triple-buffer guidance, `ADAPTED-CONCEPT`, E2 | Hard slot cap, generation/frame tokens, terminal reports | Unit, mutation, native presentation, lifecycle soak tooling; Alpine E3 | No main-thread completion wait supported; physical latency/residency pending |
| ALG-004 | Structured present recovery | WGPU surface-outcome discipline plus Apple lifecycle, `ADAPTED-CONCEPT`, E2 | Unsupported, unavailable, interrupted, stale, and unexpected outcomes preserve identity | Fault injection and lifecycle models; Alpine E3 | Correctness boundary supported; device-loss hardware reproduction incomplete |
| ALG-005 | One ordered instanced paint upload | GPUI batching, `ADAPTED-CONCEPT`, E2 | Quads and monochrome glyphs lower into one ordered representation | Shader ABI, lowering, oracle, Metal readback; Alpine E3 | Draw-call invariant supported; specialized pipeline comparison pending |
| ALG-006 | Visible-range text work | Zed editor/GPUI visible construction, `ADAPTED-CONCEPT`, E2 | Explicit line and overscan admission in editor-specific layout | Layout, scrolling, omission tests; Alpine E3 | Avoided offscreen work supported; physical scene-build profile pending |
| ALG-007 | Two-frame line cache | ZGP-S004, `ADAPTED-CONCEPT`, E2 | Byte ceiling and collision-confirmed rope range/content identity | Cache hit/miss/eviction tests; Alpine E3 | Reuse correctness supported; p99 shaping savings unqualified |
| ALG-008 | Lookup-first indexed A8 atlas | General/GPUI atlas pattern, `ADAPTED-CONCEPT`, E2 | Hash index plus deterministic storage/eviction metadata | PR #295 and #298 regressions, 10,000 warm frames; Alpine E3 | Zero warm rasterization in model supported; comparator CPU claim pending |
| ALG-009 | Row-delta atlas publication | No equivalent guarantee identified in reviewed GPUI boundary, `ALPINE-ORIGINAL`, E2 | `None`, `Full`, `Rows`, revision and byte identity | PR #300 deterministic row mutation tests; Alpine E3 | Zero warm publication and bounded miss rows supported |
| ALG-010 | Retained GPU atlas with row upload | General reusable GPU ownership, `ALPINE-ORIGINAL` modification, E2 | Full upload only for init/growth/recovery; bounded row updates otherwise | PR #301 Metal regressions; Alpine E3 | Upload-byte invariant supported; driver/GPU-time improvement pending E4 |
| ALG-011 | Local CoreText service | Apple platform requirement, `INDEPENDENT-CONVERGENCE`, E1 | Safe Alpine-owned shape/raster interface and top-down A8 orientation | Native text tests and orientation defect regression #293; Alpine E3 | Correct orientation supported; scratch allocation and fallback-cache profile pending |
| ALG-012 | Bounded runtime queues | No GPUI executor adaptation, `ALPINE-ORIGINAL`, E2 | Standard threads, fixed admission, external wake, stale-result rejection | Runtime model/property/mutation tests; Alpine E3 | Queue bounds supported; scheduling latency and wake cost pending |
| ALG-013 | Handle-free frame evidence | No reviewed equivalent guarantee, `ALPINE-ORIGINAL`, E2 | Submitted/completed snapshots cannot retain scenes or native handles | API and lifecycle tests; Alpine E3 | Ownership claim supported; instrumentation overhead pending |
| ALG-014 | CPU oracle and versioned trace | Comparator methodology, `ALPINE-ORIGINAL`, E2 | Immutable v1 solid-quad control plus v2 prepared clips, quads, A8 atlas, glyphs, and identity-bound scroll/resize pairs | PR #343, Alpine Zed Lab PR #5, and the exact hosted plus physical `assurance/lab/v2` record; composed E3 across eight fixtures | Prepared renderer semantic equivalence supported; atlas recovery, timing, memory, and E4 qualification remain #53 |
| ALG-015 | Event-to-present correlation | Apple timestamps and profiling practice, `INDEPENDENT-CONVERGENCE`, E2 | Event, submission, completion, presented-handler stages plus opt-in native terminal signposts | PR #307 and #312 plus Task #314 deterministic evidence; external capture not retained | Diagnostic-only; defect #304 physical distributions and causal correction block latency claims |
| ALG-016 | TLA+, Kani, mutation risk gates | Formal and mutation methods, `ALPINE-ORIGINAL` composition, E2 | State-machine models, bounded Rust checks, viable-mutant enforcement by risk | CI and formal effectiveness report; Alpine E3 for modeled properties | Does not replace production journey, hardware, fuzz, or performance evidence |

## Studio mechanisms

| ID | Alpine mechanism | Origin and lineage | Alpine modification | Evidence | Claim status and next gate |
| --- | --- | --- | --- | --- | --- |
| ALS-001 | Local revisioned rope buffer | Zed local editing behavior but not collaborative text internals, `INDEPENDENT-CONVERGENCE`, E2 | Ropey, immutable snapshots, local transactions and undo | Differential String model, Unicode/property tests, Miri where applicable; Alpine E3 | Local correctness supported; million-line and long-session physical memory pending |
| ALS-002 | Atomic save and dirty recovery | Editor durability requirement, `INDEPENDENT-CONVERGENCE`, E2 | Atomic replacement, external-change detection, bounded dirty journal | Filesystem fault and recovery tests; Alpine E3 | No known modeled data loss; sustained dogfood remains required |
| ALS-003 | Bounded workspace/tree | Zed workspace behavior, `ADAPTED-CONCEPT`, E2 | Lazy local inventory with per-path and aggregate byte caps | Fixture, mutation, restoration tests; Alpine E3 | Bounded Alpine-owned bytes supported; 100k-file responsiveness pending |
| ALS-004 | Tabs, splits, and session | Zed pane behavior, `ADAPTED-CONCEPT`, E2 | Small purpose-built layout and checksummed restoration | State, corruption, launch, recovery tests; Alpine E3 | Single-window behavior supported; multi-window excluded |
| ALS-005 | Quick open and project search | Zed/Sublime local-speed patterns, `ADAPTED-CONCEPT`, E2 | Streaming bounded workers, explicit truncation, result/path/read budgets | Fixture/property/mutation tests; Alpine E3 | Caps supported; ranking quality and large-repo latency pending dogfood |
| ALS-006 | Static command and settings schema | Zed settings lessons, `ADAPTED-CONCEPT`, E2 | Compile-time commands, no plugin registration, deterministic layers | Unit/mutation and no-bloat policy tests; Alpine E3 | Core implemented; safe reload/migration #222 open |
| ALS-007 | Built-in syntax cohort | Zed language behavior, `ADAPTED-CONCEPT`, E2 | Fixed Rust/Markdown/TOML/JSON/plain-text set, no extension API | Syntax and product-boundary tests; Alpine E3 | Cohort behavior supported; accuracy and large-file profile pending |
| ALS-008 | Bounded local LSP transport | LSP/JSON-RPC and Zed project behavior, `INDEPENDENT-CONVERGENCE`, E2 | One local child process, bounded framing/state, revision-tagged diagnostics, completion, navigation, and symbol results, plus canonical workspace-confined source navigation | Mock protocol, malformed input, lifecycle, pinned rust-analyzer tests, PR #345 exact-head run `32762895848`, merge `7db5e18f6da8e02cd171668d4714c745c55d7eda`, and Task #221 parser/model/process/scene evidence pending exact hosted identity; Alpine E3 for merged mechanisms and E1 for the unmerged symbol slice | Diagnostics, completion, hover, definition, references, and bounded document/workspace symbols implemented; rename/formatting publication #220 remains |
| ALS-009 | Native accessibility transport | AppKit/AX requirement, `INDEPENDENT-CONVERGENCE`, E2 | Bounded semantics, text mappings, actions, notifications, destruction | Snapshot/native model tests plus PR #322 exact-head real Studio process composition and hosted run `32675083043`; Alpine E3 | Production-process transport is supported for the accepted journey; physical VoiceOver and AXObserver evidence #253/#273 remain open |
| ALS-010 | Stable local app bundle | macOS product requirement, `INDEPENDENT-CONVERGENCE`, E1 | Revision-pinned assembly and explicit recovery launch | PR #306, PR #313, and Task #303 Finder launch evidence | Finder launch, dirty-close protection, save, and normal exit qualified; daily-driver, signing, and public release remain open |
| ALS-011 | No-bloat boundary | Sublime-like product decision, `ALPINE-ORIGINAL`, E2 | CI rejects GPUI, WGPU, Tokio, AI, collaboration, plugin, cloud, telemetry dependencies | Product-boundary CI, Alpine E3 | Dependency absence supported; speed and memory effect pending measurement |
| ALS-012 | Unified Studio event finalization | Studio document, language, recovery, semantic, and frame authority requirement, `ALPINE-ORIGINAL`, E2 | Ordinary input and state-changing accessibility actions share one finalizer; document authority advances before active-document language synchronization; read-only, unchanged, rejected, and stale actions remain mutation- and frame-neutral | PR #322 source head `c5cd78779e33bbbf7ea6296f50d63d08b7f727de`, focused regressions, 4 of 4 current event-finalizer mutants, 3 of 3 startup-prefix mutants, and hosted run `32675083043`; Alpine E3 | Scoped production-process authority and coalescing behavior supported; physical observer, latency, residency, and dogfood evidence remain open |

## Explicit non-evidence

The following observations must never be promoted automatically:

| Observation | What it proves | What it does not prove |
| --- | --- | --- |
| No GPUI/WGPU shipping dependency | A smaller declared dependency boundary | Faster startup, lower RSS, smaller binary, or lower maintenance cost |
| Cache byte cap | Alpine-owned retained bytes cannot exceed the modeled cap | Total physical footprint, allocator fragmentation, driver residency, or no leaks |
| Zero idle submissions in a model | No unnecessary Alpine frame request in modeled idle states | Whole-system energy use or every AppKit display transition |
| Zero warm rasterizations | CoreText rasterization is avoided after modeled admission | No shaping, allocation, scene build, upload, driver work, or presentation delay |
| One draw call | Submission is batched | Faster GPU time than GPUI on realistic text |
| Passing Kani/TLA+ | Bounded modeled properties hold | Native implementation equivalence, temporal performance, or absence of all races |
| Feature exclusion | Alpine has a narrower product | A measured CPU, memory, startup, or latency advantage |

## Evidence advancement queue

The next valid advances are:

- ALG-014: eight-fixture prepared-scene E3 to atlas lifecycle and recovery, then calibrated E4 paired GPUI comparison under #53.
- ALG-015: diagnostic-only to E3 physical event-to-photon reproduction, then E4 matched product comparison.
- ALG-008 through ALG-010: deterministic avoided work to physical CPU/GPU/upload/residency E3.
- ALS-001 through ALS-010 and ALS-012: production implementation to sustained revision-pinned dogfood E3, with ALS-009 and ALS-012 first requiring physical #273/#253 evidence.
- ALS-011: boundary evidence to normalized and stock-product memory/startup E4.
