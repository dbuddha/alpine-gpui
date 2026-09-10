#!/bin/sh
# Deterministic receipt controls; fixtures do not claim native execution.
set -eu
case "${1-}" in ''|--pinned-cli) ;; *) echo 'usage: test-native-mutation-receipts.sh [--pinned-cli]' >&2; exit 2 ;; esac
[ "$#" -le 1 ] || exit 2
pinned_cli=${1-}
script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
checker="$script_dir/check-native-mutation-receipts.sh"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
GITHUB_SHA=$(git rev-parse HEAD)
ALPINE_NATIVE_HEAD=$GITHUB_SHA ALPINE_NATIVE_BASE=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
GITHUB_RUN_ID=1 GITHUB_RUN_ATTEMPT=1
ALPINE_NATIVE_TOOLCHAIN='fixture rustc' ALPINE_NATIVE_MUTATOR='cargo-mutants 27.1.0'
export GITHUB_SHA ALPINE_NATIVE_HEAD ALPINE_NATIVE_BASE GITHUB_WORKFLOW_SHA GITHUB_RUN_ID GITHUB_RUN_ATTEMPT
export ALPINE_NATIVE_TOOLCHAIN ALPINE_NATIVE_MUTATOR
case_number=0
prepare() {
    case_number=$((case_number + 1))
    root="$fixture/case-$case_number"
    if [ "$1" = platform ]; then
        mkdir -p "$root"
        git diff --no-ext-diff --no-textconv "$ALPINE_NATIVE_BASE...$ALPINE_NATIVE_HEAD" > "$root/alpine.diff"
    fi
    "$checker" prepare "$1" 1 "$root"
}
fail_finish() {
    if "$checker" finish "$1" 1 "$root" "$2" > "$root/rejected.log" 2>&1; then
        echo "receipt unexpectedly accepted: $3" >&2
        exit 1
    fi
}
# Generate the pinned tool schema independently of the production verifier.
populate() {
    directory="$root/$1-1.out/mutants.out"
    mkdir -p "$directory"
    jq -n --arg source "$2" '
        def mutant($n): {file:$source,name:($source+":"+($n|tostring)+":1: replace fixture"),
                        replacement:"false",diff:"fixture difference"};
        [mutant(1),mutant(2)]' > "$directory/mutants.json"
    jq -n --slurpfile inventory "$directory/mutants.json" --arg packages "$3" '
        ($packages|split(",")|map("--package="+.)) as $packages
        | def phase($name;$status): {phase:$name,duration:0.01,process_status:$status,
              argv:(["cargo",($name|ascii_downcase)] + $packages + ["--locked"])};
          {cargo_mutants_version:"27.1.0",total_mutants:2,caught:1,unviable:1,missed:0,timeout:0,success:0,
           outcomes:[
             {scenario:"Baseline",summary:"Success",phase_results:[phase("Build";"Success"),phase("Test";"Success")]},
             {scenario:{Mutant:($inventory[0][0]|del(.diff))},summary:"CaughtMutant",
                phase_results:[phase("Build";"Success"),phase("Test";{Failure:101})]},
             {scenario:{Mutant:($inventory[0][1]|del(.diff))},summary:"Unviable",
                phase_results:[phase("Build";{Failure:101})]}]}' > "$directory/outcomes.json"
}
studio() { populate native-studio-mutants apps/alpine-studio/src/lib.rs alpine-studio; }
platform() {
    for scope in native-mutants native-platform-spi-mutants native-submission-mutants \
        native-platform-contract-mutants native-accessibility-mutants \
        native-studio-accessibility-mutants native-studio-accessibility-process-mutants \
        native-runtime-mutants native-ax-client-factory-mutants
    do
        mkdir -p "$root/$scope-1.out/mutants.out"
        printf '[]\n' > "$root/$scope-1.out/mutants.out/mutants.json"
    done
    populate native-platform-mutants crates/alpine-platform-macos/src/native.rs alpine-platform-macos,alpine-studio
}
alter() {
    jq "$2" "$directory/$1" > "$root/altered.json"
    mv "$root/altered.json" "$directory/$1"
}
prepare studio
studio
"$checker" finish studio 1 "$root" success
jq -e '.status=="passed" and (.scopes|length)==1 and .scopes[0].caught==1
    and .scopes[0].unviable==1 and .scopes[0].selected==2 and .scopes[0].executed==2
    and .scopes[0].baseline==true' "$root/native-mutation-receipts-studio-1/result.json" >/dev/null
