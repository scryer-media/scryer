"""Coverage for the Scryer CI changed-path classifier."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from classify_scryer_ci_paths import FULL, NONE, WEB, classify_paths, requires_xtask_validation


class ClassifyScryerCiPathsTests(unittest.TestCase):
    def test_documentation_only_changes_need_no_expensive_jobs(self) -> None:
        self.assertEqual(
            classify_paths(["README.md", "docs/getting-started.md", "release-notes/v1.md"]),
            NONE,
        )

    def test_web_only_changes_run_only_web_validation(self) -> None:
        self.assertEqual(
            classify_paths(["apps/scryer-web/src/app.tsx"]),
            WEB,
        )

    def test_web_and_documentation_changes_run_only_web_validation(self) -> None:
        self.assertEqual(
            classify_paths(["apps/scryer-web/package-lock.json", "README.md"]),
            WEB,
        )

    def test_rust_and_build_inputs_need_full_validation(self) -> None:
        for path in [
            "crates/scryer/src/main.rs",
            "Cargo.lock",
            "rust-toolchain.toml",
            "docker/scryer.Dockerfile",
            "packaging/windows/build-msi.ps1",
            ".github/actions/download-builtins/action.yml",
            ".github/workflows/scryer.yml",
        ]:
            with self.subTest(path=path):
                self.assertEqual(classify_paths([path]), FULL)

    def test_mixed_web_and_rust_changes_need_full_validation(self) -> None:
        self.assertEqual(
            classify_paths(["apps/scryer-web/src/app.tsx", "crates/scryer/src/main.rs"]),
            FULL,
        )

    def test_empty_input_fails_closed_to_full_validation(self) -> None:
        self.assertEqual(classify_paths([]), FULL)

    def test_xtask_validation_is_limited_to_tooling_and_shared_inputs(self) -> None:
        for path in [
            "xtask/src/main.rs",
            "xtask-release/src/main.rs",
            "xtask-migrations/src/main.rs",
            "xtask-support/src/lib.rs",
            "xtask-trash-guides/src/lib.rs",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
        ]:
            with self.subTest(path=path):
                self.assertTrue(requires_xtask_validation([path]))

        for path in [
            "README.md",
            "apps/scryer-web/src/app.tsx",
            "crates/scryer/src/main.rs",
            "crates/scryer-application/src/lib.rs",
        ]:
            with self.subTest(path=path):
                self.assertFalse(requires_xtask_validation([path]))


if __name__ == "__main__":
    unittest.main()
