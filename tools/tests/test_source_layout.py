"""Exercise the real audit command against isolated Git working trees."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


AUDITOR = Path(__file__).resolve().parents[1] / "audit_source_layout.py"


class SourceLayoutTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="provlita-layout-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")

    def git(self, *arguments):
        return subprocess.run(["git", "-C", str(self.root), *arguments],
                              capture_output=True, check=True)

    def write(self, relative, lines, trailing_newline=True):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        data = b"x\n" * lines
        if not trailing_newline and data:
            data = data[:-1]
        path.write_bytes(data)
        return path

    def audit(self, root=None):
        return subprocess.run(
            [sys.executable, "-B", str(AUDITOR), "--root", str(root or self.root)],
            capture_output=True, text=True, timeout=10,
        )

    def test_review_and_rejection_boundaries(self):
        for lines, report, failure in ((0, False, False), (799, False, False),
                                       (800, True, False), (1000, True, False),
                                       (1001, True, True)):
            with self.subTest(lines=lines):
                self.write("src/model.rs", lines)
                result = self.audit()
                self.assertEqual(result.returncode, int(failure), result.stderr)
                self.assertEqual("source-lines" in result.stdout, report)
                if failure:
                    self.assertIn('"src/model.rs" has 1001 lines', result.stderr)

    def test_last_line_without_newline_counts(self):
        self.write("src/model.rs", 1001, trailing_newline=False)
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn("has 1001 lines", result.stderr)

    def test_crlf_counts_as_one_line(self):
        path = self.write("src/model.rs", 0)
        path.write_bytes(b"x\r\n" * 1000)
        self.assertEqual(self.audit().returncode, 0)

    def test_tests_are_reported_without_a_hard_length_limit(self):
        for name in ("tests/presentation.rs", "crates/ui/tests/actions.rs",
                     "tools/tests/test_gate.py"):
            self.write(name, 1001)
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.count("test-lines 1001"), 3)

    def test_test_named_files_inside_src_cannot_evade_the_limit(self):
        for name in ("src/tests.rs", "crates/core/src/tests/large.rs"):
            self.write(name, 1001)
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.count("maximum is 1000"), 2)

    def test_root_and_workspace_sources_scripts_and_shaders_are_checked(self):
        for name in ("src/main.rs", "crates/core/src/lib.rs", "build.rs",
                     "examples/panel.rs", "tools/probe.py", "tools/check.sh",
                     "shaders/panel.wgsl"):
            self.write(name, 1001)
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.count("maximum is 1000"), 7)

    def test_tracked_untracked_and_ignored_working_tree_files(self):
        tracked = self.write("src/model.rs", 1)
        self.git("add", "src/model.rs")
        tracked.write_bytes(b"x\n" * 1001)
        self.write("src/new.rs", 1001)
        self.write("scratch/ignored.rs", 1001)
        (self.root / ".gitignore").write_text("scratch/\nsrc/model.rs\n")
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn('"src/model.rs"', result.stderr)
        self.assertIn('"src/new.rs"', result.stderr)
        self.assertNotIn("ignored.rs", result.stdout + result.stderr)

    def test_generated_outputs_vendored_sources_and_docs_are_excluded(self):
        for name in ("vendor/lib/src/lib.rs", "third_party/ui.c", "target/debug/lib.rs",
                     ".artifacts/probe.rs", "ARCHITECTURE.md"):
            self.write(name, 1001)
        self.git("add", ".")
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("files=0", result.stdout)

    def test_deleted_tracked_source_is_not_a_read_failure(self):
        path = self.write("src/removed.rs", 1001)
        self.git("add", ".")
        path.unlink()
        self.assertEqual(self.audit().returncode, 0)

    def test_paths_with_spaces_and_newlines_are_not_split(self):
        self.write("src/a space\nand newline.rs", 1001)
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn('"src/a space\\nand newline.rs"', result.stderr)
        self.assertIn("files=1", result.stdout)

    def test_source_symlink_is_rejected_without_reading_target(self):
        path = self.root / "outside.rs"
        path.symlink_to(self.root / "missing-private-file")
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn("source symlink is not audited", result.stderr)

    def test_non_repository_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="provlita-not-a-repo-") as directory:
            result = self.audit(Path(directory))
        self.assertEqual(result.returncode, 1)
        self.assertIn("could not complete", result.stderr)


if __name__ == "__main__":
    unittest.main()
