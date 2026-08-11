# Changelog Policy

## Goals

- Tell framework consumers what behavior changed.
- Avoid merge conflicts in a shared unreleased section.
- Separate user-facing release notes from implementation history.
- Preserve the record in Git rather than only in GitHub Releases.
- Make agent-authored changes mechanically reviewable.

## Artifacts

| Artifact | Purpose |
| --- | --- |
| `changes/*.md` | One concise user-facing change per PR |
| `CHANGELOG.md` | Curated reverse-chronological release record |
| Pull request | Context, evidence, risk, and validation |
| Commit history | Concise logical integration history |
| ADR and research notes | Decisions and source evidence |

## Fragment names

Use:

```text
<issue-or-date>-<short-slug>.<kind>.md
```

Allowed kinds:

- `added`
- `changed`
- `deprecated`
- `removed`
- `fixed`
- `performance`
- `security`

Examples:

```text
20260810-metal-readback.added.md
142-frame-coalescing.fixed.md
207-upload-ring.performance.md
```

The file contains one complete sentence written for an application developer.
Describe observable effect, not internal implementation steps. Do not add a
heading, bullet marker, issue number, author, or AI attribution.

## When a fragment is required

Add a fragment for:

- public API or behavior changes;
- bug fixes visible to applications;
- performance or memory changes;
- security changes;
- platform capability or compatibility changes;
- deprecations and removals.

A fragment is normally unnecessary for documentation-only changes, test-only
changes, internal refactors without observable effects, or CI maintenance. The
PR must explicitly say `Not applicable` and why.

## Release assembly

In a dedicated release PR:

1. Verify every merged fragment is present and classified correctly.
2. Group fragments under Keep a Changelog headings.
3. Edit for a consistent user-facing voice without changing meaning.
4. Add the version and ISO date to `CHANGELOG.md`.
5. Add comparison links when tags exist.
6. Delete only the fragments included in that release.
7. Update package versions and the lockfile together.
8. Run the complete release candidate gate.
9. Create the signed tag, release notes, artifacts, SBOM, and attestations only
   after owner approval.

The root changelog is authoritative. GitHub release notes mirror it.
