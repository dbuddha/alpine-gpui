# GitHub agent operations

> Retrieval mirror synchronized from Alpine `main` revision
> `{{ALPINE_MAIN_REVISION}}`. The repository mdBook is canonical.

Alpine owns bounded skills for evidence-first project operation,
documentation architecture, deep research and [engineering](Engineering-Skills).
They preserve the authority of
approved issues, repository documentation, CI, and releases while making
expected workflows installable and repeatable.

Pull requests use one pre-creation metadata check for the repository template,
Conventional Commit title, issue chain, release label, base, and source head.
Later metadata changes supersede but never erase earlier check suites.

Never assume `--auto` waits for CI. Auto-merge is restricted to the protected
default branch; intermediate stacks merge manually only after terminal-green
applicable exact-head checks. The operator binds source, tested base, protection,
required checks, and mergeability before merging, then requires exact-main CI.
The `--merge-readiness` checker validates a supplied snapshot, not live GitHub
truth or authorization. See the canonical source below for the command,
recheck procedure, and retained #520 incident. Failed stacked work is preserved
as superseded evidence, never relabeled green. Skill effectiveness remains a
separate behavioral evaluation, not a claim derived from policy fixture tests.

Issue kind labels, Assignees, Milestones, parent relationships, and native
blocked-by edges own their corresponding facts. Project #1 custom fields own
Delivery Gate, Evidence Level, Workload, and Acceptance Gate. Repository
documentation owns stable policy, and this Wiki remains a retrieval projection.

Canonical source: [GitHub agent operations](https://github.com/dbuddha/alpine-gpui/blob/{{ALPINE_MAIN_REVISION}}/docs/operations/github-agent-skills.md)
