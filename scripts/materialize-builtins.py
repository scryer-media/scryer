#!/usr/bin/env python3
"""Fetch and verify the version-pinned Scryer built-in plugin artifacts.

This tool intentionally has no Python package dependencies. CI installs cosign,
b3sum, and zstd explicitly, which keeps the provenance path independent of
Cargo and makes every downloaded input visible in the workflow log.
"""

from __future__ import annotations

import argparse
import atexit
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from urllib.parse import urlparse
from pathlib import Path
from typing import Any


CATALOG_REDIRECT_URL = "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.json"
CATALOG_REDIRECT_BUNDLE_URL = "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.bundle.json"
OFFICIAL_PLUGIN_REPOSITORY = "scryer-media/scryer-plugins"
OFFICIAL_PLUGIN_WORKFLOW = ".github/workflows/release-plugin-v3.yml"
OFFICIAL_PLUGIN_WORKFLOW_NAME = "release-plugin-v3"
OIDC_ISSUER = "https://token.actions.githubusercontent.com"
SIGSTORE_TUF_SOURCE = "https://tuf-repo-cdn.sigstore.dev"
SIGSTORE_TUF_TARGET = "trusted_root.json"
SIGSTORE_TRUST_ROOT_MEDIA_TYPE = (
    "application/vnd.dev.sigstore.trustedroot+json;version=0.1"
)
SIGSTORE_TRUST_ROOT_FILENAME = "sigstore-trusted-root.json"
SIGSTORE_TRUST_ROOT_PROVENANCE_FILENAME = (
    "sigstore-trusted-root.provenance.json"
)
SIGSTORE_TRUST_ROOT_TIMEOUT = "120s"
COSIGN = os.environ.get("SCRYER_COSIGN", "cosign")
BUILTINS = {
    "newznab": "newznab_indexer",
    "torznab": "torznab_indexer",
}
CORE_WASM_V1_HEADER = b"\x00asm\x01\x00\x00\x00"
WASM_COMPONENT_V1_HEADER = b"\x00asm\x0d\x00\x01\x00"


def fail(message: str) -> None:
    raise RuntimeError(message)


def download(url: str, destination: Path) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "scryer-builtins/1"})
    with urllib.request.urlopen(request, timeout=120) as response:
        payload = response.read()
    destination.write_bytes(payload)
    return payload


