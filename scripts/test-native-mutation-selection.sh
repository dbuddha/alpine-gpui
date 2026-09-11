#!/bin/sh
# Real Git/classifier/checker fixtures only; no Cargo or native execution.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
python3 - "$script_dir" "$@" <<'PY'
import hashlib, json, os, shutil, subprocess, sys, tempfile, unittest
from pathlib import Path
SCRIPTS = Path(sys.argv[1]).resolve()
ENV = {k: v for k, v in os.environ.items() if not k.startswith(("ALPINE_", "GITHUB_", "GIT_", "NATIVE_")) and k != "METAL_REQUIRED"}
ENV.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
LEAF = "apps/alpine-studio/src/commands.rs"
NATIVE = "crates/alpine-platform-macos/src/native.rs"
def git(root, *args):
    result = subprocess.run(["git", "-c", "user.name=Policy Fixture", "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args], cwd=root, env=ENV, capture_output=True, text=True, timeout=30)
    if result.returncode: raise AssertionError(result.stdout + result.stderr)
    return result.stdout.strip()
class PolicyControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="alpine-native-policy-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.home = Path(cls.temporary.name); cls.template = cls.home / "template"; cls.template.mkdir()
        for name in ("scripts/classify-ci.sh", "scripts/check-native-mutation-selection.sh", ".github/workflows/ci.yml"):
            destination = cls.template / name; destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(SCRIPTS.parent / name, destination)
        if "--remove-event-base" in sys.argv[2:]:
            path = cls.template / "scripts/check-native-mutation-selection.sh"; old = '[ "$base" = "$event_base" ] || fail \'base differs from event base\'\n'
            source = path.read_text(); assert source.count(old) == 1; path.write_text(source.replace(old, "", 1))
        git(cls.template, "init", "-q"); git(cls.template, "add", "."); git(cls.template, "commit", "-qm", "baseline")
        cls.base = git(cls.template, "rev-parse", "HEAD")
    def setUp(self): self.serial = 0
    def fixture(self, paths=(LEAF,), mode="affected", metal="true", labels=""):
        root = Path(tempfile.mkdtemp(dir=self.home)); shutil.copytree(self.template, root, dirs_exist_ok=True)
        for name in paths:
            path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text("fixture change\n")
        git(root, "add", "."); git(root, "commit", "-qm", "source change")
        head = git(root, "rev-parse", "HEAD")
        env = dict(ENV, ALPINE_BASE_SHA=self.base, ALPINE_HEAD_SHA=head, ALPINE_PR_LABELS=labels, GITHUB_SHA=head,
                   ALPINE_EVENT_BASE_SHA=self.base, ALPINE_EVENT_HEAD_SHA=head,
                   NATIVE_SELECTION_MODE=mode, METAL_REQUIRED=metal, NATIVE_MUTATION_REQUIRED=str(metal == "true" and mode == "full").lower())
        return root, env
    def check(self, root, env, accepted=True, reason=""):
        self.serial += 1; witness = root / "target" / ("witness-%s.json" % self.serial)
        env = dict(env, ALPINE_NATIVE_SELECTION_WITNESS=str(witness))
        result = subprocess.run(["sh", "scripts/check-native-mutation-selection.sh"], cwd=root, env=env, capture_output=True, text=True, timeout=30)
        if accepted:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            return json.loads(witness.read_text())
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(reason, result.stderr); self.assertFalse(witness.exists())
    def test_real_classification_boundaries_and_witness(self):
        cases = [((LEAF,), "affected", "true", ""), ((NATIVE,), "full", "true", ""),
                 ((LEAF, "README.md"), "full", "true", ""), (("unmapped.conf",), "full", "true", ""),
                 ((LEAF,), "full", "true", "review:unsafe"), (("README.md",), "full", "false", "")]
        for paths, mode, metal, labels in cases:
            with self.subTest(paths=paths, labels=labels):
                root, env = self.fixture(paths, mode, metal, labels); witness = self.check(root, env)
                self.assertEqual(witness["mode"], mode); self.assertEqual(witness["metal_required"], metal == "true")
                self.assertEqual(witness["native_mutation_required"], mode == "full" and metal == "true")
                self.assertEqual(witness["base"], self.base); self.assertEqual(witness["head"], env["ALPINE_HEAD_SHA"])
                self.assertEqual(witness["tested_commit"], env["GITHUB_SHA"]); self.assertEqual(witness["pr_labels"], labels)
                for key, name in (("classifier_sha256", "scripts/classify-ci.sh"), ("workflow_sha256", ".github/workflows/ci.yml")):
                    self.assertEqual(witness[key], hashlib.sha256((root / name).read_bytes()).hexdigest())
                diff = subprocess.check_output(["git", "diff", "--no-ext-diff", "--no-textconv", "--no-renames", self.base + "..." + env["ALPINE_HEAD_SHA"], "--"], cwd=root, env=ENV)
                self.assertEqual(witness["diff_sha256"], hashlib.sha256(diff).hexdigest())
                self.assertEqual(witness["inventory"], "not-discovered"); self.assertEqual(witness["execution"], "not-evaluated")
    def test_missing_invalid_and_contradictory_outputs(self):
        root, env = self.fixture()
        for key, message in (("NATIVE_SELECTION_MODE", "invalid selection mode"), ("NATIVE_MUTATION_REQUIRED", "invalid mutation requirement"), ("METAL_REQUIRED", "invalid metal requirement")):
            for value in (None, "", "invalid"):
                with self.subTest(key=key, value=value):
                    candidate = dict(env); candidate.pop(key)
                    if value is not None: candidate[key] = value
                    self.check(root, candidate, False, message)
        self.check(root, dict(env, NATIVE_MUTATION_REQUIRED="true"), False, "contradictory mutation requirement")
        self.check(root, dict(env, NATIVE_SELECTION_MODE="full", NATIVE_MUTATION_REQUIRED="true"), False, "classifier contradicts native_selection")
        self.check(root, dict(env, METAL_REQUIRED="false"), False, "classifier contradicts metal")
    def test_fixture_injection_and_output_redirection(self):
        root, env = self.fixture()
        for value in ("", LEAF): self.check(root, dict(env, ALPINE_CHANGED_FILES=value), False, "fixture injection")
        output = root / "github-output"; plan = root / "unexpected-ci-plan"
        self.check(root, dict(env, GITHUB_OUTPUT=str(output), ALPINE_CI_PLAN=str(plan)))
        self.assertFalse(output.exists()); self.assertFalse(plan.exists())
    def test_wrong_checkout_and_policy_source_drift(self):
        root, env = self.fixture()
        for key in ("ALPINE_EVENT_BASE_SHA", "ALPINE_EVENT_HEAD_SHA"):
            candidate = dict(env); candidate.pop(key); self.check(root, candidate, False, 'invalid commit SHA')
            self.check(root, dict(env, **{key: "bad"}), False, 'invalid commit SHA')
        self.check(root, dict(env, GITHUB_SHA=self.base), False, 'checkout differs from GITHUB_SHA')
        self.check(root, dict(env, ALPINE_HEAD_SHA=self.base), False, 'head differs from event head')
        self.check(root, dict(env, ALPINE_BASE_SHA="bad"), False, 'invalid commit SHA')
        for name in ("scripts/classify-ci.sh", ".github/workflows/ci.yml"):
            root, env = self.fixture(); path = root / name
            with path.open("a") as stream: stream.write("\n# uncommitted fixture change\n")
            self.check(root, env, False, 'policy source differs from tested commit')
    def test_exact_merge_parent_relationship(self):
        root, env = self.fixture(); head = env["ALPINE_HEAD_SHA"]
        git(root, "checkout", "-q", "-b", "base-side", self.base)
        (root / "README.md").write_text("base-side change\n")
        git(root, "add", "."); git(root, "commit", "-qm", "advance base"); base = git(root, "rev-parse", "HEAD")
        git(root, "merge", "--no-ff", "-qm", "test merge", head)
        env.update(ALPINE_BASE_SHA=base, ALPINE_EVENT_BASE_SHA=base, GITHUB_SHA=git(root, "rev-parse", "HEAD"))
        self.check(root, env)
        self.check(root, dict(env, ALPINE_BASE_SHA=self.base, ALPINE_EVENT_BASE_SHA=self.base), False, 'checkout is not head or exact base/head merge')
    def test_push_event_base_cannot_omit_native_change(self):
        root, env = self.fixture((NATIVE,), "full"); native = env["ALPINE_HEAD_SHA"]
        path = root / LEAF; path.parent.mkdir(parents=True, exist_ok=True); path.write_text("later leaf change\n")
        git(root, "add", "."); git(root, "commit", "-qm", "later leaf change")
        leaf = git(root, "rev-parse", "HEAD")
        env.update(ALPINE_HEAD_SHA=leaf, ALPINE_EVENT_HEAD_SHA=leaf, GITHUB_SHA=leaf)
        witness = self.check(root, env); self.assertEqual(witness["mode"], "full")
        self.check(root, dict(env, ALPINE_BASE_SHA=native, NATIVE_SELECTION_MODE="affected", NATIVE_MUTATION_REQUIRED="false"), False, 'base differs from event base')
unittest.main(argv=[sys.argv[0]] + (["PolicyControls.test_push_event_base_cannot_omit_native_change"] if "--remove-event-base" in sys.argv[2:] else []), verbosity=2)
PY
