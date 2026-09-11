> Historical reference. Superseded by [AGENTS.md](../../AGENTS.md).
> The governance and skill evaluation procedures below are retired.

# Alpine engineering skill system

The [skill manifest](https://github.com/dbuddha/alpine-gpui/blob/main/skills/manifest.tsv)
is the installable inventory. Requirement [#564](https://github.com/dbuddha/alpine-gpui/issues/564)
owns acceptance, [#565](https://github.com/dbuddha/alpine-gpui/issues/565) owns
implementation, and [#566](https://github.com/dbuddha/alpine-gpui/issues/566) owns
behavioral evidence. Skills are instructions, not new model weights, guaranteed
expertise, background agents or authorization to weaken repository policy.

## Select one primary skill

| Work | Primary skill | Supporting boundary |
| --- | --- | --- |
| Studio delivery, Rust integration, editor correctness | `alpine-studio-gpui-engineer` | Native timing, source translation or algorithms only when needed |
| Metal ownership, AppKit/QuartzCore scheduling, GPU/residency diagnosis | `apple-metal-performance-engineer` | Official contracts first; Asahi observations are scoped source evidence |
| Zed Editor/GPUI source analysis and narrow adaptation | `zed-gpui-architecture-expert` | Comparator pin, source/license boundary and lineage |
| Data structures, incremental work, allocation/locality | `algorithmic-performance-engineer` | Measured workload and independent correctness model |
| Issues, dependencies, milestones and delivery state | `github-project-operator` | Native GitHub authorities |
| Versioned documentation, Wiki and release projection | `github-documentation-architect` | One canonical owner per fact |
| Research design, primary sources, calibration and claims | `github-deep-researcher` | Reproduction and qualification ceilings |
| CI selection, baseline correspondence, scheduling and feedback cost | `alpine-ci-job-checker` | Existing assurance, source/base identity, and calibrated runtime evidence |
| Bounded goal admission, PR challenge and acceptance review | `alpine-goal-adversarial-reviewer` | Frozen objective, independent raw-evidence review and elapsed-time discipline |

Do not load all skills for every task. Domain skills advise the implementation;
GitHub skills retain their publication and planning roles. Automatic invocation
is enabled for the domain skills. Agents should state the selected skill and
specific decision it contributes, rather than claim that installation proves use.

The [CI checker extension](alpine-ci-job-checker.md) follows the approved CI
intervention. Its inventory and evaluations are separate from the original
three-to-seven-skill treatment; adding an eighth entry does not prove effectiveness
or reopen completed implementation. The manifest, not this historical count,
determines the current installable set.

## Installation and publication

Use the existing [installation commands](github-agent-skills.md#install).
Installer, checker and tests all consume the manifest. Whole-destination preflight
preserves foreign paths and removes only links owned by this checkout. Updating a
repository skill and installing a link are separate from publishing reviewed main.

Report local candidate, installed links, pushed branch, PR, merged source,
published Wiki and audited live Wiki separately. A new Wiki template is not live
publication. After exact-main merge, generate and audit the Wiki with the existing
Wiki tooling. `docs/SUMMARY.md` remains navigation, never live project status.

## Evidence and evolution

The [evaluation protocol](../quality/engineering-skill-evaluation.md) separates
structural validation from behavioral trials. Scenario definitions exist to make
evaluation executable and reproducible; they are not results. No skill gets E3
or an effectiveness claim merely for containing the expected words.

User corrections and failed decisions become narrow feedback on #565/#566 or
their accepted successor. Retain the failure and its context, compare baseline
and candidate in isolated trials, and reject critical regression. Accepted changes
append to the versioned evolution ledger, with issue, PR, source/evaluation/merge
identities and observed result. Previous decisions are superseded, not rewritten.

The Metal reference incorporates Rosenzweig's Asahi retrospective and controlled
partial-render investigation alongside Apple guidance. It adopts discriminating
experiments and resource-pressure testing, not private driver manipulation, Linux
scope or unsupported all-generation hardware claims. See the repository-owned
[Metal source reference](https://github.com/dbuddha/alpine-gpui/blob/main/skills/apple-metal-performance-engineer/references/asahi-metal-boundary.md).

Skill work has no milestone completion credit. Keep Studio product acceptance and
renderer superiority independent. Each use should lead to a concrete code change,
accepted/rejected experiment or removed blocker, not another general plan.

The [bounded goal review extension](alpine-goal-adversarial-reviewer.md) adds a
review role, not demonstrated effectiveness or a new authorization boundary.
