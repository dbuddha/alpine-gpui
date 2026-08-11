# Source Influence Map

- Review date: 2026-08-10
- Policy: architecture and behavior research by default; source incorporation
  only through explicit approval and the provenance ledger.

## Influence modes

| Mode | Meaning | Required record |
| --- | --- | --- |
| Architecture specimen | Study boundaries and ownership without copying source | Research note and immutable commit |
| Behavior specimen | Derive observable scenarios and failure cases | Behavior specification and independent test |
| Workload specimen | Use an application or component category as a stress case | Alpine Lab or Workspace fixture |
| Differential oracle | Compare Alpine results with another implementation | Versioned adapter and tolerance policy |
| Source incorporation | Copy or adapt source | Owner approval, license review, and symbol-level provenance |

## Reviewed repositories

| Source | Reviewed commit | What Alpine learns | How it enters Alpine | What Alpine rejects |
| --- | --- | --- | --- | --- |
| [Zed GPUI](https://github.com/zed-industries/zed/tree/1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d/crates/gpui) | `1271f8b0e8f3278eed5dd3fc12ad4bd30dce2c5d` | Entity/context model, element phases, scene boundaries, native Metal, headless tests, virtualization | Clean API and behavior specifications plus independent tests | Zed workspace coupling and source compatibility obligation |
| [GPUI-CE](https://github.com/gpui-ce/gpui-ce/tree/b172d695cd2f6d0ad70caedfc3d78d95c6b5d02b) | `b172d695cd2f6d0ad70caedfc3d78d95c6b5d02b` | Platform crate separation, headless capture, backend capability questions | Architecture boundary review and regression cases | Upstream synchronization, Git dependency ownership, manifest drift |
| [gpui-component](https://github.com/longbridge/gpui-component/tree/55968d167bd6959551c3417c3622899c33ecda20) | `55968d167bd6959551c3417c3622899c33ecda20` | Component taxonomy, typed theme tokens, input, virtual tables/lists, docking, stories, accessibility cases | Headless component specifications and Alpine Lab workload fixtures | Monolithic component dependency, global initialization, Git-sourced GPUI graph |
| [mdeand/gpui-wgpu](https://github.com/mdeand/gpui-wgpu/tree/a2158ca36a0f46b32c3a66423b6498a3f0ed6ae1) | `a2158ca36a0f46b32c3a66423b6498a3f0ed6ae1` | Embedded surfaces and unconditional-redraw failure mode | Scheduler regression test and optional future oracle | WGPU and winit as flagship ownership boundaries |
| [Nestri gpui-wgpu](https://github.com/nestrilabs/gpui-wgpu/tree/49d46d31a14f2f11efe17a3157a0f0ef4c825bd4) | `49d46d31a14f2f11efe17a3157a0f0ef4c825bd4` | Point-in-time descendant differences | Cross-check only | Treating it as independent architectural evidence |
| [WGPUI](https://github.com/Far-Beyond-Pulsar/WGPUI/tree/fd087f643f749e11f29ef53307b2fdcd83c1202a) | `fd087f643f749e11f29ef53307b2fdcd83c1202a` | Active unified WGPU backend and embedded-surface examples | Future differential fixtures | Unified backend authority over direct Metal |
| [Kael](https://github.com/Augani/kael/tree/4d67872d678454b06d7f7a3bebf7ef5f82a78c6c) | `4d67872d678454b06d7f7a3bebf7ef5f82a78c6c` | Damage, budgets, render-graph questions, native-service and test taxonomy | Feature and conformance checklist | Wholesale breadth, immature surfaces, codebase adoption |
| [awesome-gpui](https://github.com/zed-industries/awesome-gpui/tree/cf11f85a1420dfc5a7f64bc159aacba8133a2f35) | `cf11f85a1420dfc5a7f64bc159aacba8133a2f35` | Editors, terminals, databases, media, whiteboards, tables, and ecosystem API demand | Alpine Workspace workload inventory | Treating popularity as correctness or performance proof |
| [gpui-unofficial](https://github.com/iamnbutler/gpui-unofficial/tree/76d3fc3b5d4b10e218fa24431ecad0669e320dcc) | `76d3fc3b5d4b10e218fa24431ecad0669e320dcc` | Release transformation and package provenance questions | Packaging research | Automated transformed source as product foundation |

## Component requirements derived from gpui-component

Alpine will implement behavior in tiers rather than copying its broad crate:

1. Root, text, icon, button, focus, scrolling, overlay, and portal.
2. Selection controls, slider, progress, tabs, and separators.
3. Text input, menus, popovers, tooltips, dialogs, selects, and comboboxes.
4. Virtual lists, virtual row and column tables, trees, and resizing.
5. Docking and workspace layout.
6. Rich text, editor helpers, Markdown, charts, and plotting as optional layers.

Every tier receives independent state-machine, keyboard, pointer, focus,
accessibility, scale, theme, visual, allocation, and scheduling tests.

## Current incorporation status

No upstream implementation source is incorporated into Alpine GPUI. The
initial code and current governance artifacts are independently written.
