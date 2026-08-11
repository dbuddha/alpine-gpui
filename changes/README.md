# Change Fragments

This directory prevents every pull request from editing the same unreleased
section of `CHANGELOG.md`.

Create:

```text
<issue-or-date>-<short-slug>.<kind>.md
```

Kinds are `added`, `changed`, `deprecated`, `removed`, `fixed`, `performance`,
and `security`.

The file contains one complete user-facing sentence with no heading or bullet.
See `docs/engineering/changelog.md` for requirements and release assembly.
