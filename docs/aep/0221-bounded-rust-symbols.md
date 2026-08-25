# AEP 0221: Bounded Rust document and workspace symbols

- Status: Accepted
- Task: [#221](https://github.com/dbuddha/alpine-gpui/issues/221)
- Requirement: [#34](https://github.com/dbuddha/alpine-gpui/issues/34)
- Parent task: [#128](https://github.com/dbuddha/alpine-gpui/issues/128)
- Research: [#204](https://github.com/dbuddha/alpine-gpui/issues/204)

## Context

Alpine Studio needs fast keyboard document and workspace navigation without a
general picker framework, unbounded language results, remote URI authority, or
late server responses mutating another editor revision. The existing bounded
local process, JSON-RPC peer, Rust document owner, command palette, scene, and
checked source-navigation path already own the required boundaries.

## Decision

Studio owns one private symbol picker inside the existing Rust session. A
document request uses the active document URI. A workspace request carries one
bounded explicit query. Both retain exact workspace, document, buffer,
selection, process, protocol-version, request, and query-revision identity.
Query changes cancel and locally revoke the prior request before resubmission.

The foreground admits hierarchical `DocumentSymbol` values and resolved
`SymbolInformation` locations only. Document hierarchies flatten in source
order with bounded depth. Workspace results must include an explicit URI and
range. Navigation reuses the existing canonical, non-symlinked, workspace-local
path and checked UTF-16 range boundary before changing the active editor.

The picker is keyboard and IME operable and projects one bounded accessibility
dialog through the existing dirty-only frame path. No public API, dependency,
process, worker, timer, polling loop, network path, plugin, AI, cloud,
telemetry, or startup work is added.

## Locked limits

| Resource | Limit | Failure or pressure behavior |
| --- | ---: | --- |
| One symbol JSON result | 1 MiB | Reject before value admission |
| Admitted symbols | 512 | Retain the bounded prefix and report omission |
| Hierarchy depth | 32 | Reject the malformed result |
| One label | 1 KiB | Reject before retained ownership |
| Query and composition | 256 bytes each | Reject without changing the current query |
| Symbol-owned retained bytes | 512 KiB | Reject before publication |
| Visible rows | 12 | Project only the selected window |
| Active request | 1 | Cancel and revoke before supersession |

Existing process, framing, peer, queue, and message ceilings remain
authoritative and are not widened.

## Identity, failure, and lifecycle

Publication requires exact request, query, workspace, document, process epoch,
and LSP version identity. Query edits clear visible results before issuing new
work. A late, malformed, oversized, cancelled, stale, allocation-failed, or
unresolved result preserves document bytes and saving and surfaces bounded
local status. Focus loss, active-document change, restart, and shutdown release
pending and admitted symbol ownership.

The selected result navigates only after the target path and UTF-16 range are
revalidated against the current local workspace snapshot. Unsupported remote,
outside-workspace, symlinked, missing, stale, or invalid-range targets fail
closed without document mutation.

## Atomic claims and evidence contract

- **AEP-0221-C01:** Only the exact current Rust session, request, and query can
  publish symbols; supersession, identity change, focus loss, restart, and close
  revoke stale work.
- **AEP-0221-C02:** Parsing, hierarchy, labels, queries, retained bytes, result
  count, selection, and visible rows remain within locked ceilings.
- **AEP-0221-C03:** Keyboard and accessibility activation navigate only through
  the checked current local workspace location.

The claims require parser and ranking controls, mock-process document and
workspace requests, pinned rust-analyzer compatibility, query cancellation,
retained-byte accounting, Studio scene, keyboard, accessibility, path and range
validation, shutdown, dirty-only framing, Kani selection bounds, TLA+ positive
models and faulty controls, changed-line coverage, viable mutation rejection,
and hosted `ci-pass`.

## Explicit exclusions

Remote symbols, partial unresolved workspace symbols, resolve-on-selection,
global indexes, fuzzy-search workers, symbol caching across sessions, dynamic
language registration, plugins, AI, cloud, telemetry, network services, public
framework pickers, and comparative performance claims are excluded.

## Reversal conditions

Reopen this decision if pinned rust-analyzer requires unresolved symbol
resolution, dogfood demonstrates that the fixed limits materially reject normal
Alpine workspaces, or measured scene or query cost justifies a separately
approved background index. Any widening requires new identity, memory, and
stale-result evidence.
