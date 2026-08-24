#!/bin/sh
set -eu

repo_root=${1:-$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)}
catalog=$repo_root/docs/research/index.md
lineage_root=$repo_root/docs/research/alpine-lineage
lineage_index=$lineage_root/index.md
lineage_methodology=$lineage_root/methodology.md
lineage_sources=$lineage_root/source-map.md
lineage_framework=$lineage_root/framework-lineage.md
lineage_studio=$lineage_root/studio-lineage.md
lineage_evidence=$lineage_root/evidence-ledger.md
lineage_history=$lineage_root/history.md
lineage_review=$lineage_root/adversarial-review.md
lineage_experiments=$lineage_root/experiments.md
lineage_decisions=$lineage_root/alpine-decisions.md
lineage_references=$lineage_root/references.bib
review=$repo_root/docs/research/alpine-studio-adversarial-review.md
comparator=$repo_root/docs/quality/comparator-protocol.md
studio_path=$repo_root/docs/use-cases/alpine-studio-highfidelity.md
zed_editor=$repo_root/docs/case-studies/zed-editor.md
zed_gpui=$repo_root/docs/case-studies/zed-gpui.md
sublime=$repo_root/docs/case-studies/sublime-editor.md
wgpu=$repo_root/docs/case-studies/wgpu.md
wgpu_index=$repo_root/docs/research/wgpu/index.md
wgpu_sources=$repo_root/docs/research/wgpu/source-map.md
wgpu_findings=$repo_root/docs/research/wgpu/findings.md
wgpu_experiments=$repo_root/docs/research/wgpu/experiments.md
wgpu_decisions=$repo_root/docs/research/wgpu/decisions.md
wiki_policy=$repo_root/docs/wiki/README.md
failures=0

fail() {
    printf 'research retention error: %s\n' "$1" >&2
    failures=$((failures + 1))
}

for required in \
    "$catalog" \
    "$lineage_index" \
    "$lineage_methodology" \
    "$lineage_sources" \
    "$lineage_framework" \
    "$lineage_studio" \
    "$lineage_evidence" \
    "$lineage_history" \
    "$lineage_review" \
    "$lineage_experiments" \
    "$lineage_decisions" \
    "$lineage_references" \
    "$review" \
    "$comparator" \
    "$studio_path" \
    "$zed_editor" \
    "$zed_gpui" \
    "$sublime" \
    "$wgpu" \
    "$wgpu_index" \
    "$wgpu_sources" \
    "$wgpu_findings" \
    "$wgpu_experiments" \
    "$wgpu_decisions" \
    "$wiki_policy"
do
    if [ ! -f "$required" ]; then
        fail "required research artifact is missing: ${required#"$repo_root"/}"
    fi
done

if [ "$failures" -ne 0 ]; then
    exit 1
fi

for link in \
    '(alpine-lineage/index.md)' \
    '(alpine-studio-adversarial-review.md)' \
    '(../case-studies/zed-editor.md)' \
    '(../case-studies/zed-gpui.md)' \
    '(../case-studies/sublime-editor.md)' \
    '(../case-studies/wgpu.md)' \
    '(wgpu/index.md)' \
    '(../quality/comparator-protocol.md)' \
    '(../wiki/README.md)' \
    '(../use-cases/alpine-studio-highfidelity.md)'
do
    if ! grep -Fq "$link" "$catalog"; then
        fail "research catalog is missing canonical link $link"
    fi
done

link_errors=$(mktemp)
trap 'rm -f "$link_errors"' EXIT HUP INT TERM
for source in \
    "$catalog" \
    "$lineage_index" \
    "$lineage_methodology" \
    "$lineage_sources" \
    "$lineage_framework" \
    "$lineage_studio" \
    "$lineage_evidence" \
    "$lineage_history" \
    "$lineage_review" \
    "$lineage_experiments" \
    "$lineage_decisions" \
    "$review" \
    "$comparator" \
    "$studio_path" \
    "$zed_editor" \
    "$zed_gpui" \
    "$sublime" \
    "$wgpu" \
    "$wgpu_index" \
    "$wgpu_sources" \
    "$wgpu_findings" \
    "$wgpu_experiments" \
    "$wgpu_decisions" \
    "$wiki_policy"