fail_finish studio success repeated-terminal
prepare platform
platform
"$checker" finish platform 1 "$root" success
jq -e '.status=="passed" and (.scopes|length)==10
    and ([.scopes[]|select(.status=="not-required")]|length)==9
    and all(.scopes[]|select(.status=="not-required");.baseline==false and .executed==0)' \
    "$root/native-mutation-receipts-platform-1/result.json" >/dev/null

for fault in missing-outcome missing-mutant duplicate-mutant duplicate-inventory wrong-identity \
    wrong-source baseline-failed baseline-missing unknown-result missed timeout counter-mismatch \
    command-mismatch required-package-missing version-mismatch build-not-terminal zero-selection \
    truncated-inventory multi-document
do
    prepare studio
    studio
    case "$fault" in
        missing-outcome) rm "$directory/outcomes.json" ;;
        missing-mutant) alter outcomes.json 'del(.outcomes[2])' ;;
        duplicate-mutant) alter outcomes.json '.outcomes[2]=.outcomes[1]' ;;
        duplicate-inventory) alter mutants.json '.[1]=.[0]' ;;
        wrong-identity) alter outcomes.json '.outcomes[1].scenario.Mutant.replacement="true"' ;;
        wrong-source) alter mutants.json '.[0].file="crates/wrong/src/lib.rs"' ;;
        baseline-failed) alter outcomes.json '.outcomes[0].phase_results[1].process_status={Failure:101}' ;;
        baseline-missing) alter outcomes.json 'del(.outcomes[0])' ;;
        unknown-result) alter outcomes.json '.outcomes[1].summary="Unknown"' ;;
        missed) alter outcomes.json '.outcomes[1].summary="MissedMutant"' ;;
        timeout) alter outcomes.json '.outcomes[1].summary="Timeout"' ;;
        counter-mismatch) alter outcomes.json '.caught=2' ;;
        command-mismatch) alter outcomes.json '.outcomes[1].phase_results[1].argv+=["--all-features"]' ;;
        required-package-missing) alter outcomes.json '.outcomes[].phase_results[].argv|=map(select(startswith("--package=")|not))' ;;
        version-mismatch) alter outcomes.json '.cargo_mutants_version="0.0.0"' ;;
        build-not-terminal) alter outcomes.json '.outcomes[2].phase_results[0].process_status="Success"' ;;
        zero-selection) printf '[]\n' > "$directory/mutants.json"; rm "$directory/outcomes.json" ;;
        truncated-inventory) : > "$directory/mutants.json" ;;
        multi-document) cat "$directory/mutants.json" > "$root/duplicate.json"; cat "$root/duplicate.json" >> "$directory/mutants.json" ;;
    esac
    fail_finish studio success "$fault"
done
for outcome in failure cancelled skipped; do
    prepare platform
    platform
    fail_finish platform "$outcome" failed-execution
    jq -e '.status=="failed" and all(.scopes[];.status=="failed")' \
        "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
