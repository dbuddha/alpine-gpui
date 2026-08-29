#!/bin/sh
set -eu

failures=0
tla_driver=${ALPINE_TLA_DRIVER:-scripts/check-tla.sh}

fail() {
    printf 'policy error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

if [ "${GITHUB_EVENT_NAME:-}" = "pull_request" ] && [ -n "${GITHUB_REPOSITORY:-}" ] && [ -f "${GITHUB_EVENT_PATH:-}" ] && command -v gh >/dev/null 2>&1; then
    pull_request_number=$(jq -r '.pull_request.number // empty' "$GITHUB_EVENT_PATH" 2>/dev/null || true)
    if [ -z "$pull_request_number" ]; then
        pull_request_number=$(printf '%s\n' "${GITHUB_REF:-}" | sed -n 's#refs/pull/\([0-9]\+\)/.*#\1#p' || true)
    fi

    if [ -n "$pull_request_number" ]; then
        refreshed_pr_title=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json title --jq .title 2>/dev/null || true)
        refreshed_pr_body=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json body --jq .body 2>/dev/null || true)
        refreshed_pr_labels=$(gh pr view "$pull_request_number" --repo "$GITHUB_REPOSITORY" --json labels --jq '[.labels[].name] | join(",")' 2>/dev/null || true)

        if [ -n "$refreshed_pr_title" ]; then
            ALPINE_PR_TITLE=$refreshed_pr_title
        fi
        if [ -n "$refreshed_pr_body" ]; then
            ALPINE_PR_BODY=$refreshed_pr_body
        fi
        if [ -n "$refreshed_pr_labels" ]; then
            ALPINE_PR_LABELS=$refreshed_pr_labels
        fi
    fi
fi

workflow_files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) -print)

