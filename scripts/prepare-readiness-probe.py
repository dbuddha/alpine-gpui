#!/usr/bin/env python3
"""Build a fresh, source-identified local probe; never launch or replace an app."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
RA_ARCHIVE_SHA = "ece932daf2f077be87bf745d2eb0a62cbc550f4b1e2e31ca76dfafdd0cc599b3"


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def manifest_string(path, key):
    # Read only the repository-owned, single-line quoted identity fields.
    values = re.findall(r'^' + re.escape(key) + r' = "([^"\n]+)"$', path.read_text(), re.M)
    if len(values) != 1:
        raise SystemExit(f"missing or ambiguous {key} in {path}")
    return values[0]


def verify(root):
    receipt = (root / "identity.json").read_bytes()
    record = json.loads(receipt)
    embedded = Path(record["bundle"]) / "Contents/Resources/probe-identity.json"
    if not embedded.is_file() or embedded.read_bytes() != receipt:
        raise SystemExit("probe embedded identity is missing or differs from the outer receipt")
    for name, digest in record["files"].items():
        path = root / name
        if path.is_symlink() or not path.is_file() or sha(path) != digest:
            raise SystemExit(f"probe identity mismatch: {name}")
    print(f"Verified {len(record['files'])} immutable probe files at source {record['source']}")
    return record


def prepare(output):
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise SystemExit("probe preparation requires Apple Silicon macOS")
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
    if status:
        raise SystemExit("probe preparation requires a clean committed source tree")
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    archive = ROOT / "target/rust-analyzer-qualification/rust-analyzer.gz"
    if not archive.is_file() or sha(archive) != RA_ARCHIVE_SHA:
        raise SystemExit("missing verified rust-analyzer archive; use the pinned CI provisioning command")
    if output.exists() or output.is_symlink():
        raise SystemExit("output already exists; use a new directory and preserve previous runs")
    output.mkdir(parents=True, mode=0o700)
    pristine = output / "initial-workspace"
    pristine.mkdir()
    fixture = ROOT / "apps/alpine-studio/tests/fixtures/rust-analyzer-workspace"
    (pristine / "Cargo.toml").write_text((fixture / "Cargo.toml").read_text().replace("src/lib.rs", "probe.rs"))
    (pristine / "probe.rs").write_text("// Unicode probe: café 🏔️\n" + (fixture / "src/lib.rs").read_text())
    shutil.copytree(pristine, output / "workspace")
    (output / "home").mkdir(mode=0o700)
    build_output = output / "Alpine Studio.app"
    command = [str(ROOT / "scripts/build-alpine-studio-app.sh"), "--output", str(build_output)]
    toolchain = manifest_string(ROOT / "rust-toolchain.toml", "channel")
    build_env = os.environ.copy()
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "ALPINE_BUNDLE_FIXTURE_REVISION"):
        build_env.pop(key, None)
    build_env["RUSTUP_TOOLCHAIN"] = toolchain
    with (output / "build.log").open("w") as log:
        subprocess.run(command, cwd=ROOT, env=build_env, check=True, stdout=log, stderr=subprocess.STDOUT)
    after_source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    after_status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
    original = build_output / "Contents/Resources/alpine-build-identity.toml"
    if after_source != source or after_status or manifest_string(original, "revision") != source:
        raise SystemExit("source changed during probe build; output retained but is not accepted")
    if manifest_string(original, "executable_sha256") != sha(build_output / "Contents/MacOS/alpine-studio"):
        raise SystemExit("built executable differs from the original bundle receipt")
    name = f"Alpine Readiness {source[:8]}"
    bundle = output / f"{name}.app"
    build_output.rename(bundle)
    resources = bundle / "Contents/Resources"
    (resources / "alpine-build-identity.toml").rename(output / "original-build-identity.toml")
    analyzer = resources / "rust-analyzer"
    with gzip.open(archive, "rb") as compressed, analyzer.open("wb") as target:
        shutil.copyfileobj(compressed, target)
    analyzer.chmod(0o755)
    native = bundle / "Contents/MacOS/alpine-studio"
    launcher = bundle / "Contents/MacOS/probe-launcher"
    rustup_home = os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup"))
    cargo_home = os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    toolchain = manifest_string(ROOT / "rust-toolchain.toml", "channel")
    env = {"HOME": str(output / "home"), "PATH": f"{cargo_home}/bin:/usr/bin:/bin:/usr/sbin:/sbin",
           "CARGO_HOME": cargo_home, "RUSTUP_HOME": rustup_home, "RUSTUP_TOOLCHAIN": toolchain,
           "ALPINE_RUST_ANALYZER": str(analyzer), "CARGO_NET_OFFLINE": "true", "LANG": "en_US.UTF-8"}
    cstr = lambda value: json.dumps(str(value), ensure_ascii=False)
    source_text = "#include <unistd.h>\n#include <fcntl.h>\nint main(void) {\n"
    source_text += f"if(chdir({cstr(output / 'workspace')})) return 2;\n"
    source_text += f"int log = open({cstr(output / 'runtime.log')}, O_WRONLY|O_CREAT|O_APPEND, 0600);\n"
    source_text += "if(log < 0 || dup2(log, 1) < 0 || dup2(log, 2) < 0) return 3; if(log > 2) close(log);\n"
    source_text += "char *args[] = {" + cstr(native) + "," + cstr(output / "workspace/probe.rs") + ",0};\n"
    source_text += "char *env[] = {" + ",".join(cstr(f"{key}={value}") for key, value in env.items()) + ",0};\n"
    source_text += "return execve(args[0], args, env); }\n"
    launch_source = output / "launcher.c"
    launch_source.write_text(source_text)
    subprocess.run(["xcrun", "clang", "-Wall", "-Wextra", "-Werror", str(launch_source), "-o", str(launcher)], check=True)
    plist_path = bundle / "Contents/Info.plist"
    plist = plistlib.loads(plist_path.read_bytes())
    bundle_id = f"com.dbuddha.alpine-readiness-{source[:12]}"
    plist.update(CFBundleExecutable="probe-launcher", CFBundleName=name, CFBundleDisplayName=name,
                 CFBundleIdentifier=bundle_id)
    plist_path.write_bytes(plistlib.dumps(plist))
    immutable = [native, launcher, analyzer, plist_path, launch_source, output / "original-build-identity.toml",
                 pristine / "Cargo.toml", pristine / "probe.rs"]
    rustc = subprocess.check_output(["rustc", f"+{toolchain}", "--version", "--verbose"], text=True)
    record = {"rustc": rustc, "schema": "alpine-readiness-probe/v1", "source": source,
              "build_profile": "release", "build_command": command, "bundle": str(bundle),
              "bundle_identifier": bundle_id, "rust_analyzer_archive_sha256": RA_ARCHIVE_SHA,
              "environment": env, "files": {str(p.relative_to(output)): sha(p) for p in immutable},
              "performance_claim": False, "physical_acceptance": "pending"}
    text = json.dumps(record, indent=2, ensure_ascii=False) + "\n"
    (output / "identity.json").write_text(text)
    (resources / "probe-identity.json").write_text(text)
    verify(output)
    print(bundle)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    directory = args.output.absolute()
    if args.verify:
        verify(directory)
    else:
        prepare(directory)
