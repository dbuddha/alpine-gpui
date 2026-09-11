# Apple GPU sources and the Asahi boundary

Reviewed 2026-09-06 for Requirement #564 and Task #565. This is a source-based
engineering reference, not an Alpine reproduction or comparative result.

## Source hierarchy

| Source | Identity and use | Limit |
| --- | --- | --- |
| [Metal feature tables](https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf) | Apple document dated 2026-05-21; capability planning | Recheck against the actual SDK and `MTLDevice` capabilities before use |
| [Harness Apple GPUs with Metal](https://developer.apple.com/videos/play/wwdc2020/10602/) | Apple WWDC20 session 10602; tiling, attachments and partial renders | Architectural guidance, not a measurement of Alpine |
| [Resource fundamentals](https://developer.apple.com/documentation/metal/resource-fundamentals) | Public ownership/storage reference | Check resource type, usage and family; no universal storage-mode winner |
| [CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink) | Public scheduling API retrieval entry | Consult the installed SDK/current API documentation; targets are not actual presentation |
| [Dissecting the Apple M1 GPU, the end](https://alyssarosenzweig.ca/blog/asahi-gpu-part-n.html) | Alyssa Rosenzweig, 2025-08-26; first-person project retrospective | Driver achievements do not prove macOS Metal timing or Alpine correctness |
| [Part I](https://alyssarosenzweig.ca/blog/asahi-gpu-part-1.html) | Rosenzweig, 2021-01-07; early M1 reverse engineering | Early ISA observations and tentative explanations, not all-generation contracts |
| [The Apple GPU and the Impossible Bug](https://alyssarosenzweig.ca/blog/asahi-gpu-part-5.html) | Rosenzweig, 2022-05-13; controlled driver diagnosis | Historical driver fault, not a diagnosis of Alpine |

The feature table maps M1 to Apple7, M2 to Apple8, M3/M4 to Apple9 and M5 to
Apple10. This dated map is a lookup aid, not a substitute for runtime feature
detection or evidence on each chip. Record the consulted table/SDK revision and
device support; do not infer throughput, cache sizes or driver behavior from a
marketing generation or shared family.

## What the requested retrospective contributes

The author describes a progression from reverse engineering to conformant open
graphics drivers and gaming support. For Alpine, adopt the discipline of explicit
acceptance and demonstrated compatibility, not the Linux stack or its feature
scope. The retrospective itself contains little application-level Metal tuning.
It does not justify adding Vulkan, Mesa, a custom compiler or driver code to the
shipping editor. Keep such systems as bounded research inputs only.

## A useful debugging case, not an imported fix

The 2022 case rejects a plausible timeout explanation with a controlled long
shader, isolates pressure related to geometry and per-vertex output, then checks
partial-render handling against Metal and Apple guidance. Correcting one missing
path removes a fault but reveals a second rendering error. The lesson is to vary
one cause, exercise capacity boundaries, and keep checking output after a crash
disappears. Do not copy its private memory manipulation into Alpine tests.

Alpine hypothesis: editor-scale glyph/primitive pressure may expose costs that a
miniature fixture misses. Test this with supported counters, fixed output and
controlled sweeps only when the measured GPU stage warrants it. This is not proof
that Alpine currently has a tiled-vertex-buffer fault or needs a driver change.

## Apple-backed translation and exclusions

WWDC20 describes tiled vertex storage and partial renders, and recommends avoiding
unnecessary attachment loads/stores. Translate that into a review of Alpine's
existing pass and resource lifetimes, not a render graph. Transparent editor
primitives still require correct painter ordering; hardware visibility techniques
are not permission to reorder text or selection. Prefer existing batching until
representative counters demonstrate a problem.

The 2021 article explicitly contains inference about instruction throughput and
scheduling. Do not promote tentative M1 explanations into M4/M5 facts, assume
half precision preserves glyph correctness, or use undocumented opcodes in Alpine.
Use the Metal compiler and a checked error/precision corpus for any shader change.

Source agreement here supports methodological choices only. Each adopted
mechanism still needs the relevant source pin, implementation PR, correctness
control, measured result and historical lineage entry. Unknown proprietary driver
allocations stay omissions. No E3/E4 result is supplied by this reference.
