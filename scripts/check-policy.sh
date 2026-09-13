#!/bin/sh
set -eu

# Path allowlists use byte order, independent of the hosted runner's locale.
export LC_ALL=C

failures=0

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

# cargo-mutants 27.1.0 derives baseline packages from mutated files, not from
# --test-package. Common Cargo arguments must name both the mutated package and
# every configured test package, making both effective package unions identical.
# Keep commands on one line so this guard cannot silently miss a continuation.
workflow_files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) -print)
action_files=$(find .github/actions -type f \( -name '*.yml' -o -name '*.yaml' \) -print 2>/dev/null || true)

if [ -n "$workflow_files" ]; then
    ci_workflow=${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}
    for product_job in preflight quality native; do
        product_job_block=$(awk -v job="$product_job" '
            $0 == "  " job ":" { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != job ":" && capture { exit }
            capture
        ' "$ci_workflow")
        if ! printf '%s\n' "$product_job_block" | grep -Fqx '    runs-on: macos-26' \
            || printf '%s\n' "$product_job_block" | grep -Eq '^[[:space:]]+matrix:'; then
            fail "CI product job $product_job must use Apple Silicon macOS without a platform matrix"
        fi
    done
    action_source_files=$workflow_files
    if [ -n "$action_files" ]; then
        action_source_files="$action_source_files $action_files"
    fi
    action_refs=$(grep -hE '^[[:space:]]*uses:' $action_source_files || true)
    external_action_refs=$(printf '%s\n' "$action_refs" | grep -E 'uses:[[:space:]]+(actions|github)/' || true)
    if [ -n "$external_action_refs" ] && printf '%s\n' "$external_action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
        fail 'every GitHub Action must be pinned to a full commit SHA'
        printf '%s\n' "$external_action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >&2 || true
    fi

    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" \
        | grep -Ev 'uses:[[:space:]]+(actions|github)/|uses:[[:space:]]+\./\.github/actions/upload-required-artifact([[:space:]]|$)' >/dev/null; then
        fail 'only pinned GitHub-owned Actions and the governed required-artifact helper are permitted'
        printf '%s\n' "$action_refs" \
            | grep -Ev 'uses:[[:space:]]+(actions|github)/|uses:[[:space:]]+\./\.github/actions/upload-required-artifact([[:space:]]|$)' >&2 || true
    fi

    continue_on_error_lines=$(grep -hE '^[[:space:]]*continue-on-error:[[:space:]]*true[[:space:]]*$' $workflow_files || true)

    metal_validation_block=$(awk '
        /^  metal-validation:/ { capture = 1 }
        /^  [A-Za-z0-9_-]+:/ && $1 != "metal-validation:" && capture { exit }
        capture
    ' "$ci_workflow")
    preflight_block=$(awk '
        /^  preflight:/ { capture = 1 }
        /^  [A-Za-z0-9_-]+:/ && $1 != "preflight:" && capture { exit }
        capture
    ' "$ci_workflow")
    ci_pass_block=$(awk '
        /^  ci-pass:/ { capture = 1 }
        capture
    ' "$ci_workflow")
    if ! grep -Fqx '    types: [opened, synchronize, reopened]' "$ci_workflow"; then
        fail 'CI pull_request triggers must include opened, synchronize and reopened only'
    fi
    if ! grep -Fqx "  group: ci-\${{ github.workflow }}-\${{ github.ref }}" "$ci_workflow"; then
        fail 'CI events for one ref must share a concurrency group'
    fi
    if ! grep -Fqx "  cancel-in-progress: true" "$ci_workflow"; then
        fail 'CI must cancel superseded runs for the same ref'
    fi
    dispatch_block=$(awk '
        /^  workflow_dispatch:/ { capture = 1 }
        capture && /^[^[:space:]#]/ { exit }
        capture && /^  [^ ]/ && !/^  workflow_dispatch:/ { exit }
        capture
    ' "$ci_workflow")
    dispatch_base_block=$(printf '%s\n' "$dispatch_block" | awk '
        /^      base_sha:/ { capture = 1; next }
        capture && NF && !/^        / { exit }
        capture
    ')
    for required in '        required: true' '        type: string'; do
        if ! printf '%s\n' "$dispatch_base_block" | grep -Fqx "$required"; then
            fail 'CI dispatch must require an explicit string base_sha input'
            break
        fi
    done
    classify_block=$(awk '
        /^  classify:/ { capture = 1 }
        /^  [A-Za-z0-9_-]+:/ && !/^  classify:/ && capture { exit }
        capture
    ' "$ci_workflow")
    if ! printf '%s\n' "$classify_block" | grep -Fqx '          ALPINE_BASE_SHA: ${{ github.event.pull_request.base.sha || github.event.before || inputs.base_sha }}'; then
        fail 'CI classifier must bind the PR, push, or explicit dispatch base'
    fi
    for required in \
        '    name: preflight' \
        '        run: scripts/check-policy.sh' \
        '        run: scripts/check-ci-fast-feedback.sh'
    do
        if ! printf '%s\n' "$preflight_block" | grep -Fqx "$required"; then
            fail 'CI preflight must validate technical repository policy before fan-out'
            break
        fi
    done
    fast_feedback_block=$(printf '%s\n' "$preflight_block" | awk '
        /^      - name: Require cheap source feedback before assurance fan-out$/ { capture = 1; next }
        capture && /^      - / { exit }
        capture
    ')
    if ! printf '%s\n' "$fast_feedback_block" | grep -Fqx '        run: scripts/check-ci-fast-feedback.sh' \
        || ! printf '%s\n' "$fast_feedback_block" | grep -Fqx "        if: needs.classify.outputs.code == 'true'" \
        || [ "$(printf '%s\n' "$fast_feedback_block" | grep -Ec '^[[:space:]]*if:')" -ne 1 ] \
        || printf '%s\n' "$fast_feedback_block" | grep -Eq '^[[:space:]]*continue-on-error:' \
        || ! printf '%s\n' "$preflight_block" | grep -Fqx '    needs: classify' \
        || printf '%s\n' "$preflight_block" | grep -Eq '^    (if|continue-on-error):'; then
        fail 'CI fast feedback must follow code selection and propagate failures'
    fi
    for required_job in quality native metal-validation; do
        required_job_block=$(awk -v job="$required_job" '
            $0 == "  " job ":" { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != job ":" && capture { exit }
            capture
        ' "$ci_workflow")
        required_dependencies='    needs: [classify, preflight]'
        if ! printf '%s\n' "$required_job_block" | grep -Fqx "$required_dependencies"; then
            fail "CI job $required_job must wait for the fast policy preflight"
        fi
        if [ "$required_job" = native ]; then
            native_admission_block=$(printf '%s\n' "$required_job_block" | awk '
                /^      - name: Verify native execution$/ { capture = 1; next }
                capture && /^      - / { exit }
                capture
            ')
            for required in \
                "        if: needs.classify.outputs.metal == 'true'" \
                '          DEVELOPER_DIR: /Applications/Xcode_26.6.app/Contents/Developer' \
                '          MACOSX_DEPLOYMENT_TARGET: "15.0"' \
                '          ALPINE_VALIDATION_DEPLOYMENT_TARGET: "26.0"' \
                '          ALPINE_PRESENTATION_EVIDENCE_MODE: hosted-direct' \
                '          RUSTFLAGS: --cfg alpine_native_validation' \
                '        run: scripts/check-ci-native-admission.sh'
            do
                if ! printf '%s\n' "$native_admission_block" | grep -Fqx "$required"; then
                    fail 'CI native mutation admission must preserve the explicit native baseline contract'
                    break
                fi
            done
            if printf '%s\n' "$native_admission_block" | grep -q 'continue-on-error'; then
                fail 'CI native mutation admission must not ignore baseline failures'
            fi
        fi
    done
    if ! printf '%s\n' "$ci_pass_block" | grep -Fqx '    if: ${{ always() && !cancelled() }}'; then
        fail 'ci-pass must run after ordinary failures but skip a canceled workflow'
    fi
    aggregate_enforcement_block=$(printf '%s\n' "$ci_pass_block" | awk '
        /^      - name: Require selected evidence$/ { capture = 1; next }
        capture && /^      - / { exit }
        capture
    ')
    if ! printf '%s\n' "$aggregate_enforcement_block" | grep -Fqx '        run: |' \
        || printf '%s\n' "$aggregate_enforcement_block" | grep -Eq '^[[:space:]]*(if|continue-on-error):' \
        || printf '%s\n' "$ci_pass_block" | grep -Eq '^    continue-on-error:'; then
        fail 'ci-pass enforcement must run unconditionally and propagate failures'
    fi
    if ! printf '%s\n' "$ci_pass_block" | grep -Fq 'needs: [classify, preflight,' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'PREFLIGHT_RESULT: ${{ needs.preflight.result }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'require_success preflight "$PREFLIGHT_RESULT"'; then
        fail 'ci-pass must require the exact-head policy preflight'
    fi
    if ! printf '%s\n' "$ci_pass_block" | grep -Fq 'test "$2" = success || {'; then
        fail 'ci-pass must reject every required result other than success'
    fi
    if ! printf '%s\n' "$ci_pass_block" | grep -Fqx '              true|false) ;;' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fqx '              *) echo "$1 has an invalid requirement: $2" >&2; exit 1 ;;'; then
        fail 'ci-pass requirements must be boolean and reject any other value'
    fi
    if grep -lE '^[[:space:]]*issues:[[:space:]]*write' $workflow_files >/dev/null 2>&1; then
        fail 'no workflow may file issues automatically; defects are entered by a human'
    fi
    if ! awk '
        function without_comment(line, i, character, quote, escaped) {
            for (i = 1; i <= length(line); i++) {
                character = substr(line, i, 1)
                if (escaped) { escaped = 0; continue }
                if (character == "\\" && quote != "\047") { escaped = 1; continue }
                if (quote != "") {
                    if (character == quote) quote = ""
                    continue
                }
                if (character == "\"" || character == "\047") { quote = character; continue }
                if (character == "#" && (i == 1 || substr(line, i - 1, 1) ~ /[[:space:]]/))
                    return substr(line, 1, i - 1)
            }
            return line
        }
        {
            # Shell bodies are opaque, including multiline quoted strings.
            # Mode declarations belong in the two reviewed YAML env entries.
            match($0, /^ */); indentation = RLENGTH
            if (in_run && $0 ~ /[^[:space:]]/ && indentation <= run_indentation)
                in_run = 0
            if (in_run || $0 ~ /^[[:space:]]+run:/) {
                if (index($0, "CARGO_INCREMENTAL")) invalid = 1
                if (!in_run) { in_run = 1; run_indentation = indentation }
                next
            }
            $0 = without_comment($0); sub(/[[:space:]]+$/, "")
        }
        /^[[:space:]]*#/ { next }
        /^[^[:space:]]/ { section = $0; job = ""; in_environment = 0 }
        section == "jobs:" && /^  [A-Za-z0-9_-]+:$/ {
            job = $1; sub(/:$/, "", job); in_environment = 0
        }
        /^    env:$/ { in_environment = 1; next }
        /^    [^ ]/ { in_environment = 0 }
        /CARGO_INCREMENTAL/ {
            if (section == "env:" && $0 == "  CARGO_INCREMENTAL: \"0\"") global++
            else invalid = 1
        }
        END { exit (invalid || global != 1) }
    ' "${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}"; then
        fail 'CI compilation mode must stay globally disabled and unscoped'
    fi
    if grep -Eq 'ALPINE_PR_|pull_request\.(body|title|labels)|mdbook|validate --github|test-hierarchy|check-wiki|test-wiki' "$ci_workflow"; then
        fail 'ordinary CI must not depend on PR metadata, book, Wiki or live hierarchy'
    fi
    extract_metal_step() {
        printf '%s\n' "$metal_validation_block" | awk -v target="      - name: $1" '
            $0 == target { capture = 1 }
            capture && /^      - name:/ && $0 != target { exit }
            capture
        '
    }
    validate_metal_upload_retry() {
        primary_name=$1
        retry_name=$2
        primary_id=$3
        primary_step=$(extract_metal_step "$primary_name")
        retry_step=$(extract_metal_step "$retry_name")
        primary_contract=$(printf '%s\n' "$primary_step" | sed -n '/^[[:space:]]*with:/,$p')
        retry_contract=$(printf '%s\n' "$retry_step" | sed -n '/^[[:space:]]*with:/,$p')

        if [ -z "$primary_step" ] \
            || [ -z "$retry_step" ] \
            || ! printf '%s\n' "$primary_step" | grep -Fqx "        id: $primary_id" \
            || ! printf '%s\n' "$primary_step" | grep -Fqx '        if: always()' \
            || ! printf '%s\n' "$primary_step" | grep -Fqx '        continue-on-error: true' \
            || ! printf '%s\n' "$primary_step" | grep -Fqx '        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' \
            || ! printf '%s\n' "$retry_step" | grep -Fqx "        if: always() && steps.$primary_id.outcome == 'failure'" \
            || ! printf '%s\n' "$retry_step" | grep -Fqx '        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' \
            || printf '%s\n' "$retry_step" | grep -Eq 'continue-on-error:[[:space:]]*true' \
            || [ -z "$primary_contract" ] \
            || [ "$primary_contract" != "$retry_contract" ]; then
            fail "required Metal artifact upload must retain one identical blocking retry: $primary_name"
        fi
    }
    if [ "$(printf '%s\n' "$continue_on_error_lines" | grep -c . || true)" -ne 3 ] \
        || [ "$(printf '%s\n' "$metal_validation_block" | grep -Ec '^[[:space:]]*continue-on-error:[[:space:]]*true[[:space:]]*$')" -ne 3 ]; then
        fail 'continue-on-error is restricted to the three bounded primary Metal artifact uploads'
    fi
    validate_metal_upload_retry \
        'Upload compiled shader evidence' \
        'Retry compiled shader evidence upload' \
        'upload-metal-shader-primary'
    validate_metal_upload_retry \
        'Upload native lifecycle soak evidence' \
        'Retry native lifecycle soak evidence upload' \
        'upload-native-lifecycle-primary'
    validate_metal_upload_retry \
        'Upload rust-analyzer compatibility evidence' \
        'Retry rust-analyzer compatibility evidence upload' \
        'upload-rust-analyzer-compatibility-primary'
    required_artifact_action=${ALPINE_REQUIRED_ARTIFACT_ACTION:-.github/actions/upload-required-artifact/action.yml}
    if [ ! -f "$required_artifact_action" ]; then
        fail 'the governed required-artifact helper must exist'
    else
        required_primary_step=$(awk '
            /^    - name: Upload required artifact$/ { capture = 1 }
            /^    - name:/ && $0 != "    - name: Upload required artifact" && capture { exit }
            capture
        ' "$required_artifact_action")
        required_retry_step=$(awk '
            /^    - name: Retry required artifact upload$/ { capture = 1 }
            capture
        ' "$required_artifact_action")
        required_primary_contract=$(printf '%s\n' "$required_primary_step" | sed -n '/^[[:space:]]*with:/,$p')
        required_retry_contract=$(printf '%s\n' "$required_retry_step" | sed -n '/^[[:space:]]*with:/,$p')
        required_action_pin='actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a'
        required_action_valid=true
        if [ "$(grep -Fc "uses: $required_action_pin" "$required_artifact_action")" -ne 2 ] \
            || [ "$(grep -Ec '^[[:space:]]+required:[[:space:]]+true$' "$required_artifact_action")" -ne 4 ] \
            || ! grep -Fqx '  using: composite' "$required_artifact_action" \
            || ! printf '%s\n' "$required_primary_step" | grep -Fqx '      id: primary' \
            || ! printf '%s\n' "$required_primary_step" | grep -Fqx '      continue-on-error: true' \
            || ! printf '%s\n' "$required_retry_step" | grep -Fqx "      if: always() && steps.primary.outcome == 'failure'" \
            || printf '%s\n' "$required_retry_step" | grep -Eq 'continue-on-error:[[:space:]]*true' \
            || [ -z "$required_primary_contract" ] \
            || [ "$required_primary_contract" != "$required_retry_contract" ]; then
            required_action_valid=false
        fi
        for required_input in name path retention-days if-no-files-found; do
            required_forward="\${{ inputs.$required_input }}"
            if ! grep -Fqx "  $required_input:" "$required_artifact_action" \
                || [ "$(grep -Fc "$required_forward" "$required_artifact_action")" -ne 2 ]; then
                required_action_valid=false
            fi
        done
        if [ "$required_action_valid" != true ]; then
            fail 'required artifact helper must retain one tolerated primary and one identical blocking failure-only retry'
        fi
    fi


fi

# Prune the build tree instead of traversing every cached file and then filtering.
# Keep the same source-manifest inventory, including untracked source manifests.
manifest_files=$(find . -path './target' -prune -o -name Cargo.toml -print)
if [ -n "$manifest_files" ] && grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >/dev/null; then
    fail 'shipping Cargo manifests may not contain Git dependencies'
    grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >&2 || true
fi

unsafe_override_files=$(grep -lE '^unsafe_code[[:space:]]*=[[:space:]]*"allow"' $manifest_files 2>/dev/null | sort || true)
expected_unsafe_override_files='./crates/alpine-metal/Cargo.toml
./crates/alpine-platform-macos/Cargo.toml
./crates/alpine-text-layout/Cargo.toml
./tools/alpine-ax-client/Cargo.toml'
if [ "$unsafe_override_files" != "$expected_unsafe_override_files" ]; then
    fail 'only audited native Metal, macOS platform, text, and non-shipping AX crates may override unsafe-code denial'
    printf '%s\n' "$unsafe_override_files" >&2
fi

unsafe_source_files=$(find crates apps tools -type f -name '*.rs' -print0 \
    | xargs -0 grep -lE 'unsafe[[:space:]]+(extern|fn|impl|trait)|unsafe[[:space:]]*\{' 2>/dev/null \
    | sort || true)
expected_unsafe_source_files='crates/alpine-metal/src/native.rs
crates/alpine-platform-macos/src/menu.rs
crates/alpine-platform-macos/src/native.rs
crates/alpine-platform-macos/src/native_accessibility.rs
crates/alpine-platform-macos/src/native_text_input.rs
crates/alpine-platform-macos/src/signpost.rs
crates/alpine-text-layout/src/native.rs
tools/alpine-ax-client/src/native.rs'
if [ "$unsafe_source_files" != "$expected_unsafe_source_files" ]; then
    fail 'unsafe Rust constructs must remain isolated in audited native boundary files'
    printf '%s\n' "$unsafe_source_files" >&2
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
