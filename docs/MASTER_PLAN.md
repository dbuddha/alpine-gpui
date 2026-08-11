# Alpine GPUI Master Plan

This plan turns the accepted product contract into ordered, evidence-gated
vertical slices. Dates do not define completion. Each phase exits only when its
behavior, failure handling, memory use, and performance are proven.

## Guiding strategy

1. Own policy and hot paths while permitting audited bindings and
   standards-heavy libraries behind replaceable facades.
2. Prove direct Metal offscreen before native window presentation.
3. Prove demand-driven presentation before building the application runtime.
4. Build runtime, layout, and event foundations before full text integration.
5. Build headless behavior primitives before styled components.
6. Dogfood every diagnostic surface through public APIs.
7. Stabilize shared contracts before direct Vulkan and D3D12 backends.

## Phase 0: Governed foundation

Deliverables:

- repository architecture, roadmap, ADR, provenance, and dependency policy;
- hierarchical agent instructions and internal engineering guide;
- protected pull request workflow and immutable GitHub Actions;
- per-PR change fragments and curated release changelog policy;
- dependency-free core, scene, and renderer contracts;
- three-platform compile and test baseline.

Exit gate:

- every durable decision has an owner and repository artifact;
- `main` requires strict `ci-pass` and linear pull request integration;
- local and hosted baseline gates agree;
- no third-party product dependency or unrecorded upstream source exists.

## Phase 1: Direct Metal offscreen kernel

Add boundaries only when used by the vertical slice:

- `alpine-metal` for Metal ownership and encoding;
- `alpine-macos` for native API facades;
- `alpine-test` for deterministic GPU fixtures;
- `alpine-lab` for conformance scenes.

Implementation sequence:

1. Review and approve the `objc2` binding batch.
2. Enumerate devices and record runtime capability manifests.
3. Own device, queue, command buffer, buffer, texture, and pipeline lifetimes.
4. Implement explicit transient frame arenas and persistent resource registries.
5. Render clear, solid quad, clip, rounded rectangle, image, and compositing
   fixtures into offscreen textures.
6. Read pixels back through deterministic staging resources.
7. Enable Metal validation and inject allocation and unsupported-capability
   failures.
8. Report allocations, uploads, pipeline creation, draw calls, command buffers,
   submissions, and readback bytes.

Exit gate:

- deterministic exact and perceptual fixtures pass on qualified Apple Silicon;
- Metal validation reports zero findings;
- every native failure becomes a structured Alpine error;
- repeated warm frames have bounded resource growth;
- no renderer API exposes application or view state.

## Phase 2: macOS presentation and scheduler

Implementation sequence:

1. Own AppKit application and window lifecycle through a safe Rust boundary.
2. Own CAMetalLayer creation, drawable acquisition, resize, scale, and teardown.
3. Add a display-clock provider without equating clock ticks with invalidation.
4. Coalesce state, input, task, display, and animation invalidations.
5. Handle occlusion, minimize, live resize, multiple windows, surface loss, and
   application activation.
6. Add pointer, keyboard, modifiers, basic clipboard, and display-change events.
7. Expose embedded Metal surfaces and a safe custom-material resource contract.

Exit gate:

- settled and occluded windows submit no framework-triggered frames;
- multiple wake requests schedule at most one pending frame per window;
- 60 Hz and 120 Hz presentation do not leak memory or queue work;
- resize, close, and device teardown cannot race retained resources;
- custom GPU content cannot outlive the target device or presentation surface.

## Phase 3: Runtime, layout, and event kernel

Planned boundaries:

- `alpine-runtime` for entities, transactions, invalidation, and tasks;
- `alpine-layout` for portable layout inputs and provider isolation;
- `alpine-input` for events, focus, commands, and gestures;
- `alpine-ui` for headless element and component behavior.

Implementation sequence:

1. Add generational entity identities and deterministic ownership teardown.
2. Add transactional mutation, notifications, subscriptions, and task scopes.
3. Track view dependencies and invalidate only affected subtrees.
4. Define request-layout, prepaint, paint, and semantic-tree phases.
5. Add typed coordinate spaces, hit testing, event capture and bubble, pointer
   capture, focus traversal, and command routing.
6. Put layout behind an Alpine contract. Use Taffy only after dependency review
   and as a measured oracle or temporary provider.
7. Add virtual collection primitives with no per-item object requirement.
8. Tie animation activity to scheduler invalidation and stop it when settled.

Exit gate:

