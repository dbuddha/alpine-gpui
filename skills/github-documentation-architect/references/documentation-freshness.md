# Documentation freshness

Freshness is a review contract, not a date stamped without a consumer.

## Review triggers

Review a document when its accepted requirement, architecture boundary, public
API, dependency, supported platform, comparator pin, evidence protocol,
milestone contract, release, or canonical owner changes. Routine task movement
does not require rewriting stable path documentation.

## Current-page contract

A current page identifies its canonical source, audience, supported revision or
version when relevant, owner or owning process, review trigger, and replacement
when superseded. If frontmatter is used, validate every field and consume it in
navigation, review tooling, or publication.

## Staleness controls

- Reject links to missing canonical sources.
- Reject generated pages whose revision differs from their source checkout.
- Distinguish a locally valid mirror template from a fetched live remote. Only
  an exact remote inventory and byte comparison proves live Wiki freshness.
- Reject mutable version claims without a revision or release identity.
- Detect implementation inventories whose stated review revision is obsolete.
- Preserve superseded pages outside the happy path with a replacement link.
- Treat missing Project permission as unknown live state, not empty state.

Do not create busywork review dates for timeless reference pages. Prefer
event-driven triggers and automated identity checks.

Run the live Wiki audit after a canonical source change, Wiki publication,
failed publication, worktree migration, or before reporting documentation state.
Keep it outside offline CI so network availability cannot break source checks.
