# Pinned source map

## Revision identities

| ID | Source | Revision | License and boundary | Role |
| --- | --- | --- | --- | --- |
| ALP-S001 | [Alpine GPUI lineage baseline](https://github.com/dbuddha/alpine-gpui/tree/de8cd6397adc81632fe1103f1834214ae6ec6a1a) | `de8cd6397adc81632fe1103f1834214ae6ec6a1a` | Alpine repository | Original lineage audit baseline |
| ALP-S002 | [Alpine realistic trace baseline](https://github.com/dbuddha/alpine-gpui/tree/1b6d16e6ddc120a7670fc225913dad9908dd482c) | `1b6d16e6ddc120a7670fc225913dad9908dd482c` | Alpine repository | Exact source of the accepted eight-fixture trace ladder |
| ALP-S003 | [Alpine realistic trace evidence](https://github.com/dbuddha/alpine-gpui/tree/c98c22f1d3ea0c2deef5c1d082d4518cb5e91ee9) | `c98c22f1d3ea0c2deef5c1d082d4518cb5e91ee9` | Alpine repository | Exact merge retaining the composed E3 GPUI equivalence checkpoint from PR #344 |
| ALP-S004 | [Alpine Rust navigation implementation](https://github.com/dbuddha/alpine-gpui/tree/7db5e18f6da8e02cd171668d4714c745c55d7eda) | `7db5e18f6da8e02cd171668d4714c745c55d7eda` | Alpine repository | Exact merge implementing bounded hover, definition, and references from PR #345 |
| ALP-S005 | [Alpine workspace-edit preparation](https://github.com/dbuddha/alpine-gpui/tree/e2564055622dce3a7d1f277d52fc53e34c16e916) | `e256405...` | Alpine repository | Exact merge implementing strict rename and formatting admission plus immutable off-thread preparation from PR #348 |
| LAB-S001 | [alpine-zed-lab](https://github.com/dbuddha/alpine-zed-lab/tree/13fade6ac4c344a6bf40295544c49971ddfecb96) | `13fade6ac4c344a6bf40295544c49971ddfecb96` | Isolated GPL laboratory | Exact merged adapter and correctness-evidence generator |
| ZED-S001 | [Zed comparator pin](https://github.com/zed-industries/zed/tree/e17dc4f9d50db73a458b64dcce50ecd4878b98a3) | `v1.15.0`, `e17dc4f...` | GPUI crate is Apache-2.0; Zed application source is GPL-3.0-or-later | Immutable accepted comparator |
| ZED-S002 | [Zed current stable review](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210) | `v1.16.1`, `eb8e1c8...` | Same split boundary | Drift review only; does not change comparator pin |
| WGPU-S001 | [WGPU accepted study](https://github.com/gfx-rs/wgpu/tree/8ee190c6f151c731a4f8cfd9a102d6ee5903460a) | `8ee190c...` | MIT OR Apache-2.0 | E2 architecture and experiment source |
| WGPU-S002 | [WGPU current release](https://github.com/gfx-rs/wgpu/tree/40f4a34ebaf56f9a046231f54125ad046239d3f3) | `v30.0.1`, `40f4a34...` | MIT OR Apache-2.0 | Pending patch-delta review under #302 |
| AWG-S001 | [awesome-gpui accepted survey](https://github.com/zed-industries/awesome-gpui/tree/cf11f85a1420dfc5a7f64bc159aacba8133a2f35) | `cf11f85...` | CC0-1.0 | Workload discovery only |
| AWG-S002 | [awesome-gpui current review](https://github.com/zed-industries/awesome-gpui/tree/657169337a19a5b27f9aa7e53811e6f82b7f213c) | `6571693...` | CC0-1.0 | Catalog drift review only |

## GPUI and Zed source anchors

| ID | Pinned source | Observation |
| --- | --- | --- |
| ZGP-S001 | [`Element`](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/src/element.rs#L51) | GPUI elements explicitly separate layout request, prepaint, and paint |
| ZGP-S002 | [`Scene`](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/src/scene.rs#L41) | GPUI collects ordered, primitive-specific frame data |
| ZGP-S003 | [`App`](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/src/app.rs) and [`EntityMap`](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/src/app/entity_map.rs) | GPUI owns a broad retained entity and application runtime that Alpine does not recreate |
| ZGP-S004 | [Line layout cache](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/src/text_system/line_layout.rs#L393) | GPUI retains current and previous frame text layout state |
| ZGP-S005 | [Metal completion handler](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui_macos/src/metal_renderer.rs#L529) | Reusable GPU ownership follows terminal command completion |
| ZGP-S006 | [GPUI architecture README](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/gpui/README.md) | GPUI describes a hybrid immediate/retained model, entity state, elements, actions, async execution, and test support |
| ZED-S101 | [Zed text model](https://github.com/zed-industries/zed/blob/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/text/src/text.rs#L16) | Zed text includes replica identity and operation-oriented state needed for collaboration |
| ZED-S102 | [Zed editor crate](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/editor) | Full editor behavior and rendering source |
| ZED-S103 | [Zed workspace crate](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/workspace) | Broad workspace, actions, persistence, and pane behavior |
| ZED-S104 | [Zed project crate](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/project) | Local and remote project, language, collaboration, Git, and search integration |
| ZED-S105 | [Zed language crate](https://github.com/zed-industries/zed/tree/eb8e1c8b5502b7007465fbbc465f4a736fa39210/crates/language) | Tree-sitter, language registry, syntax, and LSP-facing structures |

## Alpine source anchors

| ID | Pinned source | Implemented boundary |
| --- | --- | --- |
| ALP-S101 | [`SurfaceEvent`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-platform-macos/src/lib.rs#L373) | Typed keyboard, pointer, focus, IME, clipboard, lifecycle, and timing input |
| ALP-S102 | [`NativeSurface`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-platform-macos/src/lib.rs#L2876) | Safe single-window AppKit and CAMetalDisplayLink lifecycle |
| ALP-S103 | [`AppDelegate`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-runtime/src/lib.rs#L995) and [`Application`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-runtime/src/lib.rs#L1102) | Bounded single-window application runtime without GPUI entities |
| ALP-S104 | [`Scene`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-scene/src/lib.rs#L640) | Immutable structure-of-arrays primitives plus painter-order operations |
| ALP-S105 | [`FrameReport`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-renderer/src/lib.rs#L20) | Handle-free frame evidence boundary |
| ALP-S106 | [`BufferSnapshot`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-text/src/lib.rs#L390) and [`Buffer`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-text/src/lib.rs#L990) | Local revisioned rope buffer and immutable snapshots |
| ALP-S107 | [`GlyphAtlasPublication`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-text-layout/src/lib.rs#L1109) and [`GlyphAtlas`](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-text-layout/src/lib.rs#L1224) | Byte-budgeted indexed A8 atlas with no/full/row-delta publication |
| ALP-S108 | [Warm viewport regression](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/apps/alpine-studio/src/studio_coverage_tests.rs#L256) | 10,000 modeled warm frames avoid rasterization and atlas publication |
| ALP-S109 | [Metal native renderer](https://github.com/dbuddha/alpine-gpui/blob/de8cd6397adc81632fe1103f1834214ae6ec6a1a/crates/alpine-metal/src/native.rs) | Direct Metal lowering, retained resources, async completion, and atlas delta upload |
| ALP-S110 | [Studio application](https://github.com/dbuddha/alpine-gpui/tree/de8cd6397adc81632fe1103f1834214ae6ec6a1a/apps/alpine-studio) | Local editor, workspace, settings, syntax, accessibility, and bounded LSP paths |

## WGPU and awesome-gpui anchors

The detailed WGPU primary-source table is retained in the [WGPU source
map](../wgpu/source-map.md). This package consumes its conclusions but does not
claim that Alpine shipping code derives from WGPU code.

awesome-gpui is a catalog, not a framework implementation. It supports workload
selection across editors, terminals, data grids, media, multi-window tools, and
component libraries. Catalog presence, stars, and self-description cannot prove
architecture, correctness, performance, or memory behavior.

## Known source limitations

- Zed application source may explain behavior, but GPL application code remains isolated from Alpine shipping implementation.
- Current Zed stable is a drift-review source, not the accepted comparator revision.
- WGPU experiments in Alpine are designed but not yet reproduced, so their ceiling is E2.
- Sublime Text internals are proprietary and cannot provide code lineage evidence.
- awesome-gpui is unsuitable for performance or implementation claims.
