# Worktree hygiene

Treat every worktree as execution state. Inventory before creating or removing
one, and never infer disposability from a merged-looking branch name.

## Read-only preflight

From the authoritative checkout, fetch the remote and classify the complete
registered set:

```sh
git fetch origin --prune
scripts/check-worktrees.sh --check
```

The report distinguishes presence, dirty state, ancestry to current `main`,
open pull-request ownership, archive coverage, and disposition. Live pull
request lookup is authoritative only when GitHub access succeeds. `--offline`
reports unknown pull-request state and must refuse removal planning rather than
guess.

Use the normal count gate while work is active and the closure gate after all
related pull requests merge:

```sh
scripts/check-worktrees.sh --check --offline --max-count 3
scripts/check-worktrees.sh --check --offline --require-single
```

## Removal preflight

Ask the classifier about one exact registered path before any removal:

```sh
scripts/check-worktrees.sh --plan-remove /absolute/worktree/path
```

Only `removable-merged` and `removable-archived` are admissible. Refuse an
authoritative, missing, dirty, detached, active-PR, unknown-PR, ambiguous, or
unarchived unique candidate. The command never removes a worktree.

An archive ref preserves only the commit it names. It does not preserve an
unstaged or untracked patch. Before archiving unique work, create and inspect a
commit containing the intended state, bind a named
`refs/archive/worktrees/<date>/<identity>` ref to it, and prove the worktree is
clean. Never turn unrelated user changes into an archive commit without owner
approval.

## Installed skills and closure

If installed skills target a worktree being retired, migrate them to clean
current `main` first:

```sh
scripts/install-agent-skills.sh --remove-links
scripts/install-agent-skills.sh --install
scripts/install-agent-skills.sh --check
```

Remove only the exact path accepted by the dry run, preserve branch and archive
history, then rerun inventory, installed-link validation, and the repository
gate. Record the removed path, source head, PR and merge identities, archive ref
when applicable, before and after counts, and exact validation evidence on the
owning issue.
