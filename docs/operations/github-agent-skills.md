# GitHub agent operations

Alpine owns three installable skills that turn repository policy into repeatable agent workflows. They advise and validate work; they do not supersede `AGENTS.md`, approved issues, CI, or owner-held approvals.

| Skill | Use it for | Canonical output |
| --- | --- | --- |
| `github-project-operator` | Issues, dependencies, Projects, milestones, blockers, critical path, burn-up, and status | Reconciled GitHub state linked to evidence |
| `github-documentation-architect` | mdBook, Wiki, architecture and API docs, troubleshooting, known issues, Releases | One authority per fact plus tested projections |
| `github-deep-researcher` | Source archaeology, papers, experiments, comparator research, agent evaluation | Source-pinned package and implementing decisions |

## Install

```sh
scripts/install-agent-skills.sh --install
scripts/install-agent-skills.sh --check
```

The installer creates absolute symbolic links under `${CODEX_HOME:-$HOME/.codex}/skills`. It preflights all destinations before creating or removing links and refuses paths owned by another source.

```sh
scripts/install-agent-skills.sh --remove-links
```

## Authority boundaries

Issues own live work and approval. Projects own views and planning metadata. Repository Markdown and mdBook own durable technical truth. Wiki is generated navigation. Releases own shipped-version facts and assets. Skills cannot lower repository gates.

## Research depth

Deep research states a decision question, pins primary sources, separates facts from inference, seeks contradictory evidence, records validity threats, reproduces consequential behavior, and links findings to requirements. Architecture adoption requires E2, performance design claims E3, and dominance claims E4.

Substantial research uses a frontmatter index plus source map, findings, experiments, decisions, bibliography, and checksummed raw evidence. Alpine's current path policy must change through an approved task before a new package layout is introduced.

## Validation

```sh
scripts/check-agent-skills.sh
scripts/test-agent-skills.sh
scripts/check.sh
```

Checks cover metadata, resources, triggers, overwrite refusal, idempotency, owned-link removal, foreign-link preservation, mdBook navigation, and Wiki integrity.
