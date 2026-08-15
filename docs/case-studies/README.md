# Case studies

Each case study is a dated conclusion about one immutable upstream revision.
Its GitHub Research issue owns investigation history and re-evaluation. Stable
finding IDs may motivate AEP claims and Requirements, but a finding is never
proof that Alpine implements or satisfies a behavior.

The current editor research set is the pinned
[Zed application](zed-editor.md), [Zed GPUI and macOS renderer](zed-gpui.md),
and [Sublime Text local-speed model](sublime-editor.md). Zed source findings
identify exact immutable paths and line anchors. Sublime findings distinguish
official public facts, Alpine inference, and unknown proprietary internals.

An upstream-radar workflow compares reviewed revisions with current upstream
heads and opens one deduplicated Research issue when re-evaluation is needed.
Existing snapshots are not silently rewritten. Source adaptation remains
exceptional and requires separate owner approval and conditional provenance.