- unchanged subtrees perform no layout or paint work;
- subscriptions and tasks cannot outlive their owners;
- one million logical rows remain proportional to the visible window;
- event ordering and focus behavior pass deterministic model tests;
- runtime and layout measurements are separately observable.

## Phase 4: CoreText, IME, and accessibility

Planned boundaries:

- `alpine-text` for portable text semantics and provider contracts;
- macOS providers for CoreText, marked text, input methods, and accessibility.

Implementation sequence:

1. Specify font identity, fallback, shaping, glyph, line, selection, and cursor
   semantics independent of CoreText objects.
2. Add CoreText discovery, fallback, shaping, metrics, and rasterization.
3. Add bounded glyph atlas and shaping caches with observable eviction.
4. Cover bidi, CJK, emoji, combining marks, ligatures, tabs, and line breaking.
5. Add selection, cursor movement, clipboard, marked text, and IME composition.
6. Build a semantic accessibility tree alongside visual output.
7. Bridge roles, values, actions, relationships, focus, and hit testing to macOS.

Exit gate:

- text corpus snapshots are deterministic at the semantic and geometry layers;
- native pixel comparisons use documented tolerances;
- IME composition survives focus, selection, undo, and window lifecycle changes;
- every interactive primitive exposes required semantics and actions;
- caches remain within explicit memory budgets.

## Phase 5: Components and dogfood applications

Keep behavior and styling separate:

- `alpine-ui`: state machines, focus, overlays, semantics, and headless elements;
- `alpine-components`: typed themes, tokens, and visual variants.

Component order:

1. Root, text, icon, button, scrolling, focus, overlay, and portal.
2. Checkbox, radio, switch, slider, progress, tabs, and separator.
3. Text input, menu, popover, tooltip, dialog, select, and combobox.
4. Virtual list, table, tree, and resizable panels.
5. Docking and workspace layout.
6. Rich text, Markdown, editor helpers, plotting, and charts as separate optional
   layers rather than core dependencies.

Dogfood applications:

- Alpine Lab owns component stories and conformance fixtures.
- Alpine Inspector exposes frame, scene, allocation, resource, layout, focus,
  text, and accessibility diagnostics through public APIs.
- Alpine Workspace combines editor, terminal-like grid, large table, docking,
  canvas, background tasks, and multiple windows.

Exit gate:

- each component covers keyboard, pointer, focus, accessibility, theme, scale,
  visual, and allocation behavior;
- Alpine Inspector uses no privileged private framework hook;
- Alpine Workspace meets provisional budgets on fixed baseline hardware.

## Phase 6: Direct Vulkan and D3D12

Implementation order:

1. Direct Vulkan renderer and Wayland platform integration.
2. X11 compatibility after Wayland behavior is stable.
3. Direct D3D12 renderer and Win32 platform integration.
4. Optional WGPU oracle for differential behavior, never Metal authority.

Exit gate:

- shared scene, event, semantic, and component conformance suites pass;
- capability differences are explicit rather than silently emulated;
- backend-specific tolerances and resource limits are recorded;
- no portable contract prevents a native fast path.

## Phase 7: Version 1 stabilization

- Declare the supported public API.
- Run API compatibility checks and document deprecation policy.
- Freeze file formats and diagnostic schemas that applications consume.
- Qualify signing, notarization, SBOM, and artifact attestation paths.
- Complete release, rollback, migration, and long-session reliability tests.
- Publish only after owner approval and a release candidate passes fixed-hardware
  gates.

## Provisional performance contract

These budgets are intentionally aggressive and become binding only after the
benchmark harness controls noise on fixed M1-class hardware.

| Measure | Provisional target |
| --- | --- |
| Settled window | Zero framework-triggered frames and GPU submissions over 60 seconds |
| Steady simple animation | Zero heap allocations per frame after warmup |
| 120 Hz simple reference scene | CPU scene build plus encode p99 at or below 2.0 ms |
| 120 Hz simple reference scene | GPU execution p99 at or below 2.0 ms |
| Presented-frame misses | Below 0.1% over 10,000 warm frames |
| Input to visible response | p95 at or below 16 ms on the reference application |
| Empty application framework overhead | At or below 25 MiB resident memory, excluding OS caches |
| Virtual collections | Memory proportional to visible and overscan items, never logical count |
| Resource caches | Explicit configurable budgets with observable eviction |

Each target requires a benchmark definition, measurement owner, hardware
manifest, raw samples, and documented noise threshold before it can block a PR.
