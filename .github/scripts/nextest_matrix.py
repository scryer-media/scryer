"""Generate independent Cargo target lanes and verify their CI receipts."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


EXCLUDED = {"xtask", "xtask-release", "xtask-migrations", "xtask-support"}
KINDS = {"lib", "bin", "test", "example", "bench", "proc-macro"}


def metadata():
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
    ]))


def matrix(document):
    members = set(document["workspace_members"])
    lanes = []
    for package in document["packages"]:
        if package["id"] not in members or package["name"] in EXCLUDED:
            continue
        for target in package["targets"]:
            if not target["test"]:
                continue
            kinds = set(target["kind"]) & KINDS
            if len(kinds) != 1:
                raise ValueError(f"Unsupported test target: {target}")
            kind = kinds.pop()
            if kind == "proc-macro":
                kind = "lib"
            lanes.append({
                "id": f'{package["name"]}--{kind}--{target["name"]}',
                "package": package["name"], "kind": kind, "target": target["name"],
            })
    lanes.sort(key=lambda lane: lane["id"])
    if not lanes or len(lanes) > 256:
        raise ValueError(f"Expected 1..256 target lanes, got {len(lanes)}")
    if len({lane["id"] for lane in lanes}) != len(lanes):
        raise ValueError("Duplicate target lane")
    return {"include": lanes}


def test_only_features(packages):
    """Features each workspace package has that only dev-dependencies enable.

    Such a feature is a test seam of the crate that turns it on: Cargo never
    applies another crate's dev-dependency features to a downstream build, so
    a lane that cannot address it still builds what its own tests use.
    """
    enabled_by = {}
    for package in packages.values():
        for dep in package.get("dependencies", []):
            if dep["name"] not in packages or not dep.get("path"):
                continue
            kind = dep.get("kind") or "normal"
            for feature in dep.get("features", []):
                enabled_by.setdefault((dep["name"], feature), set()).add(kind)
        # A feature table forwarding `crate/feature` enables it outside tests.
        for values in package["features"].values():
            for value in values:
                if "/" not in value:
                    continue
                dep_name, feature = value.split("/", 1)
                dep_name = dep_name.removeprefix("dep:").rstrip("?")
                target = next(
                    (dep["name"] for dep in package.get("dependencies", [])
                     if (dep.get("rename") or dep["name"]) == dep_name),
                    dep_name,
                )
                enabled_by.setdefault((target, feature), set()).add("normal")
    return {
        name: {
            feature for feature in package["features"]
            if enabled_by.get((name, feature)) == {"dev"}
        }
        for name, package in packages.items()
    }


def feature_args(document, lane):
    # Cargo accepts feature selectors for the selected package and its direct
    # dependencies, not arbitrary unrelated workspace packages.
    members = set(document["workspace_members"])
    packages = {p["name"]: p for p in document["packages"] if p["id"] in members}
    selected = packages[lane["package"]]
    dependencies = {
        dep["rename"] or dep["name"]: packages[dep["name"]]
        for dep in selected.get("dependencies", [])
        if dep["name"] in packages and dep.get("path")
    }
    # Fail closed if a future graph change hides a feature-bearing workspace
    # dependency behind another crate without a directly addressable selector.
    direct = {package["name"] for package in dependencies.values()}
    test_only = test_only_features(packages)
    pending = list(direct)
    visited = set()
    while pending:
        name = pending.pop()
        if name in visited:
            continue
        visited.add(name)
        package = packages[name]
        if name not in direct and any(
            (feature != "default" or values) and feature not in test_only[name]
            for feature, values in package["features"].items()
        ):
            raise ValueError(f"Cannot preserve transitive features of {name} in {lane['package']}")
        pending.extend(
            dep["name"] for dep in package.get("dependencies", [])
            if dep["name"] in packages and dep.get("path") and dep.get("kind") != "dev"
        )
    features = sorted(
        f'{alias}/{feature}'
        for alias, package in dependencies.items()
        for feature in package["features"]
    )
    return ["--all-features"] + (["--features", ",".join(features)] if features else [])


def target_args(lane):
    selector = ["--lib"] if lane["kind"] == "lib" else [f'--{lane["kind"]}', lane["target"]]
    return ["--locked", "-p", lane["package"], *selector]


def runner_matrix(document):
    groups = {}
    integration_index = 0
    for lane in matrix(document)["include"]:
        package = lane["package"]
        if package == "scryer":
            if lane["kind"] == "bin":
                group = "scryer-binaries"
            elif lane["target"] == "integration_graphql":
                group = "scryer-graphql"
            else:
                group = f"scryer-integration-{integration_index % 3 + 1}"
                integration_index += 1
        elif package == "scryer-application":
            group = "application-unit" if lane["kind"] == "lib" else "application-integration"
        elif package == "scryer-plugins":
            group = "plugins"
        elif package == "scryer-infrastructure-runtime":
            group = "infrastructure-runtime"
        elif package == "scryer-infrastructure-library":
            group = "infrastructure-library"
        elif package in {"scryer-infrastructure-datastore", "scryer-infrastructure-sql",
                          "scryer-infrastructure-crypto", "scryer-infrastructure-library-search"}:
            group = "datastore"
        elif package.startswith("scryer-infrastructure-"):
            group = "infrastructure-adapters"
        elif package.startswith("scryer-interface"):
            group = "interfaces"
        elif package in {"scryer-domain", "scryer-release-parser", "scryer-rules"}:
            group = "domain-parsing-rules"
        else:
            group = "support-media"
        groups.setdefault(group, []).append(lane)
    if len(groups) > 15:
        raise ValueError("Nextest runner budget exceeded")
    return {"include": [{"id": key, "targets": targets} for key, targets in sorted(groups.items())]}


def run_lane(document, lane):
    args = target_args(lane) + feature_args(document, lane)
    output = Path("nextest-results") / lane["id"]
    output.mkdir(parents=True, exist_ok=True)
    receipt = {"lane": lane, "args": args, "success": False,
               "run_attempt": int(os.environ.get("GITHUB_RUN_ATTEMPT", "1"))}
    start = time.monotonic()
    try:
        subprocess.run(["cargo", "nextest", "run", *args, "--no-run", "--timings"], check=True)
        receipt["build_seconds"] = round(time.monotonic() - start, 3)
        with (output / "inventory.json").open("w") as handle:
            subprocess.run(["cargo", "nextest", "list", *args, "--message-format", "json"],
                           stdout=handle, check=True)
        test_start = time.monotonic()
        # Empty platform-gated targets remain represented and successfully built.
        subprocess.run(["cargo", "nextest", "run", *args, "--no-fail-fast", "--no-tests", "warn"],
                       check=True)
        receipt["test_seconds"] = round(time.monotonic() - test_start, 3)
        receipt["success"] = True
    finally:
        receipt["elapsed_seconds"] = round(time.monotonic() - start, 3)
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


def run_group(group_id):
    document = metadata()
    group = next(item for item in runner_matrix(document)["include"] if item["id"] == group_id)
    failed = []
    for lane in group["targets"]:
        try:
            run_lane(document, lane)
        except subprocess.CalledProcessError:
            failed.append(lane["id"])
    if failed:
        raise RuntimeError(f"Failed targets: {failed}")


def verify(expected, receipts):
    wanted = {lane["id"]: lane for lane in expected["include"]}
    seen = set()
    for receipt in receipts:
        lane = receipt["lane"]
        key = lane["id"]
        if key in seen or wanted.get(key) != lane or receipt.get("success") is not True:
            raise ValueError(f"Invalid, duplicate, or failed receipt: {key}")
        seen.add(key)
    if seen != set(wanted):
        raise ValueError(f"Missing lanes: {sorted(set(wanted) - seen)}")


def select_evidence(records):
    selected = {}
    for receipt, inventory in records:
        key = receipt["lane"]["id"]
        previous = selected.get(key)
        attempt = receipt.get("run_attempt", 1)
        if previous is None or attempt > previous[0].get("run_attempt", 1):
            selected[key] = (receipt, inventory)
        elif attempt == previous[0].get("run_attempt", 1) and previous != (receipt, inventory):
            raise ValueError(f"Conflicting retry evidence: {key}")
    return list(selected.values())


def summarize(expected, directory):
    records = []
    for path in Path(directory).glob("*/nextest-results/*/receipt.json"):
        inventory_path = path.with_name("inventory.json")
        records.append((json.loads(path.read_text()),
                        json.loads(inventory_path.read_text()) if inventory_path.exists() else None))
    records = select_evidence(records)
    receipts = [receipt for receipt, _ in records]
    verify({"include": [lane for group in expected["include"] for lane in group["targets"]]}, receipts)
    tests = set()
    for receipt, inventory in records:
        if inventory is None:
            raise ValueError("Missing test inventories")
        lane = receipt["lane"]
        for suite_id, suite in inventory["rust-suites"].items():
            if (suite["package-name"], suite["binary-name"], suite["kind"]) != (
                lane["package"], lane["target"], lane["kind"]
            ):
                raise ValueError(f"Unexpected test binary in {lane['id']}: {suite_id}")
            for test in suite["testcases"]:
                identity = (suite_id, test)
                if identity in tests:
                    raise ValueError(f"Duplicate test across lanes: {identity}")
                tests.add(identity)
    lines = [f"Validated {len(receipts)} target lanes; {len(tests)} listed tests.", "",
             "| Target | Build seconds | Test seconds |", "|---|---:|---:|"]
    for receipt in sorted(receipts, key=lambda item: item["elapsed_seconds"], reverse=True):
        lines.append(f'| {receipt["lane"]["id"]} | {receipt["build_seconds"]} | {receipt["test_seconds"]} |')
    report = "\n".join(lines) + "\n"
    print(report)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as handle:
            handle.write(report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["matrix", "run", "verify"])
    parser.add_argument("value", nargs="?")
    args = parser.parse_args()
    if args.command == "matrix":
        document = metadata()
        generated = runner_matrix(document)
        for lane in matrix(document)["include"]:
            feature_args(document, lane)
        print("matrix=" + json.dumps(generated, separators=(",", ":")))
    elif args.command == "run":
        run_group(args.value)
    else:
        summarize(json.loads(os.environ["EXPECTED_MATRIX"]), args.value)


if __name__ == "__main__":
    main()
