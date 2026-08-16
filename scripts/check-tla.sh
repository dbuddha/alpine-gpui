#!/bin/sh
set -eu

mode=${1:-pull-request}
version=1.7.4
expected_sha=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
tool_dir=target/tla-tool
output_dir=target/tla
jar=$tool_dir/tla2tools-$version.jar

mkdir -p "$tool_dir" "$output_dir"
if [ ! -f "$jar" ]; then
    curl --fail --location --retry 3 \
        --output "$jar" \
        "https://github.com/tlaplus/tlaplus/releases/download/v$version/tla2tools.jar"
fi

actual_sha=$(shasum -a 256 "$jar" | awk '{print $1}')
if [ "$actual_sha" != "$expected_sha" ]; then
    printf 'TLA+ tool checksum mismatch: expected %s, got %s\n' \
        "$expected_sha" "$actual_sha" >&2
    exit 1
fi

case "$mode" in
    pull-request) config=PullRequest.cfg ;;
    nightly) config=Nightly.cfg ;;
    *) printf 'usage: %s [pull-request|nightly]\n' "$0" >&2; exit 2 ;;
esac

for model_dir in formal/tla/aep-*; do
    test -d "$model_dir"
    model=$(find "$model_dir" -maxdepth 1 -name '*.tla' -print | head -n 1)
    model_name=$(basename "$model" .tla)
    model_output=$output_dir/$(basename "$model_dir")
    mkdir -p "$model_output"

    (
        cd "$model_dir"
        java -XX:+UseParallelGC -cp "../../../$jar" tlc2.TLC \
            -cleanup -deadlock -workers auto -config "$config" "$model_name"
    ) >"$model_output/$config.log" 2>&1

    for faulty_path in "$model_dir"/Faulty*.cfg; do
        faulty_config=$(basename "$faulty_path")
        if (
            cd "$model_dir"
            java -XX:+UseParallelGC -cp "../../../$jar" tlc2.TLC \
                -cleanup -deadlock -workers auto -config "$faulty_config" "$model_name"
        ) >"$model_output/$faulty_config.log" 2>&1; then
            printf 'faulty model unexpectedly passed: %s %s\n' \
                "$model_dir" "$faulty_config" >&2
            exit 1
        fi

        if ! grep -Eq 'Invariant .* is violated|Invariant .* is violated\.' \
            "$model_output/$faulty_config.log"; then
            printf 'faulty model did not produce the expected invariant violation: %s %s\n' \
                "$model_dir" "$faulty_config" >&2
            exit 1
        fi
    done
done
