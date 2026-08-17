# GitHub Wiki and Releases

## Wiki role

A Wiki supports high-discovery onboarding, troubleshooting, known issues, and curated links. It is unsafe as an unreviewed second source tree.

For Alpine, author templates in the main repository, link every page to a canonical source, render an exact main revision, validate the manifest and internal links, publish only from clean exact `origin/main`, and reject unknown pages or unique claims.

Recommended routes are Home, Getting Started, Daily Driver Status, Architecture, Developer Guide, Direct Metal, Editor, Performance, Research, Troubleshooting, Known Issues, Releases and Upgrades, Deprecated, and Contributing. Add a route only when its canonical source exists.

AWS ParallelCluster demonstrates useful audience and task grouping plus troubleshooting, known issues, and deprecated-page visibility: https://github.com/aws/aws-parallelcluster/wiki

## Release role

A Release packages one deployable iteration around a tag. Include semantic version and exact commit; platforms and minimum OS; artifact names, sizes, and SHA-256 digests; signing and notarization; changes by impact; breaking changes and migrations; compatibility; known issues; rollback; qualification and provenance links; and security advisories where relevant.

Generated release notes can enumerate pull requests but do not explain user impact or migration. GitHub Releases are tag-based deployable iterations with notes and assets: https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases

## Release gate

Verify tag identity, reproducible build, checksums, signatures, notarization, SBOM or dependency record when required, licenses, install and upgrade smoke tests, known issues, release labels, and download integrity. Publish only after immediate release approval.
