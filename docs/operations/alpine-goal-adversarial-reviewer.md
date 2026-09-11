# Bounded Alpine goal review

The repository-owned `alpine-goal-adversarial-reviewer` challenges the connection
between an objective, candidate and evidence. It complements the
[CI checker](alpine-ci-job-checker.md); it does not replace engineering ownership,
independent review, branch protection or accepted assurance.

A window records an immutable elapsed-time deadline and reserves closeout time.
Queueing, research and review count. Where an automatic cutoff is unavailable,
use bounded foreground execution, not an unbounded goal. Stop with pending checks
explicit rather than relaxing acceptance. After two attempted CI windows, require
a product investment checkpoint before further CI work.

## Publication contract

Use one scoped diff and a frozen measurement packet. Independent review receives
raw evidence without the implementer's desired conclusion. PRs identify consumer,
hypothesis, falsifier, acceptance artifacts, risks and the next unfavorable-result
action. Exact-head and integrated-main obligations remain binding.

[Evaluation scenarios](https://github.com/dbuddha/alpine-gpui/blob/main/assurance/agent-skills/v1/goal-review-scenarios.tsv)
and the [evaluation protocol](../quality/engineering-skill-evaluation.md) separate
structural checks, installation, invocation, discovery and measured effectiveness.
No new skill gets product milestone or performance credit from installation.

## CI interpretation

A no-Rust-path proof can omit only `--in-diff` work. It does not omit full native
mutation. Changed Rust paths retain every existing shard, even when later
mutant discovery is empty. `classify-mutation-diff.sh` requires `GITHUB_OUTPUT`
to be defined; set it to an empty string for local stdout output.

Mutation-tool caching binds version, host/compiler, script and trust scope, then
checks bytes and executable version. GitHub ref isolation and distinct keys keep
PR entries separate from main. Self-recorded checksums detect corruption, not a
malicious writer controlling both receipt and binary. Cache tooling, never test
success. Cold-cache installation and failed validation remain observable.

Cargo can resolve external subcommands from `CARGO_HOME/bin` before PATH. The
helper therefore requires any such executable to have the verified digest,
checks the selected PATH bytes and validates Cargo's invocation. A differing
foreign tool fails admission and is preserved, not overwritten. Cache validation
and emptiness-proof steps are unconditional; missing or nonboolean aggregate
requirements fail rather than authorizing a skip.

The dispatcher also rejects `mutants` aliases from the environment or Cargo's
ancestor/home TOML configuration, including escaped keys. Configuration includes
remain unsupported and fail closed until their resolution is reviewed. Admitted
configuration bytes enter the tool identity. The guard uses Python 3.11's
standard-library TOML parser, or the existing `tomli` installation on older
Python; it never installs a parser silently. Aggregate enforcement itself is
unconditional, not merely its helper functions.

The upstream influences are [Zed's pinned dependency-aware workflow](https://github.com/zed-industries/zed/blob/a57ba9b17c433ea1ebfdec8f649f4fa5a402d03b/.github/workflows/run_tests.yml)
and [GitHub's cache access contract](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching).
Alpine retains stricter required-zero handling and its native/formal assurance.
Neither source establishes a universal fifteen-minute limit for Alpine.
