#!/bin/sh
# Native mutation receipts only. This does not execute or schedule test work.
set -eu
[ "$#" -ge 4 ] && [ "$#" -le 5 ] || { echo 'usage: prepare|finish domain id output-root [execution-outcome]' >&2; exit 2; }
mode=$1 domain=$2 id=$3 root=$4 outcome=${5-}
case "$domain" in platform|studio) ;; *) exit 2 ;; esac
case "$id" in [1-9]|1[0-6]) ;; *) exit 2 ;; esac
case "$mode" in prepare|finish) ;; *) exit 2 ;; esac
script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
receipt="$root/native-mutation-receipts-$domain-$id"
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        digest=$(sha256sum "$1") || return 1
    else
        digest=$(shasum -a 256 "$1") || return 1
    fi
    digest=${digest%% *}
    printf '%s\n' "$digest" | LC_ALL=C grep -Eq '^[0-9a-f]{64}$' || return 1
    printf '%s\n' "$digest"
}
scopes() {
    if [ "$domain" = studio ]; then
        printf '%s\n' 'native-studio-mutants|apps/alpine-studio/src/lib.rs|full'
    else
        cat <<'SCOPES'
native-mutants|crates/alpine-metal/src/native.rs|changed
native-platform-spi-mutants|crates/alpine-metal/src/platform_spi.rs|changed
native-submission-mutants|crates/alpine-metal/src/submission.rs|changed
native-platform-contract-mutants|crates/alpine-platform-macos/src/lib.rs|changed
native-platform-mutants|crates/alpine-platform-macos/src/native.rs|full
native-accessibility-mutants|crates/alpine-platform-macos/src/native_accessibility.rs|changed
native-studio-accessibility-mutants|crates/alpine-platform-macos/src/native_accessibility.rs|changed
native-studio-accessibility-process-mutants|apps/alpine-studio/src/native_validation/accessibility_process.rs|changed
native-runtime-mutants|crates/alpine-runtime/src/lib.rs|strict
native-ax-client-factory-mutants|tools/alpine-ax-client/src/native_factory.rs|optional
SCOPES
    fi
}
selection_witness() (
    # cargo-mutants 27.1.0 returns before creating output when the selected
    # Rust source is unchanged, even if another Rust source changed. Bind the
    # exact path set independently; an empty shard of a changed source still
    # requires the tool's explicit empty inventory.
    if [ "$domain" = studio ]; then
        printf 'null\n'
        exit 0
    fi
    [ -f "$root/alpine.diff" ] && [ ! -L "$root/alpine.diff" ] || {
        echo 'missing or linked native mutation input diff' >&2; exit 1;
    }
    scratch=$(mktemp -d) || exit 1
    trap 'rm -rf "$scratch"' EXIT HUP INT TERM
    git diff --no-ext-diff --no-textconv "$ALPINE_NATIVE_BASE...$ALPINE_NATIVE_HEAD" > "$scratch/expected.diff" || exit 1
    if ! cmp -s "$scratch/expected.diff" "$root/alpine.diff"; then
        echo 'native mutation input diff does not match bound source and base' >&2
        exit 1
    fi
    # No rename detection: both removed and added paths remain visible, even
    # when a Rust file is renamed to a non-Rust extension or vice versa.
    git diff --no-ext-diff --no-textconv --no-renames --name-only -z \
        "$ALPINE_NATIVE_BASE...$ALPINE_NATIVE_HEAD" -- '*.rs' > "$scratch/rust-paths" || exit 1
    merge_base=$(git merge-base "$ALPINE_NATIVE_BASE" "$ALPINE_NATIVE_HEAD") || exit 1
    diff_hash=$(sha256 "$root/alpine.diff") || exit 1
    paths_hash=$(sha256 "$scratch/rust-paths") || exit 1
    has_rust_changes=false
    if [ -s "$scratch/rust-paths" ]; then has_rust_changes=true; fi
    jq -cn --arg hash "$diff_hash" --arg paths "$paths_hash" --arg merge_base "$merge_base" \
        --rawfile rust_paths "$scratch/rust-paths" \
        --argjson has_rust_changes "$has_rust_changes" '
        {schema:"alpine-native-mutation-diff/v2",path:"alpine.diff",sha256:$hash,
         merge_base:$merge_base,rust_paths_sha256:$paths,path_encoding:"git-nul",
         rust_paths:($rust_paths|split("\u0000")|map(select(length>0))),
         has_rust_changes:$has_rust_changes}'
)
identity() {
    checkout=$(git rev-parse HEAD) || return 1
    tree=$(git rev-parse 'HEAD^{tree}') || return 1
    checker=$(sha256 "$script_dir/check-native-mutation-receipts.sh") || return 1
    rules=$(sha256 "$script_dir/native-mutation-receipt.jq") || return 1
    execution=$(jq -cen --arg head "${ALPINE_NATIVE_HEAD:?}" --arg base "${ALPINE_NATIVE_BASE:?}" \
        --arg tested "${GITHUB_SHA:?}" --arg checkout "$checkout" \
        --arg tree "$tree" --arg workflow "${GITHUB_WORKFLOW_SHA:?}" \
        --arg run "${GITHUB_RUN_ID:?}" --arg attempt "${GITHUB_RUN_ATTEMPT:?}" \
        --arg toolchain "${ALPINE_NATIVE_TOOLCHAIN:?}" --arg mutator "${ALPINE_NATIVE_MUTATOR:?}" \
        --arg flags "${RUSTFLAGS-}" --arg encoded_flags "${CARGO_ENCODED_RUSTFLAGS-}" \
        --arg incremental "${CARGO_INCREMENTAL-}" \
        --arg developer "${DEVELOPER_DIR-}" --arg deployment "${MACOSX_DEPLOYMENT_TARGET-}" \
        --arg domain "$domain" --argjson id "$id" --arg shard "$((id - 1))/16" \
        --arg checker "$checker" --arg rules "$rules" '
        if ([$head,$base,$tested,$tree,$workflow]|all(test("^[0-9a-f]{40}$")))
            and $tested == $checkout and ($run|test("^[1-9][0-9]*$"))
            and ($attempt|test("^[1-9][0-9]*$")) and ($toolchain|length)>0
            and $mutator == "cargo-mutants 27.1.0"
            and (["0","1"] | index($incremental) != null)
        then {schema:"alpine-native-mutation-identity/v2",head:$head,base:$base,
            tested_commit:$tested,tested_tree:$tree,workflow_commit:$workflow,
            run_id:$run,attempt:$attempt,toolchain:$toolchain,mutator:$mutator,
            rustflags:$flags,encoded_rustflags:$encoded_flags,developer:$developer,
            cargo_incremental:$incremental,
            deployment:$deployment,domain:$domain,id:$id,shard:$shard,
            checker_sha256:$checker,rules_sha256:$rules}
        else error("invalid native mutation execution identity") end') || return 1
    witness=$(selection_witness) || return 1
    jq -cn --argjson execution "$execution" --argjson witness "$witness" \
        '$execution + {selection_diff:$witness}'
}
current=$(identity)
expected=$(scopes)
if [ "$mode" = prepare ]; then
    # A fresh identity cannot bless cached or partially mutated output.
    while IFS='|' read -r scope source kind; do
        [ ! -e "$root/$scope-$id.out" ] && [ ! -L "$root/$scope-$id.out" ] || {
            echo "stale mutation output: $scope" >&2; exit 1;
        }
    done <<EOF
$expected
EOF
    mkdir -p "$root"
    mkdir "$receipt"
    if [ "$domain" = platform ]; then
        cp "$root/alpine.diff" "$receipt/selection.diff"
        retained_hash=$(sha256 "$receipt/selection.diff")
        printf '%s\n' "$current" | jq -e --arg hash "$retained_hash" '.selection_diff.sha256 == $hash' >/dev/null
    fi
    printf '%s\n' "$current" > "$receipt/identity.json"
    printf '%s\n' "$expected" > "$receipt/scopes.txt"
    date -u '+%Y-%m-%dT%H:%M:%SZ' > "$receipt/prepared-at.txt"
    exit 0
fi
[ -f "$receipt/identity.json" ] || { echo 'missing mutation preparation identity' >&2; exit 1; }
if ! jq -e --argjson current "$current" '. == $current' "$receipt/identity.json" >/dev/null; then
    echo 'native mutation execution identity changed' >&2
    exit 1
fi
[ ! -e "$receipt/result.json" ] || { echo 'refusing to overwrite a terminal receipt' >&2; exit 1; }
[ "$(cat "$receipt/scopes.txt")" = "$expected" ] || { echo 'scope identity changed' >&2; exit 1; }
case "$outcome" in success|failure|cancelled|skipped) ;; *) echo 'missing execution-step outcome' >&2; exit 1 ;; esac
no_rust_diff=false
if [ "$domain" = platform ]; then
    retained_hash=$(sha256 "$receipt/selection.diff")
    if ! jq -e --arg hash "$retained_hash" '.selection_diff.sha256 == $hash' "$receipt/identity.json" >/dev/null; then
        echo 'retained native mutation diff changed' >&2
        exit 1
    fi
    if jq -e '.selection_diff.has_rust_changes == false' "$receipt/identity.json" >/dev/null; then
        no_rust_diff=true
    fi