done
for fault in absent-one absent-all empty-terminal truncated-terminal malformed-empty scopes-tampered strict-unviable; do
    prepare platform
    platform
    case "$fault" in
        absent-one) rm "$root/native-runtime-mutants-1.out/mutants.out/mutants.json" ;;
        absent-all) for path in "$root"/*.out/mutants.out/mutants.json; do rm "$path"; done ;;
        empty-terminal) printf '[]\n' > "$root/native-runtime-mutants-1.out/mutants.out/outcomes.json" ;;
        truncated-terminal) : > "$root/native-runtime-mutants-1.out/mutants.out/outcomes.json" ;;
        malformed-empty) printf '[\n' > "$root/native-runtime-mutants-1.out/mutants.out/mutants.json" ;;
        scopes-tampered) : > "$root/native-mutation-receipts-platform-1/scopes.txt" ;;
        strict-unviable) populate native-runtime-mutants crates/alpine-runtime/src/lib.rs alpine-runtime,alpine-studio ;;
    esac
    fail_finish platform success "$fault"
done
prepare studio
studio
# POSIX shells can retain assignment prefixes on shell functions. Keep each
# intentionally invalid environment in a child shell, then prove recovery.
( GITHUB_RUN_ATTEMPT=2 fail_finish studio success wrong-run-identity
  grep -Fxq 'native mutation execution identity changed' "$root/rejected.log" )
( ALPINE_NATIVE_HEAD=invalid fail_finish studio success invalid-source
  grep -Fq 'invalid native mutation execution identity' "$root/rejected.log" )
[ "$GITHUB_RUN_ATTEMPT" = 1 ]
[ "$ALPINE_NATIVE_HEAD" = "$GITHUB_SHA" ]
"$checker" finish studio 1 "$root" success
root="$fixture/prepopulated"
mkdir -p "$root/native-studio-mutants-1.out"
if "$checker" prepare studio 1 "$root" > "$fixture/stale.log" 2>&1; then
    echo 'stale output accepted' >&2; exit 1
fi
grep -Fq 'stale mutation output: native-studio-mutants' "$fixture/stale.log"
mkdir "$fixture/bad-hash"
for command in sha256sum shasum; do
    printf '#!/bin/sh\necho "injected hash-command failure" >&2\nexit 1\n' > "$fixture/bad-hash/$command"
    chmod +x "$fixture/bad-hash/$command"
done
if PATH="$fixture/bad-hash:$PATH" "$checker" prepare studio 1 "$fixture/hash-failure" > "$fixture/hash.log" 2>&1; then
    echo 'hash failure accepted' >&2; exit 1
fi
grep -Fq 'injected hash-command failure' "$fixture/hash.log"
prepare studio
studio
alter outcomes.json '.outcomes[1].phase_results[].argv+=["--package=alpine-studio@0.0.0","--package=alpine-studio"]'
"$checker" finish studio 1 "$root" success
printf 'native mutation receipt controls passed (%s isolated cases)\n' "$case_number"

# Exercise real Git history independently of the verifier's path detection.
# These receipts remain fixtures; the opt-in CLI control proves only the
# pinned tool's no-work behavior, never successful native mutation execution.
(
    repository="$fixture/diff-repository"
    mkdir -p "$repository/src" "$repository/tools/alpine-ax-client/src" \
        "$repository/crates/alpine-metal/src"
    git init --quiet "$repository"
    cd "$repository"
    git config user.name 'Alpine receipt fixture'
    git config user.email 'alpine-receipt-fixture@example.invalid'
    git config commit.gpgsign false
    git config core.hooksPath /dev/null
    printf '[package]\nname = "receipt-fixture"\nversion = "0.0.0"\nedition = "2021"\n' > Cargo.toml
    printf 'pub mod other;\npub fn value() -> bool { true }\n' > src/lib.rs
    printf 'pub fn other() -> bool { true }\n' > src/other.rs
    printf 'pub fn native() -> bool { true }\n' > crates/alpine-metal/src/native.rs
    printf '// fixture\n' > tools/alpine-ax-client/src/native_factory.rs
    printf 'baseline\n' > README.md
    git add Cargo.toml src crates/alpine-metal/src/native.rs \
        tools/alpine-ax-client/src/native_factory.rs README.md
    git commit --quiet -m 'Create isolated receipt fixture'
    ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
    printf 'documentation only\n' >> README.md
    git add README.md
    git commit --quiet -m 'Change documentation only'
    GITHUB_SHA=$(git rev-parse HEAD)
    ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
    export ALPINE_NATIVE_BASE ALPINE_NATIVE_HEAD GITHUB_SHA GITHUB_WORKFLOW_SHA

    full_platform() {
        populate native-platform-mutants crates/alpine-platform-macos/src/native.rs alpine-platform-macos,alpine-studio
    }
    require_no_rust_receipt() {
        jq -e '.status=="passed" and .identity.selection_diff.has_rust_changes==false
            and ([.scopes[]|select(.reason=="verified-no-rust-diff")]|length)==9
            and all(.scopes[]|select(.reason=="verified-no-rust-diff");
                .selected==0 and .executed==0 and .baseline==false
                and .inventory_present==false and .terminal_present==false)' \
            "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    }
    prepare platform
    full_platform
    "$checker" finish platform 1 "$root" success
    require_no_rust_receipt
    if [ "$pinned_cli" = --pinned-cli ]; then
        [ "$(cargo mutants --version)" = 'cargo-mutants 27.1.0' ] || {
            echo 'pinned no-work control requires cargo-mutants 27.1.0' >&2; exit 1;
        }
        cargo mutants --no-config --file src/lib.rs --in-diff "$root/alpine.diff" \
            --output "$root/cli-execution" -- --locked > "$root/cli-execution.log" 2>&1
        [ ! -e "$root/cli-execution" ]
        cargo mutants --no-config --file src/lib.rs --in-diff "$root/alpine.diff" \
            --output "$root/cli-list" --list --json > "$root/cli-list.json" 2> "$root/cli-list.log"
        [ ! -s "$root/cli-list.json" ]
        [ ! -e "$root/cli-list" ]
        printf 'pinned cargo-mutants 27.1.0 no-Rust-diff execution/list controls passed\n'
    fi
    for fault in partial-directory linked-output missing-diff tampered-diff retained-diff missing-full; do
        prepare platform
        full_platform
        case "$fault" in
            partial-directory) mkdir "$root/native-runtime-mutants-1.out" ;;
            linked-output) ln -s "$root/absent" "$root/native-runtime-mutants-1.out" ;;
            missing-diff) rm "$root/alpine.diff" ;;
            tampered-diff) : > "$root/alpine.diff" ;;
            retained-diff) : > "$root/native-mutation-receipts-platform-1/selection.diff" ;;
            missing-full) rm -r "$root/native-platform-mutants-1.out" ;;
        esac
        fail_finish platform success "$fault"
    done
    prepare studio
    fail_finish studio success missing-full-studio
    for outcome in failure cancelled skipped; do
        prepare platform
        full_platform
        fail_finish platform "$outcome" failed-no-rust-execution
        jq -e '.status=="failed" and all(.scopes[];.status=="failed")' \
            "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    done
    prepare platform
    full_platform
    ( ALPINE_NATIVE_BASE=0000000000000000000000000000000000000000
      export ALPINE_NATIVE_BASE
      fail_finish platform success nonexistent-base )
    "$checker" finish platform 1 "$root" success
    require_no_rust_receipt
    root="$fixture/missing-input-diff"
    mkdir "$root"
    if "$checker" prepare platform 1 "$root" > "$root/rejected.log" 2>&1; then
        echo 'missing input diff accepted during preparation' >&2; exit 1
    fi
    grep -Fq 'missing or linked native mutation input diff' "$root/rejected.log"

    # A mixed Rust diff does not make every unchanged source a selected scope.
    ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
    printf 'pub fn other() -> bool { false }\n' > src/other.rs
    git add src/other.rs
    git commit --quiet -m 'Change only an unrelated Rust source'
    GITHUB_SHA=$(git rev-parse HEAD)
    ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
    prepare platform
    full_platform
    "$checker" finish platform 1 "$root" success
    jq -e '.status=="passed"
        and .identity.selection_diff.schema=="alpine-native-mutation-diff/v2"
        and .identity.selection_diff.has_rust_changes==true
        and .identity.selection_diff.rust_paths==["src/other.rs"]
        and ([.scopes[]|select(.reason=="verified-unchanged-rust-source")]|length)==9
        and all(.scopes[]|select(.reason=="verified-unchanged-rust-source");
            .selected==0 and .executed==0 and .baseline==false
            and .inventory_present==false and .terminal_present==false)' \
        "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    for fault in partial-directory linked-output path-list-tampered; do
        prepare platform
        full_platform
        case "$fault" in
            partial-directory) mkdir "$root/native-runtime-mutants-1.out" ;;
            linked-output) ln -s "$root/absent" "$root/native-runtime-mutants-1.out" ;;
            path-list-tampered)
                jq '.selection_diff.rust_paths=[]' "$root/native-mutation-receipts-platform-1/identity.json" > "$root/tampered.json"
                mv "$root/tampered.json" "$root/native-mutation-receipts-platform-1/identity.json" ;;
        esac
        fail_finish platform success "$fault-mixed-rust-diff"
        if [ "$fault" = path-list-tampered ]; then
            grep -Fxq 'native mutation execution identity changed' "$root/rejected.log"
        fi
    done
    for outcome in failure cancelled skipped; do
        prepare platform
        full_platform
        fail_finish platform "$outcome" failed-mixed-rust-execution
        jq -e 'all(.scopes[];.status=="failed")' "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    done
    if [ "$pinned_cli" = --pinned-cli ]; then
        # Discovery proves this target was eligible before the diff filter.
        cargo mutants --no-config --file src/lib.rs --list --json \
            --output "$root/cli-discovery" > "$root/cli-discovery.json" 2> "$root/cli-discovery.log"
        jq -e 'length>0 and all(.[];.file=="src/lib.rs")' "$root/cli-discovery.json" >/dev/null
        cargo mutants --no-config --file src/lib.rs --in-diff "$root/alpine.diff" \
            --output "$root/cli-unchanged-source" -- --locked > "$root/cli-unchanged-source.log" 2>&1
        [ ! -e "$root/cli-unchanged-source" ]
        cargo mutants --no-config --file src/lib.rs --in-diff "$root/alpine.diff" \
            --output "$root/cli-unchanged-list" --list --json > "$root/cli-unchanged-list.json" 2> "$root/cli-unchanged-list.log"
        [ ! -s "$root/cli-unchanged-list.json" ]
        [ ! -e "$root/cli-unchanged-list" ]
        selected_base=$(git rev-parse HEAD)
        printf 'pub mod other;\npub fn value() -> bool { false }\n' > src/lib.rs
        git add src/lib.rs
        git commit --quiet -m 'Change the CLI-selected Rust source'
        git diff "$selected_base...HEAD" > "$root/selected-source.diff"
        cargo mutants --no-config --file src/lib.rs --in-diff "$root/selected-source.diff" \
            --shard 15/16 --sharding round-robin --output "$root/cli-empty-shard" \
            -- --locked > "$root/cli-empty-shard.log" 2>&1
        jq -e 'type=="array" and length==0' "$root/cli-empty-shard/mutants.out/mutants.json" >/dev/null
        [ ! -e "$root/cli-empty-shard/mutants.out/outcomes.json" ]
        printf 'pinned CLI discoverable unchanged-source and changed-source empty-shard controls passed\n'
    fi

    # Mutate actual declared scope paths, not the unrelated fixture crate path.
    for change in modified renamed-out renamed-in deleted; do
        ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
        case "$change" in
            modified) printf 'pub fn native() -> bool { false }\n' > crates/alpine-metal/src/native.rs ;;
            renamed-out) mv crates/alpine-metal/src/native.rs crates/alpine-metal/src/native.txt ;;
            renamed-in) mv crates/alpine-metal/src/native.txt crates/alpine-metal/src/native.rs ;;
            deleted) rm crates/alpine-metal/src/native.rs ;;
        esac
        git add -A crates/alpine-metal/src
        git commit --quiet -m "Create $change Rust-path control"
        GITHUB_SHA=$(git rev-parse HEAD)
        ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
        prepare platform
        full_platform
        fail_finish platform success "$change-rust-missing-inventory"
        jq -e '.status=="failed" and .identity.selection_diff.has_rust_changes==true
            and (.identity.selection_diff.rust_paths|index("crates/alpine-metal/src/native.rs"))!=null
            and all(.scopes[]|select(.scope=="native-mutants");.status=="failed")' \
            "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
        if [ "$change" = modified ]; then
            prepare platform
            full_platform
            mkdir -p "$root/native-mutants-1.out/mutants.out"
            printf '[]\n' > "$root/native-mutants-1.out/mutants.out/mutants.json"
            "$checker" finish platform 1 "$root" success
            jq -e '.status=="passed" and all(.scopes[]|select(.scope=="native-mutants");
                .reason=="successful empty selection" and .baseline==false)' \
                "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
        fi
    done
    for change in renamed-out renamed-in deleted; do
        ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
        case "$change" in
            renamed-out) mv tools/alpine-ax-client/src/native_factory.rs tools/alpine-ax-client/src/native_factory.txt ;;
            renamed-in) mv tools/alpine-ax-client/src/native_factory.txt tools/alpine-ax-client/src/native_factory.rs ;;
            deleted) rm tools/alpine-ax-client/src/native_factory.rs ;;
        esac
        git add -A tools/alpine-ax-client/src
        git commit --quiet -m "Create optional-source $change control"
        GITHUB_SHA=$(git rev-parse HEAD)
        ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
        prepare platform
        full_platform
        fail_finish platform success "$change-optional-source"
        jq -e 'any(.scopes[];.scope=="native-ax-client-factory-mutants" and .status=="failed")' \
            "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    done
    ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
    mkdir -p crates/alpine-platform-macos/src
    printf '// shared-source fixture\n' > crates/alpine-platform-macos/src/native_accessibility.rs
    git add crates/alpine-platform-macos/src/native_accessibility.rs
    git commit --quiet -m 'Add a source shared by two mutation scopes'
    GITHUB_SHA=$(git rev-parse HEAD)
    ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
    prepare platform
    full_platform
    fail_finish platform success shared-source-missing-inventories
    jq -e '([.scopes[]|select((.scope=="native-accessibility-mutants"
        or .scope=="native-studio-accessibility-mutants") and .status=="failed")]|length)==2' \
        "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    ALPINE_NATIVE_BASE=$(git rev-parse HEAD)
    printf 'unchanged optional absence\n' >> README.md
    git add README.md
    git commit --quiet -m 'Keep an absent optional source unchanged'
    GITHUB_SHA=$(git rev-parse HEAD)
    ALPINE_NATIVE_HEAD=$GITHUB_SHA GITHUB_WORKFLOW_SHA=$GITHUB_SHA
    prepare platform
    full_platform
    "$checker" finish platform 1 "$root" success
    jq -e '.status=="passed" and any(.scopes[];.scope=="native-ax-client-factory-mutants"
        and .status=="not-applicable" and .reason=="conditional source absent and unchanged")' \
        "$root/native-mutation-receipts-platform-1/result.json" >/dev/null
    prepare platform
    full_platform
    ln -s "$root/absent" "$root/native-ax-client-factory-mutants-1.out"
    fail_finish platform success dangling-optional-output
    root="$fixture/stale-linked-optional-output"
    mkdir "$root"
    git diff "$ALPINE_NATIVE_BASE...$ALPINE_NATIVE_HEAD" > "$root/alpine.diff"
    ln -s "$root/absent" "$root/native-ax-client-factory-mutants-1.out"
    if "$checker" prepare platform 1 "$root" > "$root/rejected.log" 2>&1; then
        echo 'linked optional output accepted during preparation' >&2; exit 1
    fi
    grep -Fq 'stale mutation output: native-ax-client-factory-mutants' "$root/rejected.log"
    printf 'Git-bound per-source diff controls passed (%s prepared cases plus stale-output controls)\n' "$case_number"
)
