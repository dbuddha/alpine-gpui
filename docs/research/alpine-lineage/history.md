# Historical implementation and evidence log

This is append-only at the event level. Later corrections may mark an event
superseded but must not erase the earlier claim or evidence state.

## Foundation and comparator history

| Date | Change | Origin classification | Evidence change | Current interpretation |
| --- | --- | --- | --- | --- |
| 2026-08-09 and earlier | Research #5, #18, #23, and #24 established GPUI, Zed, WGPU, and awesome-gpui boundaries | E1/E2 source research | No comparative performance evidence | Architectural inputs only |
| 2026-08 | PRs #54-#57 established validated Direct Metal offscreen rendering and independent CPU oracle | `ALPINE-ORIGINAL` plus standard Metal convergence | Deterministic scene/readback evidence | M1 implementation exists, comparison remains open |
| 2026-08 | PR #73 created one AppKit and CAMetalDisplayLink surface | `ADAPTED-CONCEPT` | Native lifecycle evidence began | Foundation retained |
| 2026-08 | PRs #87-#92 added epochs, SDR handling, recovery, and cancellation | `ALPINE-ORIGINAL` strengthening | Failure identity and lifecycle models expanded | Retained |
| 2026-08 | PR #112 shipped the first real application executable | Independent product work | Production launch path existed | Superseded by richer app bundle and recovery launch |
| 2026-08 | PRs #135 and #136 replaced blocking presentation with bounded async slots | `ADAPTED-CONCEPT` from completion-owned rendering | Main-thread `waitUntilCompleted` removed from normal presentation | Retained, physical latency pending |
| 2026-08 | PR #138 added the bounded single-window runtime | `ALPINE-ORIGINAL` narrowing | Runtime transition and queue evidence | Retained, no GPUI entity graph |

## Editor vertical-slice history

| Date | Change | Evidence change | Supersession or remaining gap |
| --- | --- | --- | --- |
| 2026-08 | PR #140 added local checked text | Differential local buffer evidence | Large-file and physical residency remain |
| 2026-08 | PR #142 added visible-range CoreText layout, scene glyphs, and atlas | Text rendering became real | Initial atlas hot path later proved inefficient |
| 2026-08 | PRs #144, #145, and #149 added native input and one-file editing | Production editing journey | Physical smoothness was not yet measured |
| 2026-08 | PRs #155, #157, #158, and #159 added clipboard, close, save, and lifecycle behavior | Data-safety and close evidence | Sustained dogfood remains |
| 2026-08 | PR #162 added the local workspace | Workspace fixtures and bounds | Expanded through later shell slices |
| 2026-08 | PR #164 added tabs | State and rendering evidence | Retained |
| 2026-08 | PR #166 added bounded in-file find | Search state evidence | Retained |
| 2026-08 | PR #170 added quick open | Lazy inventory and ranking evidence | Large-repo latency pending |
| 2026-08 | PR #173 added a lazy file tree | Virtualization and retained-byte bounds | Retained |
| 2026-08 | PR #179 added a static command palette | No dynamic command registry | Retained |
| 2026-08 | PR #182 added streaming project search | Explicit read, result, path, and batch bounds | Dogfood quality and latency pending |
| 2026-08 | PRs #186-#192 added splits, pane tabs, session, dirty recovery, tree restore, and folder launch | Local workspace shell became useful | M3 accepted |

## Daily-driver and qualification history

| Date | Change | Evidence change | Current gap |
| --- | --- | --- | --- |
| 2026-08-18 | PR #193 added bounded compiled syntax | Built-in language cohort | No extension grammar path by design |
| 2026-08-18 | PRs #194-#196 centralized settings, shortcuts, and deterministic layering | Static typed configuration evidence | Reload and migration #222 open |
| 2026-08-18 | PR #199 enforced the no-bloat dependency boundary | CI evidence of excluded dependencies | Performance effect not measured |
| 2026-08-18 to 2026-08-19 | PRs #197, #201, #206, #207, #209, #217, and #231 added bounded local rust-analyzer, JSON-RPC, diagnostics, and completion | Real local language path | Hover/navigation, rename/format, symbols remain |
| 2026-08-18 | PRs #213 and #216 admitted bounded external results and woke the main loop | Background work reaches UI without polling | Scheduling latency pending profiling |
| 2026-08-19 | PRs #245-#249 added production native journey, SDR, resource, soak, and idle evidence lanes | Native assurance improved | Several physical hardware tasks remain open |
| 2026-08-20 | PRs #254, #256, #264, #274-#276 added accessibility transport, mappings, focus cancellation, actions, and notifications | M4 implementation substantially complete | Physical VoiceOver/AXObserver and real-process gates open |
| 2026-08-22 | PR #322 composed the real Studio workspace, runtime, AppKit accessibility, Direct Metal frame, local language server, save, close, and owner-drain journey | Production-process accessibility evidence covers current semantic identities, accepted actions, zero-frame queries, coalesced visible frames, exact file bytes, and terminal ownership; explicit hosted-direct presentation remains non-physical | Physical AXObserver and VoiceOver Tasks #273 and #253 remain |
| 2026-08-21 | PR #287 measured formal-test effectiveness | Assurance controls became observable | Cost/benefit needs continued review |
| 2026-08-21 | PRs #288-#291 improved native failure, idle, and residency evidence | Better lifecycle diagnostics | Locked/headless session remains a fragile physical lane |