do
    grep -Eo '\]\([^)]+\)' "$source" 2>/dev/null \
        | sed 's/^](//; s/)$//' \
        | while IFS= read -r link; do
            case "$link" in
                http://*|https://*|mailto:*|'#'*) continue ;;
            esac
            target=${link%%#*}
            target=${target%%\?*}
            if [ -n "$target" ] && [ ! -e "$(dirname "$source")/$target" ]; then
                printf '%s -> %s\n' "${source#"$repo_root"/}" "$link" >> "$link_errors"
            fi
        done
done
if [ -s "$link_errors" ]; then
    fail 'repository-relative research links do not resolve'
    cat "$link_errors" >&2
fi

for requirement in 32 33 34 35 36 37; do
    if ! grep -Fq "https://github.com/dbuddha/alpine-gpui/issues/$requirement" "$review"; then
        fail "adversarial review is missing research anchor for Requirement #$requirement"
    fi
done

for issue in 23 99 113 114 115 116 118 132 174 175 202 315; do
    if ! grep -Fq "https://github.com/dbuddha/alpine-gpui/issues/$issue" "$catalog"; then
        fail "research catalog is missing issue anchor #$issue"
    fi
done

for pin in \
    de8cd6397adc81632fe1103f1834214ae6ec6a1a \
    c98c22f1d3ea0c2deef5c1d082d4518cb5e91ee9 \
    7db5e18f6da8e02cd171668d4714c745c55d7eda \
    e17dc4f9d50db73a458b64dcce50ecd4878b98a3 \
    eb8e1c8b5502b7007465fbbc465f4a736fa39210 \
    40f4a34ebaf56f9a046231f54125ad046239d3f3 \
    657169337a19a5b27f9aa7e53811e6f82b7f213c
do
    if ! grep -Fq "$pin" "$lineage_sources"; then
        fail "lineage source map is missing retained revision pin $pin"
    fi
done

for stale in \
    'one solid-quad trace only' \
    'Open [#219' \
    '#219-#221 remain'
do
    if grep -Fq "$stale" \
        "$lineage_index" \
        "$lineage_studio" \
        "$lineage_evidence"
    then
        fail "lineage package retains superseded current-state claim: $stale"
    fi
done

for required_evidence in \
    'https://github.com/dbuddha/alpine-gpui/pull/344' \
    'https://github.com/dbuddha/alpine-gpui/pull/345' \
    '32762895848'
do
    if ! grep -Fq "$required_evidence" \
        "$lineage_index" \
        "$lineage_studio" \
        "$lineage_evidence" \
        "$lineage_history"
    then
        fail "lineage package is missing post-baseline evidence $required_evidence"
    fi
done

for classification in \
    ADAPTED-CONCEPT \
    INDEPENDENT-CONVERGENCE \
    ALPINE-ORIGINAL \
    COMPARATOR-ONLY \
    REJECTED \
    DEFERRED
do
    if ! grep -Fq "$classification" "$lineage_methodology"; then
        fail "lineage methodology is missing classification $classification"
    fi
done

for mechanism in ALG-001 ALG-016 ALS-001 ALS-011; do
    if ! grep -Fq "$mechanism" "$lineage_evidence"; then
        fail "lineage evidence ledger is missing anchor $mechanism"
    fi
done

for pin in \
    ee5cfb074fd0c4e318b5f8608df504678e4e17ac \
    8ee190c6f151c731a4f8cfd9a102d6ee5903460a
do
    if ! grep -Fq "$pin" "$wgpu_index" || ! grep -Fq "$pin" "$wgpu"; then
        fail "WGPU research is missing retained revision pin $pin"
    fi
done

for heading in \
    '## Primary-source findings' \
    '## Alpine inferences' \
    '## Unverified hypotheses'
do
    if ! grep -Fqx "$heading" "$wgpu_findings"; then
        fail "WGPU findings are missing evidence classification: $heading"
    fi
done

for field in workload_identity_hash environment_hash exclusion_manifest_hash; do
    if ! grep -Fq "$field" "$comparator"; then
        fail "comparator protocol is missing mandatory field $field"
    fi
done

for heading in \
    '## Adaptation separation' \
    '## Explicit exclusion manifest' \
    '## Correctness admission' \
    '## Invalid runs' \
    '## Claim grammar'
do
    if ! grep -Fqx "$heading" "$comparator"; then
        fail "comparator protocol is missing required section: $heading"
    fi
done

if [ "$failures" -ne 0 ]; then
    exit 1
fi

printf 'research retention catalog and evidence chain are valid\n'