if [ -n "$workflow_files" ]; then
    ci_workflow=${ALPINE_CI_WORKFLOW:-.github/workflows/ci.yml}
    action_refs=$(grep -hE '^[[:space:]]*uses:' $workflow_files || true)
    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >/dev/null; then
        fail 'every GitHub Action must be pinned to a full commit SHA'
        printf '%s\n' "$action_refs" | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' >&2 || true
    fi

    if [ -n "$action_refs" ] && printf '%s\n' "$action_refs" | grep -Ev 'uses:[[:space:]]+(actions|github)/' >/dev/null; then
        fail 'only GitHub-owned Actions are permitted'
        printf '%s\n' "$action_refs" | grep -Ev 'uses:[[:space:]]+(actions|github)/' >&2 || true
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
    ci_pass_block=$(awk '
        /^  ci-pass:/ { capture = 1 }
        capture
    ' "$ci_workflow")
    if ! printf '%s\n' "$ci_pass_block" | grep -Fqx '    if: ${{ always() && !cancelled() }}'; then
        fail 'ci-pass must run after ordinary failures but skip a canceled workflow'
    fi
    if ! printf '%s\n' "$ci_pass_block" | grep -Fq 'test "$2" = success || {'; then
        fail 'ci-pass must reject every required result other than success'
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
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Ec '^[[:space:]]+shard: [0-7]/8$')" -ne 8 ] \
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Ec 'cargo mutants ')" -ne 10 ] \
        || [ "$(printf '%s\n' "$native_mutation_block" | grep -Fc -- '--shard "${{ matrix.shard }}"')" -ne 10 ]; then
        fail 'pull-request native mutation must preserve all ten scopes across eight deterministic shards'
    fi
    for shard in 0 1 2 3 4 5 6 7; do
        if ! printf '%s\n' "$native_mutation_block" | grep -Fq "shard: $shard/8"; then
            fail "pull-request native mutation is missing shard $shard/8"
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
    if ! printf '%s\n' "$native_mutation_block" | grep -Fq 'name: native-mutation-${{ matrix.id }}-${{ github.sha }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'native-mutation]' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'NATIVE_MUTATION_RESULT: ${{ needs.native-mutation.result }}' \
        || ! printf '%s\n' "$ci_pass_block" | grep -Fq 'require_selected native-mutation "$METAL_REQUIRED" "$NATIVE_MUTATION_RESULT"'; then
        fail 'ci-pass must require and retain exact-head native mutation matrix evidence'
    fi

    nightly_native_workflow=.github/workflows/nightly-assurance.yml
    if [ -f "$nightly_native_workflow" ]; then
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
        nightly_studio_accessibility_block=$(awk '
            /^  native-studio-accessibility-mutation:/ { capture = 1 }
            /^  [A-Za-z0-9_-]+:/ && $1 != "native-studio-accessibility-mutation:" && capture { exit }
            capture
        ' "$nightly_native_workflow")
        if ! grep -Fq 'native-accessibility-mutation:' "$nightly_native_workflow" \
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

manifest_files=$(find . -name Cargo.toml -not -path './target/*' -print)
if [ -n "$manifest_files" ] && grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >/dev/null; then
    fail 'shipping Cargo manifests may not contain Git dependencies'
    grep -nE 'git[[:space:]]*=[[:space:]]*"https?://' $manifest_files >&2 || true
fi

unsafe_override_files=$(grep -lE '^unsafe_code[[:space:]]*=[[:space:]]*"allow"' $manifest_files 2>/dev/null | sort || true)
expected_unsafe_override_files='./crates/alpine-metal/Cargo.toml
./crates/alpine-platform-macos/Cargo.toml
./crates/alpine-text-layout/Cargo.toml'
if [ "$unsafe_override_files" != "$expected_unsafe_override_files" ]; then
    fail 'only audited native Metal and macOS platform crates may override unsafe-code denial'
    printf '%s\n' "$unsafe_override_files" >&2
fi

unsafe_source_files=$(find crates -type f -name '*.rs' -print0 \
    | xargs -0 grep -lE 'unsafe[[:space:]]+(extern|fn|impl|trait)|unsafe[[:space:]]*\{' 2>/dev/null \
    | sort || true)
expected_unsafe_source_files='crates/alpine-metal/src/native.rs
crates/alpine-platform-macos/src/native.rs
crates/alpine-platform-macos/src/native_accessibility.rs
crates/alpine-platform-macos/src/signpost.rs
crates/alpine-text-layout/src/native.rs'
if [ "$unsafe_source_files" != "$expected_unsafe_source_files" ]; then
    fail 'unsafe Rust constructs must remain isolated in audited native boundary files'
    printf '%s\n' "$unsafe_source_files" >&2
fi

old_project='Ro''ck GPUI'
old_probe='ro''ck-metal-probe'
if git grep -n -I -e "$old_project" -e "$old_probe" -- . ':!scripts/check-policy.sh' >/dev/null 2>&1; then
    fail 'retired project names remain in tracked files'
    git grep -n -I -e "$old_project" -e "$old_probe" -- . ':!scripts/check-policy.sh' >&2 || true
fi

for retired_path in \
    PRODUCT.md \
    CHANGELOG.md \
    CONTRIBUTING.md \
    crates/AGENTS.md \
    crates/README.md \
    .github/AGENTS.md
do
    if [ -e "$retired_path" ]; then
        fail "unexpected standing artifact: $retired_path"
    fi
done

for retired_directory in changes; do
    if [ -d "$retired_directory" ] && find "$retired_directory" -type f -print -quit | grep -q .; then
        fail "unexpected files under retired directory: $retired_directory"
    fi
done

deleted_references='PRODUCT\.md|CHANGELOG\.md|changes/|docs/(MASTER_PLAN|ROADMAP|adr|DEPENDENCIES|ci|engineering)|crates/(AGENTS|README)\.md|provenance-ledger'
if [ "${ALPINE_POLICY_REFERENCE_INPUT+x}" = x ]; then
    retired_reference_matches=$(printf '%s\n' "$ALPINE_POLICY_REFERENCE_INPUT" \
        | grep -nE "$deleted_references" || true)
else
    retired_reference_matches=$(git grep -n -I -E "$deleted_references" -- . ':!scripts/check-policy.sh' || true)
fi
if [ -n "$retired_reference_matches" ]; then
    fail 'tracked files reference retired repository documents'
    printf '%s\n' "$retired_reference_matches" >&2
fi

research_files=$(find docs/research -type f -print 2>/dev/null | sort || true)
expected_research_files='docs/research/alpine-lineage/adversarial-review.md
docs/research/alpine-lineage/alpine-decisions.md
docs/research/alpine-lineage/evidence-ledger.md
docs/research/alpine-lineage/experiments.md
docs/research/alpine-lineage/framework-lineage.md
docs/research/alpine-lineage/history.md
docs/research/alpine-lineage/index.md
docs/research/alpine-lineage/methodology.md
docs/research/alpine-lineage/references.bib
docs/research/alpine-lineage/source-map.md
docs/research/alpine-lineage/studio-lineage.md
docs/research/alpine-studio-adversarial-review.md
docs/research/index.md
docs/research/macos-accessibility-lifecycle/decisions.md
docs/research/macos-accessibility-lifecycle/experiments.md
docs/research/macos-accessibility-lifecycle/findings.md
docs/research/macos-accessibility-lifecycle/index.md
docs/research/macos-accessibility-lifecycle/source-map.md
docs/research/native-idle-energy/decisions.md
docs/research/native-idle-energy/experiments.md
docs/research/native-idle-energy/findings.md
docs/research/native-idle-energy/index.md
docs/research/native-idle-energy/source-map.md
docs/research/wgpu/decisions.md
docs/research/wgpu/experiments.md
docs/research/wgpu/findings.md
docs/research/wgpu/index.md
docs/research/wgpu/source-map.md'
if [ "$research_files" != "$expected_research_files" ]; then
    fail 'docs/research may contain only accepted catalog, review, and decision-grade package artifacts'
    printf '%s\n' "$research_files" >&2
fi

agent_lines=$(wc -l < AGENTS.md | tr -d ' ')
if [ "$agent_lines" -lt 80 ] || [ "$agent_lines" -gt 120 ]; then
    fail "AGENTS.md must remain between 80 and 120 lines, found $agent_lines"
fi

if ! sed -n '1,4p' AGENTS.md | grep -q '^schema: alpine-agent-policy/v1$'; then
    fail 'AGENTS.md must declare schema alpine-agent-policy/v1 in frontmatter'
fi

for required_path in \
    book.toml \
    docs/SUMMARY.md \
    docs/vision.md \
    docs/concepts/traceability.md \
    docs/quality/assurance.md \
    assurance/evidence.toml \
    assurance/alpine-studio-dependencies.txt \
    scripts/check-product-boundary.sh \
    scripts/test-product-boundary.sh \
    scripts/test-assurance.sh \
    scripts/test-metal-library.sh \
    scripts/verify-metal-library.sh \
    scripts/check-workload-hashes.sh \
    scripts/check-research-retention.sh \
    scripts/test-research-retention.sh \
    scripts/check-tla.sh \
    docs/research/index.md \
    docs/research/alpine-lineage/index.md \
    docs/research/alpine-lineage/methodology.md \
    docs/research/alpine-lineage/source-map.md \
    docs/research/alpine-lineage/framework-lineage.md \
    docs/research/alpine-lineage/studio-lineage.md \
    docs/research/alpine-lineage/evidence-ledger.md \
    docs/research/alpine-lineage/history.md \
    docs/research/alpine-lineage/adversarial-review.md \
    docs/research/alpine-lineage/experiments.md \
    docs/research/alpine-lineage/alpine-decisions.md \
    docs/research/alpine-lineage/references.bib \
    docs/research/alpine-studio-adversarial-review.md \
    docs/research/wgpu/index.md \
    docs/research/wgpu/source-map.md \
    docs/research/wgpu/findings.md \
    docs/research/wgpu/experiments.md \
    docs/research/wgpu/decisions.md \
    .cargo/mutants.toml
do
    if [ ! -f "$required_path" ]; then
        fail "required assurance artifact is missing: $required_path"
    fi
done

if [ ! -f "$tla_driver" ]; then
    fail "TLA+ driver is missing: $tla_driver"
elif ! grep -Fq 'pull-request) config=PullRequest.cfg; lncheck=default ;;' "$tla_driver" \
    || ! grep -Fq 'nightly) config=Nightly.cfg; lncheck=final ;;' "$tla_driver" \
    || [ "$(grep -Fc -- '-lncheck "$lncheck"' "$tla_driver" || true)" -ne 2 ]; then
    fail 'TLA+ must preserve default pull-request checks and final-graph Nightly liveness checks'
fi

issue_form_count=$(find .github/ISSUE_TEMPLATE -maxdepth 1 -type f -name '*.yml' ! -name config.yml | wc -l | tr -d ' ')
if [ "$issue_form_count" -ne 5 ]; then
    fail "exactly five structured issue forms are required, found $issue_form_count"
fi

if [ ! -f .github/ISSUE_TEMPLATE/capability.yml ] || [ -f .github/ISSUE_TEMPLATE/user-journey.yml ]; then
    fail 'the top-level work item must be the capability issue form'
fi

for release_label in release:breaking release:feature release:fix release:none; do
    if ! grep -q -- "- $release_label" .github/release.yml; then
        fail "release-note configuration is missing $release_label"
    fi
done

if [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; then
    release_label_count=$(printf '%s\n' "${ALPINE_PR_LABELS:-}" | tr ',' '\n' | grep -Ec '^release:(breaking|feature|fix|none)$' || true)
    if [ "$release_label_count" -ne 1 ]; then
        fail "pull requests require exactly one release-impact label, found $release_label_count"
    fi

    if ! printf '%s\n' "${ALPINE_PR_TITLE:-}" | grep -Eq '^(build|chore|ci|docs|feat|fix|perf|refactor|revert|test)(\([a-z0-9_-]+\))?!?: .+'; then
        fail 'pull request title must use a Conventional Commit summary'
    fi

    for heading in \
        '## Closing issue' \
        '## Parent capability' \
        '## Claims and evidence' \
        '## Decision or research' \
        '## Acceptance evidence' \
        '## Risk and scope' \
        '## Test plan' \
        '## Performance and memory' \
        '## Release impact' \
        '## Dependencies, provenance, and unsafe code' \
        '## Adversarial review'
    do
        if ! printf '%s\n' "$ALPINE_PR_BODY" | grep -Fqx "$heading"; then
            fail "pull request body is missing required heading: $heading"
        fi
    done
fi

if [ -n "${ALPINE_BASE_SHA:-}" ] && [ -n "${ALPINE_HEAD_SHA:-}" ]; then
    if [ -n "${ALPINE_CHANGED_FILES:-}" ]; then
        changed_files=$ALPINE_CHANGED_FILES
    else
        changed_files=$(git diff --name-only "$ALPINE_BASE_SHA...$ALPINE_HEAD_SHA")
    fi
    implementation_changes=$(printf '%s\n' "$changed_files" | grep -E '^(Cargo\.toml$|Cargo\.lock$|crates/.+/Cargo\.toml$|crates/.+\.rs$|tools/alpine-trace/|shaders/)' || true)

    if [ -n "$implementation_changes" ] && { [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; }; then
        closing_issue=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | sed -nE 's/.*([Cc]loses|[Cc]ontributes to)[[:space:]]+#([0-9]+).*/\2/p' | head -n 1)
        if [ -z "$closing_issue" ]; then
            fail 'implementation pull requests must close or contribute to a requirement or task issue'
        elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
            issue_labels=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
            issue_state=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
            issue_kind=$(printf '%s\n' "$issue_labels" | grep -E '^kind:(requirement|task)$' || true)
            if [ -z "$issue_kind" ]; then
                fail "linked issue #$closing_issue must have kind:requirement or kind:task"
            elif [ "$issue_state" != OPEN ]; then
                fail "linked issue #$closing_issue must be open"
            else
                issue_body=$(gh issue view "$closing_issue" --repo "$GH_REPOSITORY" --json body --jq .body 2>/dev/null || true)
                if printf '%s\n' "$issue_kind" | grep -Fxq kind:task; then
                    requirement_issue=$(printf '%s\n' "$issue_body" | awk '/^### Parent capability or requirement$/{capture=1; next} /^### /{if (capture) exit} capture' | grep -Eo '#[0-9]+' | tr -d '#' | head -n 1 || true)
                else
                    requirement_issue=$closing_issue
                fi

                if [ -z "${requirement_issue:-}" ]; then
                    fail "linked task #$closing_issue must name its parent requirement"
                else
                    if printf '%s\n' "$issue_kind" | grep -Fxq kind:task; then
                        native_requirement=$(gh api "repos/$GH_REPOSITORY/issues/$closing_issue/parent" --jq .number 2>/dev/null || true)
                        if [ "$native_requirement" != "$requirement_issue" ]; then
                            fail "task #$closing_issue must be a native sub-issue of requirement #$requirement_issue"
                        fi
                    fi

                    requirement_labels=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
                    requirement_state=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
                    if ! printf '%s\n' "$requirement_labels" | grep -Fxq kind:requirement; then
                        fail "parent issue #$requirement_issue must have kind:requirement"
                    fi
                    if [ "$requirement_state" != OPEN ]; then
                        fail "requirement #$requirement_issue must be open"
                    fi
                    if ! printf '%s\n' "$requirement_labels" | grep -Fxq owner:approved; then
                        fail "requirement #$requirement_issue requires owner:approved"
                    fi

                    requirement_body=$(gh issue view "$requirement_issue" --repo "$GH_REPOSITORY" --json body --jq .body 2>/dev/null || true)
                    capability_issue=$(printf '%s\n' "$requirement_body" | awk '/^### Parent capability or requirement$/{capture=1; next} /^### /{if (capture) exit} capture' | grep -Eo '#[0-9]+' | tr -d '#' | head -n 1 || true)
                    if [ -z "$capability_issue" ]; then
                        fail "requirement #$requirement_issue must name its parent capability"
                    else
                        native_capability=$(gh api "repos/$GH_REPOSITORY/issues/$requirement_issue/parent" --jq .number 2>/dev/null || true)
                        if [ "$native_capability" != "$capability_issue" ]; then
                            fail "requirement #$requirement_issue must be a native sub-issue of capability #$capability_issue"
                        fi

                        capability_labels=$(gh issue view "$capability_issue" --repo "$GH_REPOSITORY" --json labels --jq '.labels[].name' 2>/dev/null || true)
                        capability_state=$(gh issue view "$capability_issue" --repo "$GH_REPOSITORY" --json state --jq .state 2>/dev/null || true)
                        if ! printf '%s\n' "$capability_labels" | grep -Fxq kind:capability; then
                            fail "parent issue #$capability_issue must have kind:capability"
                        fi
                        if [ "$capability_state" != OPEN ]; then
                            fail "capability #$capability_issue must be open"
                        fi
                        if ! printf '%s\n' "$capability_labels" | grep -Fxq owner:approved; then
                            fail "capability #$capability_issue requires owner:approved"
                        fi

                        pr_capability_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Parent capability$/{capture=1; next} /^## /{if (capture) exit} capture')
                        pr_capability=$(printf '%s\n' "$pr_capability_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | head -n 1 || true)
                        if [ "$pr_capability" != "$capability_issue" ]; then
                            fail "pull request must link parent capability #$capability_issue"
                        fi
                    fi
                fi
            fi
        fi
    fi

    if { [ -n "${ALPINE_PR_BODY:-}" ] || [ -n "${ALPINE_PR_TITLE:-}" ]; } && printf '%s\n' "$changed_files" | grep -Fxq ARCHITECTURE.md; then
        decision_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Decision or research$/{capture=1; next} /^## /{if (capture) exit} capture')
        decision_issues=$(printf '%s\n' "$decision_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | awk '!seen[$0]++' || true)
        if [ -z "$decision_issues" ]; then
            fail 'architecture changes must link an accepted decision issue'
        elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
            accepted_decision=false
            for decision_issue in $decision_issues; do
                decision_metadata=$(gh issue view "$decision_issue" --repo "$GH_REPOSITORY" --json labels,state,stateReason --jq '[.state, .stateReason, (.labels[].name)] | @tsv' 2>/dev/null || true)
                if printf '%s\n' "$decision_metadata" | grep -Fq CLOSED && printf '%s\n' "$decision_metadata" | grep -Fq COMPLETED && printf '%s\n' "$decision_metadata" | grep -Fq kind:decision; then
                    accepted_decision=true
                fi
            done
            if [ "$accepted_decision" != true ]; then
                fail 'architecture changes require a closed kind:decision issue'
            fi
        fi
    fi
fi

if printf '%s\n' "${ALPINE_PR_LABELS:-}" | tr ',' '\n' | grep -Fxq review:provenance; then
    if [ ! -f provenance.toml ]; then
        fail 'review:provenance requires provenance.toml in the same pull request'
    fi
    research_section=$(printf '%s\n' "${ALPINE_PR_BODY:-}" | awk '/^## Decision or research$/{capture=1; next} /^## /{if (capture) exit} capture')
    research_issues=$(printf '%s\n' "$research_section" | grep -Eo '(#[0-9]+|https://github\.com/[^/]+/[^/]+/issues/[0-9]+)' | sed -E 's#^.*/issues/##; s/^#//' | awk '!seen[$0]++' || true)
    if [ -z "$research_issues" ]; then
        fail 'source-level influence must link its research issue'
    elif [ -n "${GH_REPOSITORY:-}" ] && command -v gh >/dev/null 2>&1; then
        accepted_research=false
        for research_issue in $research_issues; do
            research_metadata=$(gh issue view "$research_issue" --repo "$GH_REPOSITORY" --json labels,state,stateReason --jq '[.state, .stateReason, (.labels[].name)] | @tsv' 2>/dev/null || true)
            if printf '%s\n' "$research_metadata" | grep -Fq CLOSED && printf '%s\n' "$research_metadata" | grep -Fq COMPLETED && printf '%s\n' "$research_metadata" | grep -Fq kind:research; then
                accepted_research=true
            fi
        done
        if [ "$accepted_research" != true ]; then
            fail 'source-level influence requires a closed kind:research issue'
        fi
    fi
fi

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'repository policy checks passed\n'
