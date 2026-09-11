# Pinned inspection routes

These are search entry points, not a claim that a newer release keeps the paths.
The accepted comparator remains the repository's approved pin. The historical
Zed v1.15.0 source is `e17dc4f9d50db73a458b64dcce50ecd4878b98a3`.

| Question | Source to inspect with callers and tests |
| --- | --- |
| Painter order, primitive batches, clipping | [GPUI scene](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/scene.rs) |
| Uploads, resource reuse, commit/wait/readback | [macOS Metal renderer](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui_macos/src/metal_renderer.rs) |
| Text layout reuse and cache keys | [Line-layout cache](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/gpui/src/text_system/line_layout.rs) |
| Local state versus collaboration overhead | [Text buffer](https://github.com/zed-industries/zed/blob/e17dc4f9d50db73a458b64dcce50ecd4878b98a3/crates/text/src/text.rs) |
| Element phases and invalidation | Locate `Element`, window invalidation and layout/prepaint/paint entry points in the pinned GPUI tree |
| Native input and display | Trace the pinned platform window, input handler and macOS backend through actual delegates |
| Unsaved cross-file language behavior | Trace project language-server ownership, buffer registration and revision publication; editor mocks alone are insufficient |

Zed's [120 Hz investigation](https://zed.dev/blog/120fps) and
[game-inspired UI explanation](https://zed.dev/blog/videogame) are author
explanations. Check the pinned code and local experiment before applying a
historical diagnosis to Alpine. An investigation of a past synchronous wait does
not establish the cause of a current missing presentation timestamp.

Read existing Alpine case studies and lineage before expanding research. If the
question is a known measurement-boundary or fixture-size defect, fix that evidence
gap instead of producing another general architecture comparison. Log both
successful and rejected adaptations; label WGPU validation and awesome-gpui
catalog observations as such, never as shipped code provenance.