## Text hot-path correction history

| Date | Defect or correction | Before | After | Evidence ceiling |
| --- | --- | --- | --- | --- |
| 2026-08-21 | PR #293 fixed inverted CoreText glyph orientation | Glyph bitmap orientation produced flipped text | Top-down logical A8 orientation is tested | E3 correctness |
| 2026-08-21 | PR #295 moved atlas lookup before rasterization | Every visible glyph could enter CoreText before hit detection | Warm glyphs bypass rasterization | E3 deterministic avoided work |
| 2026-08-21 | PR #298 indexed retained glyphs | Atlas lookup was linear | Keyed lookup with deterministic storage metadata | E3 complexity/invariant, no physical speedup claim |
| 2026-08-22 | PR #300 retained atlas row mutations | Publication compared or copied broad atlas state | No/full/row update identity and bytes | E3 deterministic bandwidth bound |
| 2026-08-22 | PR #301 retained GPU atlas storage and uploaded row deltas | Atlas revision could trigger complete recreation/upload | Compatible allocation reused, dirty rows uploaded | E3 implementation, GPU improvement pending E4 |

## Launch and latency history

| Date | Change | Evidence change | Current interpretation |
| --- | --- | --- | --- |
| 2026-08-22 | PR #306 assembled a stable dogfood `.app` | Revision-pinned local application bundle exists | #303 remains open and should be reconciled |
| 2026-08-22 | PR #307 correlated event-to-present stages | Latency stages can be distinguished | Physical external trace still absent |
| 2026-08-22 | PR #312 emitted release signposts | Instruments-compatible signposts exist | Local machine lacks full Instruments tooling in the reviewed environment |
| 2026-08-22 | PR #313 composed explicit launch and recovery | Release launch enters production recovery path | Visible lag #304 remains the P0 product defect |
| 2026-08-22 | Research #315 created this lineage package | Origin, implementation, evidence, and history become queryable | Must be updated with every material mechanism or qualification change |
| 2026-08-22 | Task #314 exposed native terminal latency stages to the release signpost stream | Event-handler, frame-queue, submission, GPU-observation, presented-handler, and terminal-record durations are externally capturable without in-process sample retention | Deterministic implementation evidence only; physical distributions and causal correction remain #304 |
| 2026-08-22 | Task #303 launched the exact revision-pinned release bundle through Finder, exercised dirty-close refusal, saved, and exited normally | Physical launch evidence retained exact revision, bundle identity, executable digest, OS, and hardware | Launch is qualified only; typing latency, accessibility, signing, packaging, and daily-driver acceptance remain open |

## Native accessibility correction history

| Date | Defect or correction | Invalid or superseded evidence retained | Accepted evidence and claim ceiling |
| --- | --- | --- | --- |
| 2026-08-23 to 2026-08-24 | PR #322 unified state-changing accessibility actions with Studio's ordinary event finalizer after Defect #323 showed that tab and file actions could bypass document-authority advancement, active-document language synchronization, bounded worker submission, recovery publication, semantic invalidation, and one coalesced frame | A test selector without the `tests::` prefix selected zero tests; a process-contract selector without `alpine_native_validation` selected zero tests; the original `open` omission assumed no language process could start; one initial document-admission polarity mutant survived until the duplicated negation was replaced by the independently tested admission helper; the first complete gate exposed missing Miri ownership | Exact source head `c5cd78779e33bbbf7ea6296f50d63d08b7f727de` passed local production-path tests, 4 of 4 event-finalizer mutants, 3 of 3 startup-prefix mutants, and hosted run `32675083043` with 44 of 44 jobs green before squash merge `2836416b14e4b54172c1fe617e40e3e86611921c`. This advances only scoped production-process correctness to E3; physical AXObserver, VoiceOver, 60/120 Hz latency, residency, dogfood, and comparison remain open in #273 and #253. |

## Rules for future entries

- Append the merge date, PR, mechanism IDs, and evidence level transition.
- Mark the previous mechanism superseded when behavior changes.
- Retain failed or invalidated experiments with the invalidation reason.
- Record the selected and executed test count; zero-selected commands are invalid evidence even when the command exits successfully.
- Distinguish the source head under review from any synthetic merge-test revision used to name hosted artifacts.
- Never rewrite an E2 design observation as if it had always been E4.
- Link raw evidence identities rather than pasting benchmark headlines.
