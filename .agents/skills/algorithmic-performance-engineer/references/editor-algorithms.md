# Editor algorithm decision table

| Work | Candidates and decision variable | Required discriminator |
| --- | --- | --- |
| Local text and snapshots | Existing approved rope versus flat storage; edit locality, size and snapshot lifetime | Differential String oracle, split/merge boundaries, large paste, undo retention and copy-on-write amplification |
| Line/offset mapping | Tree summaries, prefix indexes, incremental line maps | Byte/grapheme/UTF-16 conversions, long lines, line-ending edits and stale revisions |
| Syntax | Incremental parser or lexical state propagation | Multiline comments/strings/fences and invalidation beyond the viewport |
| Search and quick open | Streaming scan, bounded index, top-k heap, fuzzy matching | Result equivalence, cancellation, ignore rules, result caps and worst-case candidates |
| Selections/diagnostics | Sorted vectors, interval structures, revision-tagged transforms | Overlap, edit-boundary affinity, batch updates and stale response rejection |
| Glyph/layout cache | Keyed lookup plus explicit eviction metadata | Warm work counts, hash/key correctness, pressure, retained bytes and churn |
| Work queues | Bounded channel, coalescing/latest-result slots | Required-event preservation, saturation, cancellation, close/drain and revision races |
| GPU instance data | Existing ordered SoA/batches versus alternative layouts | Upload/copy bytes, alignment, cache locality, painter order and actual draw/encode cost |

Do not implement a custom rope or general reactive framework before a measured
requirement defeats the accepted simpler implementation. Geometric growth reduces
reallocation frequency but can retain excess capacity; include shrinking/pressure
policy and peak old-plus-new allocation in the bound.

Primary retrieval anchors: [Amdahl's original paper](https://doi.org/10.1145/1465482.1465560),
[Roofline paper](https://doi.org/10.1145/1498765.1498785),
[Unicode segmentation](https://www.unicode.org/reports/tr29/), and
[Rust collection complexity](https://doc.rust-lang.org/std/collections/index.html).
These guide models, not Alpine performance claims. Pin the version of any Unicode
or library contract used in an implementation. Read the relevant paper before
extending its assumptions; benchmark changes on the accepted workload rather than
claiming an asymptotic result guarantees a user-visible improvement.
