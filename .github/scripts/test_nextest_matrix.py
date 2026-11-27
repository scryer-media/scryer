import copy
import unittest
import subprocess
from unittest.mock import patch

from nextest_matrix import feature_args, matrix, runner_matrix, run_group, select_evidence, target_args, verify


def package(name, targets=None, features=None):
    return {"id": name, "name": name, "features": features or {}, "targets": targets or [
        {"name": name.replace("-", "_"), "kind": ["lib"], "test": True},
    ]}


def document(*packages):
    return {"workspace_members": [p["id"] for p in packages], "packages": list(packages)}


class MatrixTests(unittest.TestCase):
    def test_identical_upload_retries_count_once(self):
        receipt = {"lane": {"id": "a"}, "success": True, "run_attempt": 1}
        record = (receipt, {"rust-suites": {}})
        self.assertEqual(select_evidence([record, copy.deepcopy(record)]), [record])

    def test_conflicting_upload_retries_fail(self):
        receipt = {"lane": {"id": "a"}, "success": True}
        with self.assertRaisesRegex(ValueError, "Conflicting retry evidence"):
            select_evidence([(receipt, {"a": 1}), (receipt, {"a": 2})])

    def test_latest_attempt_wins_even_when_failed(self):
        lane = matrix(document(package("a")))["include"][0]
        old = ({"lane": lane, "success": True}, {})
        new = ({"lane": lane, "success": False, "run_attempt": 2}, None)
        for records in ([old, new], [new, old]):
            selected = select_evidence(records)
            self.assertEqual(selected, [new])
            with self.assertRaises(ValueError):
                verify({"include": [lane]}, [receipt for receipt, _ in selected])

    def test_grouping_keeps_every_target_and_caps_runners(self):
        names = ["scryer", "scryer-application", "scryer-plugins",
                 "scryer-infrastructure-runtime", "scryer-infrastructure-library",
                 "scryer-infrastructure-datastore", "scryer-infrastructure-identity",
                 "scryer-interface", "scryer-domain", "scryer-media-types"]
        data = document(*(package(name) for name in names))
        data["packages"][0]["targets"] = [
            {"name": name, "kind": ["test"], "test": True}
            for name in ["integration_graphql", *(f"integration_{i}" for i in range(100))]
        ] + [{"name": "scryer", "kind": ["bin"], "test": True}]
        data["packages"][1]["targets"].append(
            {"name": "compat", "kind": ["test"], "test": True})
        groups = runner_matrix(data)["include"]
        self.assertEqual(len(groups), 15)
        flattened = [lane for group in groups for lane in group["targets"]]
        self.assertEqual(sorted(flattened, key=lambda lane: lane["id"]), matrix(data)["include"])
        self.assertEqual(groups, runner_matrix(data)["include"])

    def test_group_continues_after_a_target_failure(self):
        data = document(package("a"), package("b"))
        with patch("nextest_matrix.metadata", return_value=data), patch(
            "nextest_matrix.run_lane",
            side_effect=[subprocess.CalledProcessError(1, "nextest"), None],
        ) as run:
            with self.assertRaisesRegex(RuntimeError, "Failed targets"):
                run_group("support-media")
            self.assertEqual(run.call_count, 2)

    def test_new_targets_are_automatically_included(self):
        data = document(package("app", [
            {"name": "app", "kind": ["bin"], "test": True},
            {"name": "integration", "kind": ["test"], "test": True},
            {"name": "build-script-build", "kind": ["custom-build"], "test": False},
        ]))
        lanes = matrix(data)["include"]
        self.assertEqual(len(lanes), 2)
        self.assertEqual(target_args(lanes[0]), ["--locked", "-p", "app", "--bin", "app"])
        self.assertEqual(target_args(lanes[1]), ["--locked", "-p", "app", "--test", "integration"])

    def test_exclusions_and_non_members(self):
        data = document(package("app"), package("xtask"), package("xtask-release"),
                        package("xtask-support"), package("xtask-migrations"))
        data["packages"].append(package("external"))
        self.assertEqual([lane["package"] for lane in matrix(data)["include"]], ["app"])

    def test_workspace_dependency_features_are_explicit(self):
        data = document(package("app", features={"runtime": []}),
                        package("storage", features={"default": [], "backups": []}))
        data["packages"][0]["dependencies"] = [
            {"name": "storage", "rename": "store", "path": "crates/storage"},
        ]
        self.assertEqual(feature_args(data, {"package": "app"}),
                         ["--all-features", "--features", "store/backups,store/default"])
        self.assertEqual(feature_args(data, {"package": "storage"}), ["--all-features"])

    def test_unaddressable_transitive_features_fail(self):
        data = document(package("app"), package("middle"),
                        package("storage", features={"backups": []}))
        data["packages"][0]["dependencies"] = [
            {"name": "middle", "rename": None, "path": "crates/middle"},
        ]
        data["packages"][1]["dependencies"] = [
            {"name": "storage", "rename": None, "path": "crates/storage"},
        ]
        with self.assertRaisesRegex(ValueError, "transitive features"):
            feature_args(data, {"package": "app"})

    def test_transitive_test_seam_features_are_allowed(self):
        data = document(package("app"), package("middle"),
                        package("storage", features={"test-support": []}))
        data["packages"][0]["dependencies"] = [
            {"name": "middle", "rename": None, "path": "crates/middle"},
        ]
        data["packages"][1]["dependencies"] = [
            {"name": "storage", "rename": None, "path": "crates/storage"},
            {"name": "storage", "rename": None, "path": "crates/storage",
             "kind": "dev", "features": ["test-support"]},
        ]
        self.assertEqual(feature_args(data, {"package": "app"}), ["--all-features"])
        self.assertEqual(feature_args(data, {"package": "middle"}),
                         ["--all-features", "--features", "storage/test-support"])

    def test_transitive_features_enabled_outside_tests_fail(self):
        for edge in ({"kind": None, "features": ["backups"]}, None):
            data = document(package("app"), package("middle", features={}),
                            package("storage", features={"backups": []}))
            data["packages"][0]["dependencies"] = [
                {"name": "middle", "rename": None, "path": "crates/middle"},
            ]
            data["packages"][1]["dependencies"] = [
                {"name": "storage", "rename": None, "path": "crates/storage"},
                {"name": "storage", "rename": None, "path": "crates/storage",
                 "kind": "dev", "features": ["backups"]},
            ]
            if edge:
                data["packages"][1]["dependencies"].append(
                    {"name": "storage", "rename": None, "path": "crates/storage", **edge})
            else:
                data["packages"][1]["features"] = {"backups": ["storage/backups"]}
            with self.assertRaisesRegex(ValueError, "transitive features"):
                feature_args(data, {"package": "app"})

    def test_duplicates_fail(self):
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            matrix(document(package("app"), package("app")))

    def test_empty_and_oversized_matrix_fail(self):
        for data in (document(), document(*(package(f"p{i}") for i in range(257)))):
            with self.assertRaises(ValueError):
                matrix(data)

    def test_unknown_target_kind_fails(self):
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            matrix(document(package("app", [{"name": "new", "kind": ["unknown"], "test": True}])))

    def test_gate_requires_every_lane_exactly_once(self):
        expected = matrix(document(package("a"), package("b")))
        receipts = [{"lane": lane, "success": True} for lane in expected["include"]]
        verify(expected, receipts)
        for invalid in (receipts[:1], receipts + receipts[:1]):
            with self.assertRaises(ValueError):
                verify(expected, invalid)
        failed = copy.deepcopy(receipts)
        failed[0]["success"] = False
        with self.assertRaises(ValueError):
            verify(expected, failed)
        changed = copy.deepcopy(receipts)
        changed[0]["lane"]["target"] = "wrong"
        with self.assertRaises(ValueError):
            verify(expected, changed)


if __name__ == "__main__":
    unittest.main()
