# Worktree inventory and cleanup

Alpine keeps worktrees bounded so branch state, installed agent skills, pull
requests, and retained evidence do not diverge. The repository permits at most
three concurrent GPUI worktrees during active delivery and returns to one
authoritative checkout after related merges.

## Authority and safety

`scripts/check-worktrees.sh` is a read-only classifier. It never prunes,
removes, commits, archives, or changes a branch. Git remains authoritative for
registered paths, heads, branches, dirty state, and ancestry. GitHub is
authoritative for open pull-request heads.

The classifier reports these dispositions:

| Disposition | Meaning |
| --- | --- |
| `retain-authoritative` | Current canonical checkout; never a removal target |
| `removable-merged` | Clean, non-active branch whose head is reachable from current `main` |
| `removable-archived` | Clean, non-active unique head retained by a named worktree archive ref |
| `archive-required` | Unique clean work has no archive ref |
| `refuse-dirty` | Uncommitted or untracked state remains |
| `refuse-active-pr` | An open pull request owns the branch |
| `refuse-ambiguous-pr` | Live pull-request state could not be established |
| `refuse-detached` | No stable branch identity exists |
| `refuse-missing-registration` | Git registration exists but its path is absent |
| `refuse-ambiguous` | Identity or ancestry could not be proven |

An archive ref preserves only the commit it names. It does not preserve dirty
files. A unique archive is valid only when its commit contains the intended
state, the ref is named under `refs/archive/worktrees/<date>/`, and the source
worktree is clean.

## Active-delivery check

Fetch before evaluating ancestry or pull requests:

```sh
git fetch origin --prune
scripts/check-worktrees.sh --check --max-count 3
```

The default command queries open pull-request branches when `gh` and GitHub
access are available. CI uses the deterministic count and registration gate:

```sh
scripts/check-worktrees.sh --check --offline --max-count 3
```

Offline mode intentionally leaves pull-request state unknown. It is sufficient
for inventory and count enforcement, but not for approving a removal.

## Removal procedure

1. Bind the worktree to its issue, pull request, exact source head, exact-head
   CI run, and final merge identity.
2. Preserve unique committed state under a named archive ref. Leave dirty state
   in place until it is deliberately resolved.
3. Migrate installed agent skill links if they target the candidate worktree.
4. Ask for an exact dry-run disposition:

```sh
scripts/check-worktrees.sh --plan-remove /absolute/worktree/path
```

5. Remove only a candidate reported as `removable-merged` or
   `removable-archived`. Do not delete its branch or archive history as part of
   worktree cleanup.
6. Re-run inventory and installed-skill validation from current `main`:

```sh
scripts/check-worktrees.sh --check --offline --require-single
scripts/install-agent-skills.sh --check
scripts/check.sh
```

7. Record before and after counts, removed paths, preserved refs, exact merge
   identities, and gate results on the owning issue.

## Missing registrations

Missing registrations fail the check. Before pruning one, recover its recorded
disposition and prove that every dirty or unique state was preserved. Pruning
is bookkeeping after evidence capture, never a substitute for it.
