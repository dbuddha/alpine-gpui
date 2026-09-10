#!/bin/sh
# Deterministic receipt controls; fixtures do not claim native execution.
set -eu
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
