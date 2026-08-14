# Platform strategy

Apple Silicon macOS is the first supported platform. Alpine will use direct
Metal and native macOS integration so platform-specific fast paths, lifecycle,
accessibility, text, and diagnostics remain available.

Portable contracts stop above backend-owned targets, resources, and errors.
Linux with Vulkan and Wayland and Windows with D3D12 and Win32 follow only after
the macOS application path is measurable and the portable contracts have been
validated by a second backend. WGPU may serve as a differential oracle or an
explicit optional compatibility backend, but it does not define Alpine's Metal
semantics or performance.
