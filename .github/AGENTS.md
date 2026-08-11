# GitHub and CI Instructions

These rules apply to workflows, templates, and repository automation.

- Pin every Action to a reviewed full-length commit SHA.
- Allow no third-party Action without owner approval and source review.
- Declare the minimum workflow token permissions explicitly.
- Do not use `pull_request_target` for untrusted code execution.
- Do not expose secrets to pull request jobs.
- Give every job a timeout and stable name.
- Keep `ci-pass` as the single required aggregate check.
- Required checks must never skip because a path filter omitted all work.
- Test the committed lockfile with `--locked`; dependency freshness is a
  separate reviewed change.
- Security, correctness, validation, and license jobs fail closed. Any temporary
  exception needs an owner, reason, and expiry.
- Keep expensive Metal work behind a deterministic relevance decision while
  preserving a required aggregate result.
- Upload artifacts only when they aid diagnosis, with minimum retention and no
  secrets or proprietary data beyond repository scope.
- Local scripts should own policy logic so contributors can reproduce CI.
