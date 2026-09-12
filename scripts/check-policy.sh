#!/bin/sh
set -eu

failures=0
tla_driver=${ALPINE_TLA_DRIVER:-scripts/check-tla.sh}

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

# cargo-mutants 27.1.0 derives baseline packages from mutated files, not from
# --test-package. Common Cargo arguments must name both the mutated package and
# every configured test package, making both effective package unions identical.
# Keep commands on one line so this guard cannot silently miss a continuation.
check_mutation_baseline() {
    if ! awk '
        /cargo mutants / {
            mutated_package = ""
            test_packages = 0
            if ($NF == "\\") {
                print FILENAME ":" FNR ": mutation command must remain on one line" > "/dev/stderr"
                invalid = 1
            }
            for (i = 1; i <= NF; i++) {
                if ($i == "--") break
                if ($i == "--file") {
                    count = split($(i + 1), path, "/")
                    if (count >= 3 && path[1] ~ /^(crates|apps|tools)$/) mutated_package = path[2]
                }
                if ($i ~ /^--test-package=/ || $i ~ /^--test-workspace(=|$)/) {
                    print FILENAME ":" FNR ": mutation package selector must use explicit --test-package entries" > "/dev/stderr"
                    invalid = 1
                }
                if ($i == "--baseline=skip" || ($i == "--baseline" && $(i + 1) == "skip")) {
                    print FILENAME ":" FNR ": mutation baseline must execute" > "/dev/stderr"
                    invalid = 1
                }
                if ($i != "--test-package") continue
                test_packages++
                package = $(i + 1)
                expected = "--cargo-arg=--package=" package
                found = 0
                for (j = 1; j <= NF; j++) {
                    if ($j == "--") break
                    if ($j == expected) found = 1
                }
                if (package !~ /^[a-z][a-z0-9-]*$/ || !found) {
                    print FILENAME ":" FNR ": missing common baseline package " package > "/dev/stderr"
                    invalid = 1
                }
            }
            if (test_packages) {
                expected = "--cargo-arg=--package=" mutated_package
                found = 0
                for (j = 1; j <= NF; j++) {
                    if ($j == "--") break
                    if ($j == expected) found = 1
                }
                if (mutated_package !~ /^[a-z][a-z0-9-]*$/ || !found) {
                    print FILENAME ":" FNR ": missing common mutated package " mutated_package > "/dev/stderr"
                    invalid = 1
                }
            }
        }
        END { exit invalid }
    ' "$1"; then
        fail 'mutation baseline and mutant package scopes must correspond'
    fi
}

workflow_files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) -print)
action_files=$(find .github/actions -type f \( -name '*.yml' -o -name '*.yaml' \) -print 2>/dev/null || true)

