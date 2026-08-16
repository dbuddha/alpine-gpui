# Task 0163 finite document-tab model

`DocumentTabs.tla` models bounded tab count, retained path bytes, navigation
history, dirty-close refusal, duplicate-open activation, and monotonic local
document identity. It is an independent finite abstraction, not a refinement
of the Rust implementation or filesystem.

`OpenNew` maps to atomic insertion after file admission. `OpenDuplicate` maps
to activation of an existing canonical path. `Edit`, `Save`, and `Close` map
to active editor outcomes. History saturates at its configured bound to model
oldest-entry eviction without retaining paths or document payloads.

Pull-request bounds use three tabs, path-byte units, and history entries.
Nightly bounds use five. The two faulty configurations independently prove
that dirty close cannot remove a document and duplicate open cannot increase
the retained tab count. The model excludes filesystem admission, UTF-8,
rendering, native event delivery, allocation failure, and elapsed time.
