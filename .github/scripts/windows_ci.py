"""Native Windows validation; the test lane runs only Windows-gated tests."""

import argparse
import json
from pathlib import Path
import subprocess
import sys


# These tests are gated by cfg(windows), either individually or by their module.
# Keep the interactive Credential Manager round trip ignored as declared in Rust.
WINDOWS_TESTS = {
    "scryer-application": [
        "windows_disk_space_query_smoke_test",
        "source_root_containment_accepts_windows_case_and_separator_variants",
        "current_is_windows_on_windows",
    ],
    "scryer-disk": [
        "non_utf8_windows_paths_round_trip",
        "unix_paths_decode_lossily_on_windows",
        "a_windows_cross_device_move_unlinks_a_read_only_source",
        "a_windows_exclusive_rename_refuses_an_occupied_destination",
        "a_windows_exclusive_rename_moves_to_a_free_name",
        "a_windows_read_only_file_moves_without_a_staging_leftover",
        "windows_names_a_volume_from_a_handle_and_tells_no_files_apart",
    ],
    "scryer-infrastructure-library": [
        "rename_path_key_folds_case_and_separators_on_windows",
        "windows_plain_media_file_path_lookup_normalizes_case_and_separators",
    ],
    "scryer-infrastructure-datastore": [
        "credential_manager_round_trip",
        "desktop_profile_uses_dedicated_credential_namespace",
    ],
    "scryer": [
        "ctrl_w_is_the_letter_w",
        "default_wasmtime_cache_uses_local_app_data",
        "desktop_profile_is_isolated_from_legacy_portable_state",
        "every_menu_command_has_its_own_id",
        "every_tray_callback_event_is_distinct",
        "the_app_window_class_is_not_the_tray_class",
        "the_callback_anchor_is_signed",
        "the_flyout_timers_do_not_collide",
        "tray_mutex_is_global_but_scoped_to_one_windows_user",
    ],
}


def filterset():
    return " | ".join(
        f"(package(={package}) & test(/::({ '|'.join(names) })$/))"
        for package, names in WINDOWS_TESTS.items()
    )


def validate_inventory(inventory):
    expected = {(package, name) for package, names in WINDOWS_TESTS.items() for name in names}
    selected = set()
    for suite in inventory["rust-suites"].values():
        for name, test in suite["testcases"].items():
            if test["filter-match"]["status"] != "matches":
                continue
            identity = (suite["package-name"], name.rsplit("::", 1)[-1])
            if identity in selected or identity not in expected:
                raise ValueError(f"Unexpected or duplicate Windows test: {identity}")
            interactive = identity == ("scryer-infrastructure-datastore", "credential_manager_round_trip")
            if test["ignored"] != interactive:
                raise ValueError(f"Unexpected ignored status for Windows test: {identity}")
            selected.add(identity)
    if selected != expected:
        raise ValueError(f"Missing Windows tests: {sorted(expected - selected)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["clippy", "tests"])
    args = parser.parse_args()
    if sys.platform != "win32":
        raise SystemExit("Windows validation must run natively on Windows")
    target = ["--locked", "--target", "x86_64-pc-windows-msvc"]
    if args.mode == "clippy":
        # The Unix launcher is built and tested by the Linux launcher lanes.
        excluded = [value for package in ["xtask", "xtask-release", "xtask-migrations", "xtask-support",
                                          "scryer-launcher"]
                    for value in ["--exclude", package]]
        subprocess.run(["cargo", "clippy", *target, "--workspace", *excluded,
                        "--all-targets", "--all-features", "--", "-D", "warnings"], check=True)
    else:
        packages = [value for package in WINDOWS_TESTS for value in ["-p", package]]
        selection = [*target, *packages, "--lib", "--bins", "--all-features", "-E", filterset()]
        with Path("windows-test-inventory.json").open("w") as handle:
            subprocess.run(["cargo", "nextest", "list", *selection, "--timings",
                            "--run-ignored", "all", "--message-format", "json"], stdout=handle, check=True)
        validate_inventory(json.loads(Path("windows-test-inventory.json").read_text()))
        subprocess.run(["cargo", "nextest", "run", *selection,
                        "--no-fail-fast", "--no-tests", "fail"], check=True)


if __name__ == "__main__":
    main()