fi
while IFS='|' read -r scope source kind; do
    directory="$root/$scope-$id.out/mutants.out"
    inventory="$directory/mutants.json"
    terminal="$directory/outcomes.json"
    destination="$receipt/$scope.json"
    reason=
    if [ "$outcome" != success ]; then
        reason="execution step ended as $outcome; raw partial evidence is not accepted"
    elif [ "$kind" = optional ] && [ ! -f "$source" ] \
        && [ ! -e "$root/$scope-$id.out" ] && [ ! -L "$root/$scope-$id.out" ] \
        && jq -e --arg source "$source" '(.selection_diff.rust_paths|type)=="array"
            and (.selection_diff.rust_paths|index($source))==null' "$receipt/identity.json" >/dev/null; then
        jq -n --arg scope "$scope" --arg source "$source" '{scope:$scope,source:$source,
            status:"not-applicable",reason:"conditional source absent and unchanged",
            selected:0,executed:0,baseline:false}' > "$destination"
        continue
    elif [ "$kind" != full ] \
        && [ ! -e "$root/$scope-$id.out" ] && [ ! -L "$root/$scope-$id.out" ] \
        && jq -e --arg source "$source" '(.selection_diff.rust_paths|type)=="array"
            and (.selection_diff.rust_paths|index($source))==null' "$receipt/identity.json" >/dev/null; then
        jq -n --arg scope "$scope" --arg source "$source" --arg hash "$retained_hash" \
            --argjson no_rust "$no_rust_diff" '
            {scope:$scope,source:$source,status:"not-required",
             reason:(if $no_rust then "verified-no-rust-diff" else "verified-unchanged-rust-source" end),
             selected:0,executed:0,baseline:false,inventory_present:false,
             terminal_present:false,selection_diff_sha256:$hash}' > "$destination"
        continue
    elif [ ! -f "$inventory" ]; then
        reason='missing selected inventory'
    else
        inventory_hash=$(sha256 "$inventory")
        if [ -e "$terminal" ]; then
            terminal_hash=$(sha256 "$terminal")
            set -- --slurpfile terminal "$terminal" --arg terminal_hash "$terminal_hash" --argjson terminal_present true
        else
            set -- --argjson terminal '[]' --arg terminal_hash '' --argjson terminal_present false
        fi
        if jq -se --arg scope "$scope" --arg source "$source" --arg kind "$kind" \
            --arg inventory_hash "$inventory_hash" "$@" \
            -f "$script_dir/native-mutation-receipt.jq" "$inventory" > "$destination" 2> "$receipt/$scope.error.log"; then
            continue
        fi
        reason='invalid, incomplete or unsuccessful mutation evidence; see scope error and raw artifacts'
    fi
    jq -n --arg scope "$scope" --arg reason "$reason" '{scope:$scope,status:"failed",reason:$reason,selected:null,executed:null,baseline:null}' > "$destination"
done < "$receipt/scopes.txt"
# Collect through checked commands, not a pipeline whose producer can fail silently.
: > "$receipt/collected.json"
while IFS='|' read -r scope source kind; do
    cat "$receipt/$scope.json" >> "$receipt/collected.json"
done < "$receipt/scopes.txt"
jq -s --arg outcome "$outcome" --arg expected "$expected" \
    --slurpfile identity "$receipt/identity.json" '
    ($expected|split("\n")|map(split("|")[0])|sort) as $names
    | if (map(.scope)|sort)!=$names or (map(.scope)|unique|length)!=length
        or any(.[];.status as $status | ["complete","not-required","not-applicable","failed"]|index($status)|not)
      then error("incomplete or duplicate scope receipts") else . end
    | {schema:"alpine-native-mutation-receipt/v1",identity:$identity[0],
       execution_outcome:$outcome,scopes:.,status:
       (if $outcome=="success" and all(.status!="failed") then "passed" else "failed" end)}
    ' "$receipt/collected.json" > "$receipt/result.json"
date -u '+%Y-%m-%dT%H:%M:%SZ' > "$receipt/finished-at.txt"
jq -e '.status == "passed"' "$receipt/result.json" >/dev/null
