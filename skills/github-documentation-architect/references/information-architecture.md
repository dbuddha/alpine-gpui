# Enterprise documentation information architecture

| Type | Reader need | Canonical Alpine location |
| --- | --- | --- |
| Tutorial | Learn through a complete path | mdBook use cases or getting started |
| How-to | Complete a bounded task | mdBook guide |
| Reference | Look up exact contracts | rustdoc, schemas, mdBook reference |
| Explanation | Understand tradeoffs | architecture or accepted AEP |
| Troubleshooting | Recover from failure | mdBook, projected to Wiki |
| Decision | Know direction and consequences | GitHub decision issue plus AEP when architectural |
| Research | Audit sources, methods, and experiments | repository package plus research issue |
| Release record | Understand one shipped version | GitHub Release and signed tag |

## Metadata

Use frontmatter when supported: title, slug, audience, status, owners, product versions, last reviewed, review due, canonical source, related issues, and supersedes. Do not add metadata without a consumer or validation rule.

## Navigation

Organize first by reader task, then subsystem. Keep top-level navigation stable. Provide audience starts. Put known issues and troubleshooting within two clicks of getting started. Link historical decisions from current architecture without placing stale decisions in the happy path. Maintain a deprecation index.

## Enterprise gates

Validate Markdown structure, links, examples, generated API references, navigation membership, duplicate titles and slugs, canonical-source declarations, version support, ownership, review dates, secrets, accessible diagrams, and deterministic generation. Compile API examples. Test security or operations commands in isolation. Link raw evidence for performance claims.

## Lifecycle

`draft -> reviewed -> current -> superseded -> archived`

Drafts are not canonical. Current pages have an owner and review trigger. Superseded pages remain addressable and link replacements. Archived content leaves active navigation but remains discoverable.
