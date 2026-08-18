# Research package schema

```text
docs/research/<topic>/
  index.md
  source-map.md
  findings.md
  experiments.md
  alpine-decisions.md
  references.bib
assurance/research/<topic>/<run-id>/
  manifest.toml
  environment.json
  workload.json
  samples/
  checksums.txt
```

The frontmatter and index answer what, why, conclusion, evidence level, owner, review date, and deep evidence location. Separate files keep archaeology, findings, experiments, and decisions reviewable.

```yaml
---
title: ""
slug: ""
status: "proposed|active|accepted|superseded"
research_kind: ""
question: ""
decision_owner: ""
last_reviewed: "YYYY-MM-DD"
review_due: "YYYY-MM-DD or event"
evidence_level: "E0|E1|E2|E3|E4"
upstream_revisions: []
related_issues: []
related_requirements: []
supersedes: []
---
```

Use the reusable [index template](../assets/research-index-template.md).

Commit compact text, methods, manifests, derived tables, and decisions. Do not commit third-party trees, secrets, proprietary data, or huge outputs. Put large immutable evidence in checksummed release assets or an approved artifact store. A bibliography does not replace a source map; a summary does not replace an experiment.