if [ -n "$workflow_files" ]; then
    ci_workflow=${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}
    check_mutation_baseline "$ci_workflow"
    assurance_failure_workflow=${ALPINE_ASSURANCE_FAILURE_WORKFLOW:-.github/workflows/assurance-failure.yml}
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

    if ! grep -Fq -- "--exclude 'crates/alpine-platform-macos/src/native_accessibility.rs'" "$ci_workflow"; then
        fail 'Linux changed-code mutation must delegate native accessibility to macOS validation'
    fi
    if ! grep -Fq -- '--file crates/alpine-platform-macos/src/native_accessibility.rs' "$ci_workflow"; then
        fail 'required macOS validation must own changed native accessibility mutation'
    fi
    native_accessibility_pr_scope='RefreshOutcome::post|NotificationIntent::kind|NotificationIntent::record|NotificationIntent::retained_bytes|NotificationIntent::post|NativeAccessibilityAdapter::refresh_view_if_active|NativeAccessibilityAdapter::refresh_view|NativeAccessibilityAdapter::reconcile_elements|NativeAccessibilityAdapter::append_notification_intents|NativeAccessibilityAdapter::push_notification|NativeAccessibilityAdapter::push_layout_notification|NativeAccessibilityAdapter::push_announcement|NativeAccessibilityAdapter::record_posted|NativeAccessibilityAdapter::begin_revoke|NativeAccessibilityAdapter::finish_revoke|NativeAccessibilityAdapter::set_selection|NativeAccessibilityAdapter::activate|NativeAccessibilityElement::with_adapter|NativeAccessibilityElement::with_adapter_mut|NativeAccessibilityElement::accessibility_frame_impl|layout_user_info_valid|announcement_user_info_valid|layout_semantics_changed|reusable_semantics|checked_range'
    if ! grep -Fq -- "--re '$native_accessibility_pr_scope'" "$ci_workflow" \
        || ! grep -Fq -- '-- --locked --test native_accessibility' "$ci_workflow"; then
        fail 'required macOS validation must mutation-test the bounded native accessibility risk slice through its exact journey'
    fi
    if ! grep -Fq -- 'native_validation::NativeAccessibilityEvidence::' "$ci_workflow"; then
        fail 'validation-only native accessibility evidence getters must not consume the pull-request mutation budget'
    fi

    native_mutation_block=$(awk '
        /^  native-mutation:/ { capture = 1 }
        /^  [A-Za-z0-9_-]+:/ && $1 != "native-mutation:" && capture { exit }
        capture
    ' "$ci_workflow")
    mutation_diff_block=$(awk '
        /^  mutation-diff:/ { capture = 1 }
        /^  [A-Za-z0-9_-]+:/ && $1 != "mutation-diff:" && capture { exit }
        capture
    ' "$ci_workflow")
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
        || printf '%s\n' "$fast_feedback_block" | grep -Eq '^[[:space:]]*(if|continue-on-error):' \
        || printf '%s\n' "$preflight_block" | grep -Eq '^    (if|continue-on-error):'; then
        fail 'CI fast feedback must execute unconditionally and propagate failures'
    fi
    if ! printf '%s\n' "$native_mutation_block" | grep -Fqx "    if: needs.classify.outputs.native_mutation == 'true'" \
        || printf '%s\n' "$native_mutation_block" | grep -Eq '^    continue-on-error:'; then
        fail 'CI native mutation must retain success-gated admission'
    fi
    if ! printf '%s\n' "$mutation_diff_block" | grep -Fqx "    if: needs.classify.outputs.mutation_diff == 'true'" \
        || ! printf '%s\n' "$classify_block" | grep -Fqx '      mutation_diff: ${{ steps.mutation-diff.outputs.required }}' \
        || ! printf '%s\n' "$classify_block" | grep -Fqx '        run: scripts/classify-mutation-diff.sh "${{ steps.classify.outputs.mutation }}" "${{ steps.classify.outputs.base_sha }}" "${{ steps.classify.outputs.head_sha }}"' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fqx '          MUTATION_REQUIRED: ${{ needs.classify.outputs.mutation_diff }}'; then
        fail 'CI diff mutation must bind proven-empty selection to its aggregate requirement'
    fi
    for required in \
        '      native_mutation: ${{ steps.classify.outputs.native_mutation }}' \
        "          ALPINE_CI_ASSURANCE: \${{ github.event_name == 'workflow_dispatch' && inputs.assurance || false }}"
    do
        if ! printf '%s\n' "$classify_block" | grep -Fqx "$required"; then
            fail 'specialized assurance must be explicitly manual and separately select native mutation'
        fi
    done
    assurance_input=$(printf '%s\n' "$dispatch_block" | awk '
        /^      assurance:/ { capture = 1; next }
        capture && NF && !/^        / { exit }
        capture
    ')
    for required in '        default: false' '        type: boolean' '        required: false'; do
        if ! printf '%s\n' "$assurance_input" | grep -Fqx "$required"; then
            fail 'manual specialized assurance must default to false'
        fi
    done
    if ! printf '%s\n' "$ci_pass_block" | grep -Fqx '          NATIVE_MUTATION_REQUIRED: ${{ needs.classify.outputs.native_mutation }}'; then
        fail 'native mutation aggregate must use its independent opt-in selection'
    fi
    if grep -Eq '^  schedule:' "${ALPINE_NIGHTLY_ASSURANCE_WORKFLOW:-.github/workflows/nightly-assurance.yml}"; then
        fail 'nightly specialized assurance must remain manual only'
    fi
    for job in upstream-radar mutation coverage; do
        weekly_job=$(awk -v job="$job" '
            $0 == "  " job ":" { capture = 1; next }
            capture && /^  [A-Za-z0-9_-]+:/ { exit }
            capture
        ' "${ALPINE_WEEKLY_ASSURANCE_WORKFLOW:-.github/workflows/weekly-assurance.yml}")
        if ! printf '%s\n' "$weekly_job" | grep -Fqx "    if: github.event_name == 'workflow_dispatch'"; then
            fail 'weekly expensive assurance and project radar must remain manual only'
        fi
    done
    mutation_proof_block=$(printf '%s\n' "$classify_block" | awk '
        /^      - id: mutation-diff$/ { capture = 1; next }
        capture && /^      - / { exit }
        capture
    ')
    if ! printf '%s\n' "$mutation_proof_block" | grep -Fqx '        run: scripts/classify-mutation-diff.sh "${{ steps.classify.outputs.mutation }}" "${{ steps.classify.outputs.base_sha }}" "${{ steps.classify.outputs.head_sha }}"' \
        || printf '%s\n' "$mutation_proof_block" | grep -Eq '^[[:space:]]*(if|continue-on-error):' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fqx '              true|false) ;;' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fqx '              *) echo "$1 has an invalid requirement: $2" >&2; exit 1 ;;'; then
        fail 'CI emptiness proof must run unconditionally and aggregate requirements must be boolean'
    fi
    for tool_job_block in "$mutation_diff_block" "$native_mutation_block"; do
        if ! printf '%s\n' "$tool_job_block" | grep -Fqx '        run: scripts/prepare-mutation-tool.sh' \
            || ! printf '%s\n' "$tool_job_block" | grep -Fqx '        uses: actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830' \
            || ! printf '%s\n' "$tool_job_block" | grep -Fqx '          ALPINE_MUTATION_CACHE_SCOPE: ${{ github.ref }}' \
            || printf '%s\n' "$tool_job_block" | grep -Eq 'restore-keys:|enableCrossOsArchive:|cache-hit.*true'; then
            fail 'CI mutation tooling must validate exact scoped cache contents, including on a cache hit'
        fi
        tool_verification_block=$(printf '%s\n' "$tool_job_block" | awk '
            /^      - name: Verify or install pinned mutation tooling$/ { capture = 1; next }
            capture && /^      - / { exit }
            capture
        ')
        if ! printf '%s\n' "$tool_verification_block" | grep -Fqx '        run: scripts/prepare-mutation-tool.sh' \
            || printf '%s\n' "$tool_verification_block" | grep -Eq '^[[:space:]]*(if|continue-on-error):' \
            || ! printf '%s\n' "$tool_job_block" | grep -Fqx "          key: alpine-mutants-v1-\${{ runner.os }}-\${{ runner.arch }}-\${{ github.ref }}-\${{ hashFiles('rust-toolchain.toml', 'scripts/prepare-mutation-tool.sh') }}"; then
            fail 'CI mutation tool verification must run unconditionally with its exact scoped key'
        fi
    done
    for required_job in quality native coverage mutation-diff kani tla miri metal-validation native-mutation; do
        required_job_block=$(awk -v job="$required_job" '
            $0 == "  " job ":" { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != job ":" && capture { exit }
            capture
        ' "$ci_workflow")
        required_dependencies='    needs: [classify, preflight]'
        if [ "$required_job" = native-mutation ]; then
            required_dependencies='    needs: [classify, preflight, native]'
        fi
        if ! printf '%s\n' "$required_job_block" | grep -Fqx "$required_dependencies"; then
            fail "CI job $required_job must wait for the fast policy preflight"
        fi
        if [ "$required_job" = native ]; then
            native_admission_block=$(printf '%s\n' "$required_job_block" | awk '
                /^      - name: Require unmutated native admission before mutation fan-out$/ { capture = 1; next }
                capture && /^      - / { exit }
                capture
            ')
            for required in \
                "        if: matrix.name == 'macos-arm64' && needs.classify.outputs.metal == 'true'" \
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
    assurance_routing_guard="    if: github.event.workflow_run.conclusion == 'failure' || github.event.workflow_run.conclusion == 'timed_out'"
    if [ ! -x scripts/route-assurance-failures.sh ] \
        || ! grep -Fqx "$assurance_routing_guard" "$assurance_failure_workflow" \
        || ! grep -Fqx '        run: scripts/route-assurance-failures.sh' "$assurance_failure_workflow" \
        || ! grep -Fqx '  checks: read' "$assurance_failure_workflow"; then
        fail 'assurance routing must use the tested current-main failure router and read-only checks'
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
    if [ -z "$native_mutation_block" ] \
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Ec '^[[:space:]]+shard: ([0-9]|1[0-5])/16$')" -ne 32 ] \
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Ec 'cargo mutants --no-config ')" -ne 11 ] \
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Fc -- '--shard "${{ matrix.shard }}"')" -ne 11 ]; then
        fail 'pull-request native mutation must preserve all eleven scopes across sixteen deterministic shards'
    fi
    # Preserve sixteen logical shards while separating serial platform/Studio cost.
    if ! printf '%s\n' "$native_mutation_block" | awk '
        function row( key) {
            if (!pending) return
            key = domain ":" id
            if (id !~ /^[0-9]+$/ || id < 1 || id > 16 ||
                (domain != "platform" && domain != "studio") ||
                shard != (id - 1) "/16" || seen[key]++) invalid = 1
            rows++
            pending = 0
        }
        /^          - id:/ { row(); id = $3; domain = ""; shard = ""; pending = 1 }
        /^            domain:/ { domain = $2 }
        /^            shard:/ { shard = $2 }
        /^    env:/ { row() }
        /^    timeout-minutes:/ { job_timeout++; if ($2 != 30) invalid = 1 }
        /^      - name:/ { owner = ""; guard = ""; cap = 0 }
        /^        id: native-(platform|studio)-mutants$/ { owner = $2; owners[owner]++ }
        /^        if: matrix.domain ==/ { guard = $4; gsub(/\047/, "", guard) }
        /^        timeout-minutes:/ { cap = $2; caps++; if (cap != 24) invalid = 1 }
        /cargo mutants --no-config / {
            expected = index($0, "--file apps/alpine-studio/src/lib.rs ") ? "studio" : "platform"
            if (owner != "native-" expected "-mutants" || guard != expected || cap != 24) invalid = 1
            commands[expected]++
        }
        END {
            row()
            for (i = 1; i <= 16; i++)
                if (seen["platform:" i] != 1 || seen["studio:" i] != 1) invalid = 1
            if (rows != 32 || job_timeout != 1 || caps != 2 ||
                owners["native-platform-mutants"] != 1 || owners["native-studio-mutants"] != 1 ||
                commands["platform"] != 10 || commands["studio"] != 1) invalid = 1
            exit invalid
        }
    '; then
        fail 'native mutation placement must preserve disjoint domain/shard ownership and bounded execution'
    fi
    for required in \
        '      - name: Bind native mutation execution identity' \
        '          ALPINE_NATIVE_TOOLCHAIN=$(rustc -Vv)' \
        '          ALPINE_NATIVE_MUTATOR=$(cargo mutants --version)' \
        '          scripts/check-native-mutation-receipts.sh prepare "${{ matrix.domain }}" "${{ matrix.id }}" target' \
        '      - name: Require complete native mutation receipts' \
        '          scripts/check-native-mutation-receipts.sh finish "${{ matrix.domain }}" "${{ matrix.id }}" target "$EXECUTION_OUTCOME"' \
        '        uses: ./.github/actions/upload-required-artifact' \
        '            target/native-mutation-receipts-${{ matrix.domain }}-${{ matrix.id }}' \
        '          if-no-files-found: error'
    do
        if ! printf '%s\n' "$native_mutation_block" | grep -Fqx "$required"; then
            fail 'native mutation must retain identity-bound terminal receipts and blocking artifacts'
            break
        fi
    done
    if ! printf '%s\n' "$native_mutation_block" | awk '
        /^      - name:/ {
            if (required && !always) invalid = 1
            required = ($0 == "      - name: Require complete native mutation receipts" ||
                        $0 == "      - name: Upload native mutation evidence")
            always = 0
            if (required) count++
        }
        /^        if: always\(\)$/ { if (required) always = 1 }
        /continue-on-error:/ { invalid = 1 }
        END { if (required && !always) invalid = 1; exit (invalid || count != 2) }
    '; then
        fail 'native mutation receipts and blocking uploads must run after failed execution'
    fi
    for required in \
        '      ALPINE_NATIVE_HEAD: ${{ needs.classify.outputs.head_sha }}' \
        '      ALPINE_NATIVE_BASE: ${{ needs.classify.outputs.base_sha }}' \
        '          EXECUTION_OUTCOME: ${{ matrix.domain == '"'"'platform'"'"' && steps.native-platform-mutants.outcome || steps.native-studio-mutants.outcome }}'
    do
        if ! printf '%s\n' "$native_mutation_block" | grep -Fqx "$required"; then
            fail 'native mutation receipts must bind selected execution and source/base identity'
        fi
    done
    # Only native mutation copies change compiler mode. Keep independent
    # ordinary validation and all other jobs at the existing global default.
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
            else if (section == "jobs:" && job == "native-mutation" && in_environment &&
                $0 == "      CARGO_INCREMENTAL: \"1\"") native++
            else invalid = 1
        }
        END { exit (invalid || global != 1 || native != 1) }
    ' "${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}"; then
        fail 'CI mutation compilation mode must be explicit and scoped'
    fi
    if ! grep -Fqx 'scripts/test-native-mutation-receipts.sh' scripts/check.sh; then
        fail 'local quality gate must exercise native mutation receipt controls'
    fi
    for shard in 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
        if ! printf '%s\n' "$native_mutation_block" | grep -Fq "shard: $shard/16"; then
            fail "pull-request native mutation is missing shard $shard/16"
        fi
    done
    if [ -z "$mutation_diff_block" ] \
        || [ "$(printf '%s\n' "$mutation_diff_block" | grep -Ec '^[[:space:]]+shard: [0-7]/8$')" -ne 8 ] \
        || [ "$(printf '%s\n' "$mutation_diff_block" | grep -Ec 'cargo mutants ')" -ne 2 ] \
        || [ "$(printf '%s\n' "$mutation_diff_block" | grep -Fc -- '--shard "${{ matrix.shard }}"')" -ne 2 ] \
        || ! printf '%s\n' "$mutation_diff_block" | grep -Fq 'name: mutation-${{ matrix.id }}-${{ github.sha }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'MUTATION_RESULT: ${{ needs.mutation-diff.result }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'require_selected mutation-diff "$MUTATION_REQUIRED" "$MUTATION_RESULT"'; then
        fail 'changed-code mutation must preserve shipping and assurance scopes across eight deterministic exact-head shards'
    fi
    for shard in 0 1 2 3 4 5 6 7; do
        if ! printf '%s\n' "$mutation_diff_block" | grep -Fq "shard: $shard/8"; then
            fail "changed-code mutation is missing shard $shard/8"
        fi
    done
    if ! printf '%s\n' "$mutation_diff_block" | grep -Fq -- "--exclude 'apps/alpine-studio/src/native_validation/accessibility_process.rs'" \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq -- '--file apps/alpine-studio/src/native_validation/accessibility_process.rs' \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq 'ALPINE_STUDIO_NATIVE_PROCESS_SCOPE=accessibility' \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq 'target/native-studio-accessibility-process-mutants-${{ matrix.id }}.out'; then
        fail 'Studio accessibility process mutation must transfer explicitly from Linux to accessibility-scoped retained native shards'
    fi
    if ! printf '%s\n' "$mutation_diff_block" | grep -Fq -- "--exclude 'tools/alpine-ax-client/src/native.rs'" \
        || ! printf '%s\n' "$mutation_diff_block" | grep -Fq -- "--exclude 'tools/alpine-ax-client/src/native_factory.rs'" \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq -- 'if [ -f tools/alpine-ax-client/src/native_factory.rs ]; then' \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq -- '--file tools/alpine-ax-client/src/native_factory.rs' \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq -- '--test-package alpine-ax-client' \
        || ! printf '%s\n' "$native_mutation_block" | grep -Fq 'target/native-ax-client-factory-mutants-${{ matrix.id }}.out'; then
        fail 'AX target-only mutation must leave Linux explicitly and retain one conditional Apple factory owner'
    fi
    for language_evidence_owner in \
        reset_native_validation_language_evidence \
        record_native_validation_language_snapshot \
        record_native_validation_language_publication \
        record_native_validation_language_submission \
        record_native_validation_language_observation \
        native_validation_language_evidence; do
        if ! printf '%s\n' "$mutation_diff_block" | grep -Fq "$language_evidence_owner" \
            || ! printf '%s\n' "$native_mutation_block" | grep -Fq "$language_evidence_owner"; then
            fail 'validation-only Studio language evidence mutation must transfer explicitly from Linux to retained Apple native shards'
        fi
    done
    if printf '%s\n' "$metal_validation_block" | grep -Fq 'cargo mutants '; then
        fail 'Metal behavior validation must remain independent from native mutation enforcement'
    fi
    if ! printf '%s\n' "$native_mutation_block" | grep -Fq 'name: native-mutation-${{ matrix.domain }}-${{ matrix.id }}-${{ github.sha }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'native-mutation]' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'NATIVE_MUTATION_RESULT: ${{ needs.native-mutation.result }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'require_selected native-mutation "$NATIVE_MUTATION_REQUIRED" "$NATIVE_MUTATION_RESULT"'; then
        fail 'ci-pass must require and retain exact-head native mutation matrix evidence'
    fi

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

    nightly_native_workflow=${ALPINE_NIGHTLY_ASSURANCE_WORKFLOW:-.github/workflows/nightly-assurance.yml}
    if [ -f "$nightly_native_workflow" ]; then
        check_mutation_baseline "$nightly_native_workflow"
        if ! awk '
            function finish() {
                if (helper && (!always || !name || !path || !retention || !required)) exit 1
                if (direct && (!always || !name || !path || !retention || !supplementary)) exit 1
            }
            /^      - name:/ {
                finish()
                helper = direct = always = name = path = retention = required = supplementary = 0
            }
            /^        if: always\(\)$/ { always = 1 }
            /uses: \.\/\.github\/actions\/upload-required-artifact$/ { helper = 1; helper_count++ }
            /uses: actions\/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a$/ { direct = 1; direct_count++ }
            /^[[:space:]]+name:/ { name = 1 }
            /^[[:space:]]+path:/ { path = 1 }
            /^[[:space:]]+retention-days:/ { retention = 1 }
            /^[[:space:]]+if-no-files-found: error$/ { required = 1 }
            /^[[:space:]]+if-no-files-found: warn$/ { supplementary = 1 }
            END {
                finish()
                if (helper_count != 8 || direct_count != 1) exit 1
            }
        ' "$nightly_native_workflow"; then
            fail 'Nightly required artifacts must use eight governed retries and retain one direct supplementary upload'
        fi
        nightly_metal_block=$(awk '
            /^  metal-validation:/ { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != "metal-validation:" && capture { exit }
            capture
        ' "$nightly_native_workflow")
        nightly_accessibility_block=$(awk '
            /^  native-accessibility-mutation:/ { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != "native-accessibility-mutation:" && capture { exit }
            capture
        ' "$nightly_native_workflow")
        nightly_platform_contract_block=$(awk '
            /^  native-platform-contract-mutation:/ { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != "native-platform-contract-mutation:" && capture { exit }
            capture
        ' "$nightly_native_workflow")
        nightly_studio_accessibility_block=$(awk '
            /^  native-studio-accessibility-mutation:/ { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != "native-studio-accessibility-mutation:" && capture { exit }
            capture
        ' "$nightly_native_workflow")
        if ! grep -Fq 'native-platform-contract-mutation:' "$nightly_native_workflow" \
            || ! grep -Fq 'native-accessibility-mutation:' "$nightly_native_workflow" \
            || ! grep -Fq 'native-studio-accessibility-mutation:' "$nightly_native_workflow" \
            || [ "$(grep -Ec '^[[:space:]]+shard: [0-7]/8$' "$nightly_native_workflow")" -ne 16 ] \
            || [ "$(grep -Fc -- '--file crates/alpine-platform-macos/src/native_accessibility.rs' "$nightly_native_workflow")" -ne 1 ] \
            || [ "$(grep -Fc -- '--file apps/alpine-studio/src/native_validation/accessibility_process.rs' "$nightly_native_workflow")" -ne 1 ] \
            || [ "$(grep -Fc 'ALPINE_STUDIO_NATIVE_PROCESS_SCOPE=accessibility' "$nightly_native_workflow")" -ne 1 ] \
            || [ "$(grep -Fc -- "--file apps/alpine-studio/src/lib.rs --re 'reset_native_validation_language_evidence|record_native_validation_language_snapshot|record_native_validation_language_publication|record_native_validation_language_submission|record_native_validation_language_observation|native_validation_language_evidence'" "$nightly_native_workflow")" -ne 1 ] \
            || ! grep -Fq -- '--shard "${{ matrix.shard }}"' "$nightly_native_workflow" \
            || ! grep -Fq 'target/native-accessibility-mutants-${{ matrix.id }}.out' "$nightly_native_workflow" \
            || ! grep -Fq 'target/native-studio-accessibility-process-mutants-${{ matrix.id }}.out' "$nightly_native_workflow" \
            || ! grep -Fq 'target/native-studio-language-evidence-mutants-${{ matrix.id }}.out' "$nightly_native_workflow"; then
            fail 'nightly assurance must exhaustively shard and retain native accessibility and Studio process mutation evidence'
        fi
        if [ "$(printf '%s\n' "$nightly_platform_contract_block" | grep -Ec '^[[:space:]]+shard: [0-3]/4$')" -ne 4 ] \
            || [ "$(printf '%s\n' "$nightly_platform_contract_block" | grep -Fc -- '--file crates/alpine-platform-macos/src/lib.rs')" -ne 2 ] \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq -- '--shard "${{ matrix.shard }}"' \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq -- '--test-package alpine-studio' \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq -- "--re 'native_validation::arm_programmatic_window_close|native_validation::commit_native_text'" \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq 'Hosted AppKit cannot qualify user-facing `performClose`; physical Tasks #72 and #253 own that contract.' \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq 'target/native-platform-contract-mutants-${{ matrix.id }}.out' \
            || ! printf '%s\n' "$nightly_platform_contract_block" | grep -Fq 'target/native-platform-studio-contract-mutants.out' \
            || printf '%s\n' "$nightly_metal_block" | grep -Fq -- '--file crates/alpine-platform-macos/src/lib.rs'; then
            fail 'nightly assurance must shard native platform contracts and route Studio-only wrappers through Studio tests'
        fi
        for shard in 0 1 2 3 4 5 6 7; do
            if [ "$(grep -Fc "shard: $shard/8" "$nightly_native_workflow")" -ne 2 ]; then
                fail "nightly native accessibility mutation scopes are missing shard $shard/8"
            fi
        done
        native_metal_mutation_count=$(printf '%s\n' "$nightly_metal_block" \
            | grep -Fc -- '--file crates/alpine-metal/src/native.rs')
        native_metal_mutation_line=$(printf '%s\n' "$nightly_metal_block" \
            | grep -F -- '--file crates/alpine-metal/src/native.rs' || true)
        if [ "$native_metal_mutation_count" -ne 1 ] \
            || ! printf '%s\n' "$native_metal_mutation_line" \
                | grep -Fq -- "--exclude-re 'tests::'"; then
            fail 'nightly Metal mutation must exclude exactly the native.rs test-helper namespace'
        fi
        remaining_metal_exclusions=$(printf '%s\n' "$native_metal_mutation_line" \
            | sed "s/--exclude-re 'tests::'//")
        if printf '%s\n' "$remaining_metal_exclusions" | grep -Fq -- '--exclude-re'; then
            fail 'nightly Metal shipping mutation must not add another native.rs exclusion'
        fi
        for mutation_block in \
            "$nightly_platform_contract_block" \
            "$nightly_accessibility_block" \
            "$nightly_studio_accessibility_block"; do
            output_parent_line=$(printf '%s\n' "$mutation_block" \
                | grep -nF 'mkdir -p target' | head -n 1 | cut -d: -f1 || true)
            mutation_line=$(printf '%s\n' "$mutation_block" \
                | grep -nF 'cargo mutants ' | head -n 1 | cut -d: -f1 || true)
            if [ -z "$output_parent_line" ] || [ -z "$mutation_line" ] \
                || [ "$output_parent_line" -ge "$mutation_line" ]; then
                fail 'nightly native accessibility mutation must prepare its target output parent before cargo-mutants starts'
            fi
        done
    fi

    weekly_mutation_workflow=.github/workflows/weekly-assurance.yml
    if [ -f "$weekly_mutation_workflow" ]; then
        output_parent_line=$(grep -nF 'mkdir -p target' "$weekly_mutation_workflow" \
            | head -n 1 | cut -d: -f1 || true)
        mutation_line=$(grep -nF 'cargo mutants --workspace' "$weekly_mutation_workflow" \
            | head -n 1 | cut -d: -f1 || true)
        if [ -z "$output_parent_line" ] || [ -z "$mutation_line" ] \
            || [ "$output_parent_line" -ge "$mutation_line" ]; then
            fail 'weekly mutation must prepare its target output parent before cargo-mutants starts'
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
crates/alpine-platform-macos/src/native.rs
crates/alpine-platform-macos/src/native_accessibility.rs
crates/alpine-platform-macos/src/signpost.rs
crates/alpine-text-layout/src/native.rs
tools/alpine-ax-client/src/native.rs'
if [ "$unsafe_source_files" != "$expected_unsafe_source_files" ]; then
    fail 'unsafe Rust constructs must remain isolated in audited native boundary files'
    printf '%s\n' "$unsafe_source_files" >&2
fi

if [ ! -f "$tla_driver" ]; then
    fail "TLA+ driver is missing: $tla_driver"
elif ! grep -Fq 'pull-request) config=PullRequest.cfg; lncheck=default ;;' "$tla_driver" \
    || ! grep -Fq 'nightly) config=Nightly.cfg; lncheck=final ;;' "$tla_driver" \
    || [ "$(grep -Fc -- '-lncheck "$lncheck"' "$tla_driver" || true)" -ne 2 ]; then
    fail 'TLA+ must preserve default pull-request checks and final-graph Nightly liveness checks'
fi

# Native surface mutation proof must remain complete, deterministic, retained,
# and separate from the serial Metal ownership job.
nightly_assurance_workflow="${ALPINE_NIGHTLY_ASSURANCE_WORKFLOW:-.github/workflows/nightly-assurance.yml}"
ci_workflow="${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}"
native_surface_mutation_job="$(sed -n '/^  native-surface-mutation:/,/^  [A-Za-z0-9_-][A-Za-z0-9_-]*:$/p' "${nightly_assurance_workflow}")"
native_studio_contract_mutation_job="$(sed -n '/^  native-studio-contract-mutation:/,/^  [A-Za-z0-9_-][A-Za-z0-9_-]*:$/p' "${nightly_assurance_workflow}")"
metal_validation_job="$(sed -n '/^  metal-validation:/,/^  [A-Za-z0-9_-][A-Za-z0-9_-]*:$/p' "${nightly_assurance_workflow}")"
ci_native_mutation_job="$(sed -n '/^  native-mutation:/,/^  [A-Za-z0-9_-][A-Za-z0-9_-]*:$/p' "${ci_workflow}")"
ci_pass_job="$(sed -n '/^  ci-pass:/,/^  [A-Za-z0-9_-][A-Za-z0-9_-]*:$/p' "${ci_workflow}")"

if [ -z "${native_surface_mutation_job}" ]; then
  echo "policy failure: Nightly assurance must define native-surface-mutation" >&2
  exit 1
fi
if ! printf '%s\n' "${native_surface_mutation_job}" | awk '
    /^[[:space:]]+timeout-minutes:/ {
        timeouts++
        if ($2 != 30) invalid = 1
    }
    /^[[:space:]]+- id:/ {
        id = $3
        count++
        if (waiting || id != count || id < 1 || id > 16 || seen[id]++) invalid = 1
        waiting = 1
    }
    /^[[:space:]]+shard:/ {
        expected = sprintf("\"%d/16\"", id - 1)
        if (!waiting || $2 != expected) invalid = 1
        waiting = 0
        shards++
    }
    END { exit (invalid || waiting || count != 16 || shards != 16 || timeouts != 1) }
'; then
  fail 'native surface mutation must retain exactly sixteen unique ordered shards and artifact IDs within the 30-minute budget'
fi
native_surface_scope_count="$(printf '%s\n' "${native_surface_mutation_job}" | grep -Fc -- '--file crates/alpine-platform-macos/src/native.rs' || true)"
if [ "${native_surface_scope_count}" -ne 1 ]; then
  echo "policy failure: native surface mutation must scope native.rs exactly once" >&2
  exit 1
fi
if ! printf '%s\n' "${native_surface_mutation_job}" | grep -Fq -- "--exclude-re 'validate_initialization_rollback|run_until_frame_terminal|stop_validation_event_loop|schedule_validation_window_close|schedule_validation_user_window_close|NativeSurface::arm_window_close|NativeSurface::arm_user_window_close'"; then
  echo "policy failure: native surface mutation must preserve the reviewed physical-only exclusions" >&2
  exit 1
fi
for required in '--test-package alpine-platform-macos' '--test-package alpine-studio' '--no-shuffle' '--sharding round-robin' '--shard "${{ matrix.shard }}"' 'mkdir -p target' 'target/native-surface-mutants-${{ matrix.id }}.out' 'if-no-files-found: error'; do
  if ! printf '%s\n' "${native_surface_mutation_job}" | grep -Fq -- "${required}"; then
    echo "policy failure: native surface mutation is missing ${required}" >&2
    exit 1
  fi
done
if [ -z "${native_studio_contract_mutation_job}" ]; then
  echo "policy failure: Nightly assurance must define native-studio-contract-mutation" >&2
  exit 1
fi
for shard in 0 1 2 3 4 5 6 7; do
  if ! printf '%s\n' "${native_studio_contract_mutation_job}" | grep -Fq "shard: \"${shard}/8\""; then
    echo "policy failure: Studio native contract mutation must retain deterministic shard ${shard}/8" >&2
    exit 1
  fi
done
for required in '--file crates/alpine-runtime/src/lib.rs' '--file apps/alpine-studio/src/lib.rs' '--test-package alpine-studio' '-- --locked native_process' 'initial_scene' '--shard "${{ matrix.shard }}"' 'target/native-studio-contract-mutants-${{ matrix.id }}.out' 'if-no-files-found: error'; do
  if ! printf '%s\n' "${native_studio_contract_mutation_job}" | grep -Fq -- "${required}"; then
    echo "policy failure: Studio native contract mutation is missing ${required}" >&2
    exit 1
  fi
done
for forbidden in '--file crates/alpine-platform-macos/src/native.rs' '--file crates/alpine-runtime/src/lib.rs' '--file apps/alpine-studio/src/lib.rs'; do
  if printf '%s\n' "${metal_validation_job}" | grep -Fq -- "${forbidden}"; then
    echo "policy failure: serial metal-validation must not absorb ${forbidden}" >&2
    exit 1
  fi
done

ci_native_surface_scope="$(printf '%s\n' "${ci_native_mutation_job}" | grep -- '--file crates/alpine-platform-macos/src/native.rs' || true)"
ci_native_studio_scope="$(printf '%s\n' "${ci_native_mutation_job}" | grep -- '--file apps/alpine-studio/src/lib.rs' || true)"
ci_native_runtime_scope="$(printf '%s\n' "${ci_native_mutation_job}" | grep -- '--file crates/alpine-runtime/src/lib.rs' || true)"
if ! printf '%s\n' "${ci_native_surface_scope}" | grep -Fq -- '--sharding round-robin'; then
  echo "policy failure: exact-head native surface mutation must retain round-robin partitioning" >&2
  exit 1
fi
for scope in "${ci_native_surface_scope}" "${ci_native_studio_scope}"; do
  if [ -z "${scope}" ] || printf '%s\n' "${scope}" | grep -Fq -- '--in-diff'; then
    echo "policy failure: exact-head native process mutation scopes must be exhaustive" >&2
    exit 1
  fi
  if ! printf '%s\n' "${scope}" | grep -Fq -- '--test-package alpine-studio'; then
    echo "policy failure: exact-head native process mutation scopes must run Studio process tests" >&2
    exit 1
  fi
done
if [ -z "${ci_native_runtime_scope}" ] \
  || ! printf '%s\n' "${ci_native_runtime_scope}" | grep -Fq -- '--test-package alpine-studio' \
  || ! printf '%s\n' "${ci_native_runtime_scope}" | grep -Fq -- '-- --locked native_process'; then
  echo "policy failure: exact-head runtime mutation must use the bounded Studio native process control" >&2
  exit 1
fi
if ! printf '%s\n' "${ci_pass_job}" | grep -Fq 'native-mutation'; then
  echo "policy failure: ci-pass must require exact-head native mutation evidence" >&2
  exit 1
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
