#!/bin/sh
set -eu

model_root=${TLA_MODEL_ROOT:-formal/tla}
manifest=${TLA_CONTROL_MANIFEST:-$model_root/effectiveness-controls.tsv}

fail() {
    printf 'TLA+ effectiveness: %s\n' "$1" >&2
    exit 1
}

check_controls() {
    root=$1
    controls=$2
    test -f "$controls" || fail "missing control manifest $controls"

    temporary=$(mktemp -d)
    trap 'rm -rf "$temporary"' EXIT HUP INT TERM
    find "$root" -mindepth 2 -maxdepth 2 -name 'Faulty*.cfg' -type f \
        | sed "s#^$root/##" | sort > "$temporary/discovered"
    awk -F '\t' '
        /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
        NF != 3 || $1 == "" || $2 == "" || $3 == "" { exit 2 }
        { print $1 "/" $2 }
    ' "$controls" | sort > "$temporary/declared" ||
        fail "manifest rows must contain model, configuration, and invariant"

    if test "$(sort "$temporary/declared" | uniq -d | wc -l | tr -d ' ')" -ne 0; then
        fail "control manifest contains duplicate configurations"
    fi
    if ! cmp -s "$temporary/discovered" "$temporary/declared"; then
        diff -u "$temporary/discovered" "$temporary/declared" >&2 || true
        fail "control manifest does not exactly cover Faulty*.cfg"
    fi
    rm -rf "$temporary"
    trap - EXIT HUP INT TERM
}

extract_metrics() {
    log=$1
    metric=$(grep -Eo '[0-9][0-9,]* states generated, [0-9][0-9,]* distinct states found, [0-9][0-9,]* states left on queue' "$log" | tail -n 1 || true)
    test -n "$metric" || return 1
    normalized=$(printf '%s\n' "$metric" | tr -d ',')
    printf '%s\n' "$normalized" | sed -E 's/^([0-9]+) states generated ([0-9]+) distinct states found ([0-9]+) states left on queue$/\1\t\2\t\3/'
}

if test "${1:-}" = --check-controls; then
    check_controls "${2:-$model_root}" "${3:-$manifest}"
    exit 0
fi

mode=${1:-pull-request}
log_root=${2:-target/tla}
report_root=${3:-$log_root}
case "$mode" in
    pull-request) positive_config=PullRequest.cfg ;;
    nightly) positive_config=Nightly.cfg ;;
    *) fail "mode must be pull-request or nightly" ;;
esac

check_controls "$model_root" "$manifest"
mkdir -p "$report_root"
rows=$report_root/effectiveness.tsv
summary=$report_root/effectiveness.toml
printf 'kind\tmodel\tconfiguration\texpected_invariant\tobserved_invariant\tgenerated_states\tdistinct_states\tremaining_states\tdepth\tmodel_sha256\tconfiguration_sha256\n' > "$rows"

positive_count=0
control_count=0
for model_dir in "$model_root"/aep-*; do
    test -d "$model_dir" || continue
    model_name=$(basename "$model_dir")
    model=$(find "$model_dir" -maxdepth 1 -name '*.tla' -type f | head -n 1)
    test -n "$model" || fail "$model_name has no TLA+ model"
    positive=$log_root/$model_name/$positive_config.log
    test -f "$positive" || fail "missing positive log $positive"
    grep -Fq 'Model checking completed. No error has been found.' "$positive" ||
        fail "$model_name $positive_config lacks successful completion"
    if grep -Eq 'Invariant [[:alnum:]_]+ is violated' "$positive"; then
        fail "$model_name $positive_config contains an invariant violation"
    fi
    metrics=$(extract_metrics "$positive") ||
        fail "$model_name $positive_config lacks state-space metrics"
    generated=$(printf '%s\n' "$metrics" | cut -f1)
    distinct=$(printf '%s\n' "$metrics" | cut -f2)
    remaining=$(printf '%s\n' "$metrics" | cut -f3)
    test "$remaining" -eq 0 || fail "$model_name $positive_config left $remaining states on queue"
    depth=$(grep -Eo 'depth of the complete state graph search is [0-9]+' "$positive" | tail -n 1 | awk '{print $NF}')
    test -n "$depth" || fail "$model_name $positive_config lacks search depth"
    model_sha=$(shasum -a 256 "$model" | awk '{print $1}')
    config_sha=$(shasum -a 256 "$model_dir/$positive_config" | awk '{print $1}')
    printf 'positive\t%s\t%s\t-\t-\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$model_name" "$positive_config" "$generated" "$distinct" "$remaining" "$depth" "$model_sha" "$config_sha" >> "$rows"
    positive_count=$((positive_count + 1))
done

tab=$(printf '\t')
while IFS="$tab" read -r model_name faulty_config expected; do
    case "$model_name" in ''|'#'*) continue ;; esac
    log=$log_root/$model_name/$faulty_config.log
    config_path=$model_root/$model_name/$faulty_config
    model=$(find "$model_root/$model_name" -maxdepth 1 -name '*.tla' -type f | head -n 1)
    test -f "$log" || fail "missing negative-control log $log"
    observed=$(grep -Eo 'Invariant [[:alnum:]_]+ is violated' "$log" | sed -E 's/^Invariant ([[:alnum:]_]+) is violated$/\1/' | sort -u | tr '\n' ' ' | sed 's/ $//')
    test -n "$observed" || fail "$model_name $faulty_config has no invariant violation"
    test "$observed" = "$expected" ||
        fail "$model_name $faulty_config violated '$observed', expected '$expected'"
    metrics=$(extract_metrics "$log" || true)
    if test -n "$metrics"; then
        generated=$(printf '%s\n' "$metrics" | cut -f1)
        distinct=$(printf '%s\n' "$metrics" | cut -f2)
        remaining=$(printf '%s\n' "$metrics" | cut -f3)
    else
        generated=unknown
        distinct=unknown
        remaining=unknown
    fi
    depth=$(grep -Eo 'depth of the complete state graph search is [0-9]+' "$log" | tail -n 1 | awk '{print $NF}' || true)
    depth=${depth:-unknown}
    model_sha=$(shasum -a 256 "$model" | awk '{print $1}')
    config_sha=$(shasum -a 256 "$config_path" | awk '{print $1}')
    printf 'negative_control\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$model_name" "$faulty_config" "$expected" "$observed" "$generated" "$distinct" "$remaining" "$depth" "$model_sha" "$config_sha" >> "$rows"
    control_count=$((control_count + 1))
done < "$manifest"

revision=${REVISION:-$(git rev-parse HEAD 2>/dev/null || printf unknown)}
rows_sha=$(shasum -a 256 "$rows" | awk '{print $1}')
cat > "$summary" <<EOF
schema_version = 1
revision = "$revision"
mode = "$mode"
tool = "TLC"
tool_version = "1.7.4"
positive_models = $positive_count
negative_controls = $control_count
effectiveness_rows_sha256 = "$rows_sha"
EOF