def run(*args: str, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        args,
        cwd=cwd,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if completed.returncode:
        fail(f"command failed ({' '.join(args)}):\n{completed.stdout}")
    return completed.stdout


def zstd_decode(source: Path, destination: Path) -> bytes:
    with destination.open("wb") as output:
        completed = subprocess.run(
            ["zstd", "--quiet", "--decompress", "--stdout", str(source)],
            check=False,
            stdout=output,
            stderr=subprocess.PIPE,
            text=True,
        )
    if completed.returncode:
        fail(f"failed to decompress {source}: {completed.stderr}")
    return destination.read_bytes()


def decoded_bundle(path: Path, temp_dir: Path) -> Path:
    if path.name.endswith(".zst"):
        output = temp_dir / f"{path.name}.decoded"
        zstd_decode(path, output)
        return output
    return path


def verify_sigstore(
    blob: Path, bundle: Path, trust_root: Path, temp_dir: Path
) -> None:
    bundle = decoded_bundle(bundle, temp_dir)
    run(
        COSIGN,
        "--timeout",
        SIGSTORE_TRUST_ROOT_TIMEOUT,
        "verify-blob",
        "--bundle",
        str(bundle),
        "--trusted-root",
        str(trust_root),
        "--certificate-identity-regexp",
        ".*",
        "--certificate-oidc-issuer",
        OIDC_ISSUER,
        "--certificate-github-workflow-repository",
        OFFICIAL_PLUGIN_REPOSITORY,
        "--certificate-github-workflow-name",
        OFFICIAL_PLUGIN_WORKFLOW_NAME,
        str(blob),
    )


def cosign_version() -> str:
    try:
        version = json.loads(run(COSIGN, "version", "--json"))
    except json.JSONDecodeError as error:
        fail(f"cosign returned malformed version metadata: {error}")
    git_version = version.get("gitVersion")
    if not isinstance(git_version, str) or not git_version.strip():
        fail("cosign version metadata is missing gitVersion")
    return f"cosign {git_version.strip()}"


def current_head_commit(repo_root: Path) -> str:
    commit = os.environ.get("GITHUB_SHA") or run(
        "git", "rev-parse", "HEAD", cwd=repo_root
    ).strip()
    if not re.fullmatch(r"[0-9a-fA-F]{40,64}", commit):
        fail("source commit is not a valid Git object ID")
    return commit.lower()


def decode_trust_root_base64(value: Any, label: str) -> bytes:
    if not isinstance(value, str) or not value:
        fail(f"Sigstore {label} must be non-empty base64")
    try:
        return base64.b64decode(value, validate=True)
    except ValueError as error:
        fail(f"Sigstore {label} is invalid base64: {error}")
    raise AssertionError("unreachable")


def validate_trust_time_range(value: Any, label: str) -> None:
    if not isinstance(value, dict) or not isinstance(value.get("start"), str):
        fail(f"Sigstore {label} has no validity start")
    end_raw = value.get("end")
    if end_raw is not None and not isinstance(end_raw, str):
        fail(f"Sigstore {label} validity end must be an RFC 3339 timestamp")
    try:
        start_raw = value["start"]
        if "T" not in start_raw or (end_raw is not None and "T" not in end_raw):
            raise ValueError("timestamp must contain a date/time separator")
        start = datetime.fromisoformat(start_raw.replace("Z", "+00:00"))
        end = (
            datetime.fromisoformat(end_raw.replace("Z", "+00:00"))
            if isinstance(end_raw, str)
            else None
        )
    except ValueError as error:
        fail(f"Sigstore {label} has an invalid validity timestamp: {error}")
    if start.utcoffset() is None or (end is not None and end.utcoffset() is None):
        fail(f"Sigstore {label} validity timestamp must include a UTC offset")
    if end is not None and end < start:
        fail(f"Sigstore {label} validity ends before it starts")


def validate_der_with_openssl(
    der: bytes, kind: str, label: str, temp_dir: Path, index: int
) -> None:
    path = temp_dir / f"trust-material-{index}.der"
    path.write_bytes(der)
    try:
        if kind == "public-key":
            run(
                "openssl",
                "pkey",
                "-pubin",
                "-inform",
                "DER",
                "-in",
                str(path),
                "-noout",
            )
        elif kind == "certificate":
            run(
                "openssl",
                "x509",
                "-inform",
                "DER",
                "-in",
                str(path),
                "-noout",
            )
        else:
            raise AssertionError(f"unsupported DER validation kind: {kind}")
    except RuntimeError as error:
        fail(f"failed to parse Sigstore {label}: {error}")
    finally:
        path.unlink(missing_ok=True)


def validate_trusted_logs(
    root: dict[str, Any], field: str, label: str, temp_dir: Path, index: int
) -> int:
    logs = root.get(field)
    if not isinstance(logs, list) or not logs:
        fail(f"Sigstore trusted root has no {field}")
    key_ids: set[bytes] = set()
    for log in logs:
        if not isinstance(log, dict):
            fail(f"Sigstore {label} log must be an object")
        log_id = log.get("logId")
        public_key = log.get("publicKey")
        if not isinstance(log_id, dict) or not isinstance(public_key, dict):
            fail(f"Sigstore {label} log is missing its ID or public key")
        key_id = decode_trust_root_base64(log_id.get("keyId"), f"{label} log ID")
        if not key_id:
            fail(f"Sigstore {label} log ID is empty")
        if key_id in key_ids:
            fail(f"Sigstore trusted root contains a duplicate {label} log ID")
        key_ids.add(key_id)
        key_der = decode_trust_root_base64(
            public_key.get("rawBytes"), f"{label} public key"
        )
        validate_der_with_openssl(
            key_der, "public-key", f"{label} public key", temp_dir, index
        )
        index += 1
        validate_trust_time_range(
            public_key.get("validFor"), f"{label} public key"
        )
    return index


def validate_certificate_authorities(
    root: dict[str, Any], field: str, label: str, temp_dir: Path, index: int
) -> int:
    authorities = root.get(field)
    if not isinstance(authorities, list) or not authorities:
        fail(f"Sigstore trusted root has no {field}")
    for authority_index, authority in enumerate(authorities):
        if not isinstance(authority, dict):
            fail(f"Sigstore {label} authority must be an object")
        validate_trust_time_range(
            authority.get("validFor"),
            f"{label} authority {authority_index}",
        )
        chain = authority.get("certChain")
        certificates = chain.get("certificates") if isinstance(chain, dict) else None
        if not isinstance(certificates, list) or not certificates:
            fail(
                f"Sigstore {label} authority {authority_index} has an empty certificate chain"
            )
        for certificate_index, certificate in enumerate(certificates):
            if not isinstance(certificate, dict):
                fail(
                    f"Sigstore {label} authority {authority_index} certificate "
                    f"{certificate_index} must be an object"
                )
            certificate_der = decode_trust_root_base64(
                certificate.get("rawBytes"),
                f"{label} authority {authority_index} certificate {certificate_index}",
            )
            validate_der_with_openssl(
                certificate_der,
                "certificate",
                f"{label} authority {authority_index} certificate {certificate_index}",
                temp_dir,
                index,
            )
            index += 1
    return index


def validate_runtime_trust_root(root: dict[str, Any]) -> None:
    if root.get("mediaType") != SIGSTORE_TRUST_ROOT_MEDIA_TYPE:
        fail("Sigstore trusted root has an unsupported media type")
    with tempfile.TemporaryDirectory(prefix="scryer-trust-validation-") as temporary:
        temp_dir = Path(temporary)
        index = validate_trusted_logs(root, "tlogs", "Rekor", temp_dir, 0)
        index = validate_trusted_logs(root, "ctlogs", "CT", temp_dir, index)
        index = validate_certificate_authorities(
            root,
            "certificateAuthorities",
            "Fulcio",
            temp_dir,
            index,
        )
        validate_certificate_authorities(
            root,
            "timestampAuthorities",
            "timestamp-authority",
            temp_dir,
            index,
        )


def materialize_sigstore_trust_root(
    repo_root: Path, output_dir: Path
) -> Path:
    trust_root_path = output_dir / SIGSTORE_TRUST_ROOT_FILENAME
    run(
        COSIGN,
        "--timeout",
        SIGSTORE_TRUST_ROOT_TIMEOUT,
        "initialize",
    )
    verified_target = Path.home() / ".sigstore/root/targets" / SIGSTORE_TUF_TARGET
    if not verified_target.is_file():
        fail(
            "cosign initialize did not materialize the TUF-verified "
            f"{SIGSTORE_TUF_TARGET} target"
        )
    shutil.copyfile(verified_target, trust_root_path)
    root_bytes = trust_root_path.read_bytes()
    try:
        root = json.loads(root_bytes)
    except json.JSONDecodeError as error:
        fail(f"Sigstore trusted root is invalid JSON: {error}")
    if not isinstance(root, dict):
        fail("Sigstore trusted root must be a JSON object")
    validate_runtime_trust_root(root)

    sha256 = hashlib.sha256(root_bytes).hexdigest()
    receipt: dict[str, Any] = {
        "schemaVersion": 1,
        "source": SIGSTORE_TUF_SOURCE,
        "target": SIGSTORE_TUF_TARGET,
        "sha256": sha256,
        "retrievedAt": datetime.now(timezone.utc).isoformat(),
        "sigstoreVersion": cosign_version(),
        "sourceCommit": current_head_commit(repo_root),
    }
    for field, environment_name in (
        ("githubRepository", "GITHUB_REPOSITORY"),
        ("githubWorkflowRef", "GITHUB_WORKFLOW_REF"),
        ("githubRunId", "GITHUB_RUN_ID"),
    ):
        value = os.environ.get(environment_name)
        if value:
            receipt[field] = value

    receipt_path = output_dir / SIGSTORE_TRUST_ROOT_PROVENANCE_FILENAME
    receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    parsed_receipt = json.loads(receipt_path.read_text())
    if parsed_receipt.get("sha256") != hashlib.sha256(
        trust_root_path.read_bytes()
    ).hexdigest():
        fail("Sigstore trust-root provenance digest mismatch")
    return trust_root_path


def blake3(path: Path) -> str:
    return run("b3sum", "--no-names", str(path)).strip().lower()


def required_blake3(label: str, digests: list[str]) -> str:
    for digest in digests:
        if digest.lower().startswith("blake3:"):
            return digest.split(":", 1)[1].lower()
    fail(f"{label} does not declare a blake3 digest")
    raise AssertionError("unreachable")


def assert_blake3(label: str, path: Path, digests: list[str]) -> str:
    expected = required_blake3(label, digests)
    actual = blake3(path)
    if actual != expected:
        fail(f"{label} digest mismatch: expected {expected}, got {actual}")
    return actual


def read_leb(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        if offset >= len(data) or shift > 35:
            fail("invalid WASM LEB128 value")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            return value, offset
        shift += 7


def embedded_descriptor(wasm: bytes) -> dict[str, Any]:
    if wasm[:8] not in {CORE_WASM_V1_HEADER, WASM_COMPONENT_V1_HEADER}:
        fail("downloaded builtin is not a WebAssembly v1 module or component")
    offset = 8
    descriptor: bytes | None = None
    while offset < len(wasm):
        section_id = wasm[offset]
        offset += 1
        section_size, offset = read_leb(wasm, offset)
        section_end = offset + section_size
        if section_end > len(wasm):
            fail("WASM section exceeds module length")
        if section_id == 0:
            name_len, name_offset = read_leb(wasm, offset)
            name_end = name_offset + name_len
            if name_end > section_end:
                fail("WASM custom section name exceeds section length")
            name = wasm[name_offset:name_end].decode("utf-8")
            if name == "scryer.plugin-descriptor.v1":
                if descriptor is not None:
                    fail("WASM contains duplicate built-in descriptor sections")
                descriptor = wasm[name_end:section_end]
        offset = section_end
    if descriptor is None:
        fail("WASM does not contain the built-in descriptor section")
    try:
        parsed = json.loads(descriptor)
    except json.JSONDecodeError as error:
        fail(f"embedded descriptor is invalid JSON: {error}")
    if not isinstance(parsed, dict):
        fail("embedded descriptor must be a JSON object")
    return parsed


def parse_version(value: str) -> tuple[int, int, int]:
    match = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value.strip())
    if not match:
        fail(f"invalid semantic version: {value}")
    return tuple(int(part) for part in match.groups())


def version_is(value: str, wanted: str) -> bool:
    return value.strip().lstrip("v") == wanted.strip().lstrip("v")


def version_matches_constraint(
    version: tuple[int, int, int], constraint: str
) -> bool:
    """Evaluate the catalog's comma-separated comparison constraints."""
    for clause in constraint.split(","):
        match = re.fullmatch(r"\s*(>=|<=|>|<|=)\s*v?(\d+\.\d+\.\d+)\s*", clause)
        if not match:
            fail(f"unsupported semantic version constraint: {constraint}")
        operator, raw_expected = match.groups()
        expected = parse_version(raw_expected)
        if operator == ">=" and version < expected:
            return False
        if operator == "<=" and version > expected:
            return False
        if operator == ">" and version <= expected:
            return False
        if operator == "<" and version >= expected:
            return False
        if operator == "=" and version != expected:
            return False
    return True


def current_scryer_version(repo_root: Path) -> tuple[int, int, int]:
    contents = (repo_root / "crates/scryer/Cargo.toml").read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', contents, flags=re.MULTILINE)
    if not match:
        fail("could not determine the host Scryer version from crates/scryer/Cargo.toml")
    return parse_version(match.group(1))


def current_sdk_version(repo_root: Path) -> tuple[int, int, int]:
    contents = (repo_root / "crates/scryer-plugin-sdk/src/lib.rs").read_text()
    match = re.search(r'pub const SDK_VERSION: &str = "([^"]+)";', contents)
    if not match:
        fail("could not determine the host SDK version from scryer-plugin-sdk")
    return parse_version(match.group(1))


def legacy_sdk_constraint(sdk_version: tuple[int, int, int]) -> str:
    major, minor, _ = sdk_version
    lower_major, lower_minor = (1, 0) if major == 1 else (major, minor)
    upper_major = 3 if major == 1 else major + 1
    return f">={lower_major}.{lower_minor}.0, <{upper_major}.0.0"


def release_is_compatible(
    plugin_id: str,
    release: dict[str, Any],
    scryer_version: tuple[int, int, int],
    sdk_version: tuple[int, int, int],
) -> bool:
    sdk_constraint = str(release.get("sdk_constraint", "")).strip()
    if not sdk_constraint:
        fail(f"{plugin_id} {release.get('version')} has no sdk_constraint")
    min_scryer = str(release.get("min_scryer_version", "")).strip()
    if not min_scryer:
        fail(f"{plugin_id} {release.get('version')} has no min_scryer_version")
    if not version_matches_constraint(sdk_version, sdk_constraint):
        return False
    if scryer_version < parse_version(min_scryer):
        return False
    max_scryer = str(release.get("max_scryer_version", "")).strip()
    return not max_scryer or scryer_version <= parse_version(max_scryer)


def copy_bundle(bundle: Path, destination: Path) -> str:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(bundle, destination)
    return hashlib.sha256(destination.read_bytes()).hexdigest()


def bundle_temp_path(temp_dir: Path, url: str, fallback: str) -> Path:
    name = Path(urlparse(url).path).name
    return temp_dir / (name or fallback)


def publish_directory(staging_dir: Path, output_dir: Path) -> None:
    backup_dir = output_dir.parent / f".{output_dir.name}.previous-{os.getpid()}"
    if backup_dir.exists():
        fail(f"refusing to overwrite stale materialization backup {backup_dir}")
    had_previous = output_dir.exists()
    if had_previous:
        os.replace(output_dir, backup_dir)
    try:
        os.replace(staging_dir, output_dir)
    except OSError:
        if had_previous:
            os.replace(backup_dir, output_dir)
        raise
    if had_previous:
        shutil.rmtree(backup_dir)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path)
    parser.add_argument("--provenance", type=Path)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    manifest_path = repo_root / "crates/scryer-plugins/builtin-versions.json"
    final_output_dir = (
        args.output or repo_root / "crates/scryer-plugins/builtins"
    ).resolve()
    final_provenance_path = (
        args.provenance or final_output_dir / "provenance.json"
    ).resolve()
    try:
        provenance_relative_path = final_provenance_path.relative_to(final_output_dir)
    except ValueError:
        fail("--provenance must be located within --output")
    final_output_dir.parent.mkdir(parents=True, exist_ok=True)
    staging_parent = Path(
        tempfile.mkdtemp(
            prefix=".builtins-materializing-", dir=final_output_dir.parent
        )
    )
    atexit.register(shutil.rmtree, staging_parent, ignore_errors=True)
    output_dir = staging_parent / "builtins"
    provenance_path = output_dir / provenance_relative_path
    output_dir.mkdir(parents=True)

    scryer_version = current_scryer_version(repo_root)
    sdk_version = current_sdk_version(repo_root)
    manifest = json.loads(manifest_path.read_text())
    if manifest.get("schemaVersion") != 2 or not isinstance(
        manifest.get("plugins"), dict
    ):
        fail(
            f"invalid built-in version manifest: {manifest_path} "
            "(schemaVersion 2 pins version and wasm_blake3; "
            "run `cargo xtask builtins sync`)"
        )
    pins = manifest["plugins"]
    if set(pins) != set(BUILTINS):
        fail("built-in version manifest must select exactly the supported built-ins")
    for plugin_id, pin in pins.items():
        if not isinstance(pin, dict) or not isinstance(pin.get("version"), str):
            fail(f"built-in pin for {plugin_id} must be an object with a version string")
        try:
            parse_version(pin["version"])
        except RuntimeError as error:
            fail(f"invalid selected version for {plugin_id}: {error}")
        digest = pin.get("wasm_blake3")
        if not isinstance(digest, str) or len(digest) != 64 or any(
            ch not in "0123456789abcdef" for ch in digest
        ):
            fail(
                f"built-in pin for {plugin_id} must carry wasm_blake3 as 64 "
                "lowercase hex characters (run `cargo xtask builtins sync` to refresh it)"
            )
    selected = {plugin_id: pin["version"] for plugin_id, pin in pins.items()}

    bundle_dir = output_dir / "provenance"
    trust_root_path = materialize_sigstore_trust_root(repo_root, output_dir)
    with tempfile.TemporaryDirectory(prefix="scryer-builtins-") as temporary:
        temp_dir = Path(temporary)
        redirect_path = temp_dir / "catalog.redirect.json"
        redirect_bundle_path = temp_dir / "catalog.redirect.bundle.json"
        download(CATALOG_REDIRECT_URL, redirect_path)
        download(CATALOG_REDIRECT_BUNDLE_URL, redirect_bundle_path)
        verify_sigstore(
            redirect_path, redirect_bundle_path, trust_root_path, temp_dir
        )
        redirect_bundle_sha256 = copy_bundle(
            redirect_bundle_path,
            bundle_dir / "catalog-v3.redirect.bundle.json",
        )
        redirect = json.loads(redirect_path.read_text())
        artifacts = redirect.get("artifacts") or []
        if not artifacts:
            fail("catalog redirect has no artifacts")
        catalog_artifact = artifacts[-1]
        catalog_path = temp_dir / "catalog.bundle"
        catalog_signature_path = bundle_temp_path(
            temp_dir,
            catalog_artifact["signature_url"],
            "catalog.bundle.sigstore",
        )
        download(catalog_artifact["url"], catalog_path)
        download(catalog_artifact["signature_url"], catalog_signature_path)
        verify_sigstore(
            catalog_path, catalog_signature_path, trust_root_path, temp_dir
        )
        catalog_bundle_sha256 = copy_bundle(
            catalog_signature_path,
            bundle_dir / "catalog-v3.bundle.sigstore",
        )
        if catalog_artifact["url"].endswith(".zst"):
            catalog_bytes = zstd_decode(catalog_path, temp_dir / "catalog.json")
        else:
            catalog_bytes = catalog_path.read_bytes()
        catalog = json.loads(catalog_bytes)
        catalog_plugins = {
            entry["id"]: entry for entry in catalog.get("plugins", [])
        }
        provenance_plugins: dict[str, Any] = {}

        for plugin_id, stem in BUILTINS.items():
            entry = catalog_plugins.get(plugin_id)
            if entry is None:
                fail(f"catalog does not contain built-in plugin {plugin_id}")
            signer = entry.get("required_signer") or {}
            if (
                signer.get("github_repository") != OFFICIAL_PLUGIN_REPOSITORY
                or signer.get("github_workflow") != OFFICIAL_PLUGIN_WORKFLOW
            ):
                fail(f"{plugin_id} has an unexpected required signer")
            wanted_version = str(selected[plugin_id])
            release = next(
                (
                    item
                    for item in entry.get("releases", [])
                    if version_is(str(item.get("version", "")), wanted_version)
                ),
                None,
            )
            if release is None:
                fail(
                    f"catalog does not contain requested {plugin_id} "
                    f"version {wanted_version}"
                )
            if not release_is_compatible(
                plugin_id, release, scryer_version, sdk_version
            ):
                fail(
                    f"requested {plugin_id} version {wanted_version} is not "
                    f"compatible with Scryer {'.'.join(map(str, scryer_version))} "
                    f"and SDK {'.'.join(map(str, sdk_version))}"
                )
            artifact = next(
                (
                    item
                    for runtime in ("wasm32-wasip2", "wasm32-wasip1")
                    for item in release.get("artifacts", [])
                    if item.get("runtime") == runtime
                    and not item.get("required_features")
                    and str(item.get("url", "")).endswith(".wasm.zst")
                ),
                None,
            )
            if artifact is None:
                fail(f"{plugin_id} {wanted_version} has no baseline WASM artifact")
            compressed = temp_dir / f"{stem}.wasm.zst"
            signature = bundle_temp_path(
                temp_dir,
                artifact["signature_url"],
                f"{stem}.wasm.zst.sigstore",
            )
            download(artifact["url"], compressed)
            download(artifact["signature_url"], signature)
            verify_sigstore(compressed, signature, trust_root_path, temp_dir)
            signature_sha256 = copy_bundle(
                signature, bundle_dir / f"{stem}.wasm.zst.sigstore"
            )
            compressed_digest = assert_blake3(
                f"{plugin_id} compressed artifact",
                compressed,
                artifact.get("digests") or [],
            )
            wasm_path = temp_dir / f"{stem}.wasm"
            wasm = zstd_decode(compressed, wasm_path)
            wasm_digest = assert_blake3(
                f"{plugin_id} WASM artifact",
                wasm_path,
                artifact.get("wasm_digests") or [],
            )
            pinned_wasm = pins[plugin_id]["wasm_blake3"]
            if wasm_digest.lower() != pinned_wasm:
                fail(
                    f"{plugin_id} {wanted_version} WASM digest does not match "
                    "the pinned wasm_blake3 in builtin-versions.json "
                    f"(pinned {pinned_wasm}, catalog {wasm_digest}); if this "
                    "change is intended, refresh the pin with "
                    "`cargo xtask builtins sync`"
                )
            descriptor = embedded_descriptor(wasm)
            if (
                descriptor.get("id") != plugin_id
                or str(descriptor.get("version")) != wanted_version
            ):
                fail(
                    f"{plugin_id} descriptor does not match the requested catalog release"
                )
            descriptor["sdk_version"] = ".".join(map(str, sdk_version))
            descriptor["sdk_constraint"] = legacy_sdk_constraint(sdk_version)
            destination = output_dir / f"{stem}.wasm.zst"
            shutil.copyfile(compressed, destination)
            descriptor_path = output_dir / f"{stem}.descriptor.json"
            descriptor_path.write_text(
                json.dumps(descriptor, indent=2, sort_keys=True) + "\n"
            )
            (output_dir / f"{stem}.description.txt").write_text(
                str(entry.get("description", "")).strip() + "\n"
            )
            provenance_plugins[plugin_id] = {
                "version": wanted_version,
                "artifact_url": artifact["url"],
                "signature_url": artifact["signature_url"],
                "compressed_blake3": compressed_digest,
                "wasm_blake3": wasm_digest,
                "descriptor_sha256": hashlib.sha256(
                    descriptor_path.read_bytes()
                ).hexdigest(),
                "signature_bundle_sha256": signature_sha256,
            }

        provenance_path.parent.mkdir(parents=True, exist_ok=True)
        provenance_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "host": {
                        "scryer_version": ".".join(map(str, scryer_version)),
                        "sdk_version": ".".join(map(str, sdk_version)),
                        "sdk_constraint": legacy_sdk_constraint(sdk_version),
                    },
                    "catalog_redirect_url": CATALOG_REDIRECT_URL,
                    "catalog_url": catalog_artifact["url"],
                    "verified_bundles": {
                        "catalog_redirect_sha256": redirect_bundle_sha256,
                        "catalog_sha256": catalog_bundle_sha256,
                    },
                    "signer": {
                        "repository": OFFICIAL_PLUGIN_REPOSITORY,
                        "workflow": OFFICIAL_PLUGIN_WORKFLOW,
                    },
                    "plugins": provenance_plugins,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )

    publish_directory(output_dir, final_output_dir)
    shutil.rmtree(staging_parent, ignore_errors=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, KeyError, ValueError) as error:
        print(f"built-in materialization failed: {error}", file=sys.stderr)
        raise SystemExit(1)
