# Development

Use Apple Silicon macOS 15 or newer with Xcode and the Rust toolchain pinned in
`rust-toolchain.toml` for native Studio work. Linux and Windows exercise portable
contracts; they do not run the macOS editor. Use existing pinned tools and official
installers, not ad hoc dependency upgrades.

```sh
cargo run --locked -p alpine-editor
cargo test --locked -p alpine-editor --lib
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Build a canonical unsigned local application from a clean revision:

```sh
scripts/build-alpine-editor-app.sh
scripts/launch-alpine-editor-app.sh path/to/file-or-folder
```

The bundle is `target/release/Alpine Editor.app`; this is private dogfood
infrastructure, not a signed public release. See [limitations](reference/limitations.md),
[settings](reference/studio-settings.md) and [testing](testing.md).

Use a focused branch and preserve dirty or parked work. Repository-local skills
is discovered from `.agents/skills` when working in a checkout containing it.

For an isolated readiness session, first provision the checksummed rust-analyzer
archive using the pinned command in CI, then build from a clean commit:

```sh
python3 scripts/prepare-readiness-probe.py target/readiness-<new-run>
python3 scripts/prepare-readiness-probe.py target/readiness-<new-run> --verify
```

Open the printed application path. Each run has a separate home, disposable Rust
workspace, pinned language server and runtime log. The verifier checks both bundle
and outer identities and immutable file hashes. Preserve previous runs and unsaved
buffers; never replace an executable inside an existing probe. This establishes
artifact identity, not physical or performance acceptance.
