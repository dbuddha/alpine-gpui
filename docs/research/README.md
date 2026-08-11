# Research Records

Research in this directory preserves the evidence behind Alpine GPUI decisions.
It is not a backlog of links and it does not authorize source copying.

Each ecosystem review records an immutable commit, the question asked, useful
concepts, observed failure modes, and the exact way the result may influence
Alpine. [`source-map.md`](source-map.md) is the current influence map, while
[`provenance-ledger.md`](provenance-ledger.md) is the authority for any source
that is copied or adapted.

When a research finding becomes an architectural choice, create or update an
ADR. When it becomes required behavior, put the requirement in a specification
and add an independent test. When it becomes benchmark policy, define the
measurement environment and store summarized results under `docs/performance/`.

Upstream code is never merged mechanically. Source incorporation requires prior
owner approval, a compatible license review, a narrow diff, and symbol-level
provenance.
