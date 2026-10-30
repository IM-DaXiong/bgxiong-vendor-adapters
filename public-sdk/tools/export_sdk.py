"""Export only the sealed public release inventory. Python 3.11+, standard library.

This tool measures/copies explicit inputs; it never builds, executes binaries, grants
licenses, configures Cargo, discovers user caches, or declares SDK_READY. Directory
publication has an incomplete window: ownership is claimed with mkdir(exist_ok=False),
and only this tool's new directory is cleaned after a failure. It never overwrites.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tempfile
import tomllib


class ExportError(Exception):
    pass


GOVERNANCE_OUTPUTS = ("release-lock.json", "RELEASE-STATUS.json")
INVENTORY_EXCLUSIONS = {"release-files.json", *GOVERNANCE_OUTPUTS}


def require(condition, message):
    if not condition:
        raise ExportError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def relative(value):
    require(isinstance(value, str) and value and "\\" not in value,
            "INVALID_PATH: expected a nonempty POSIX relative path")
    p = PurePosixPath(value)
    require(not p.is_absolute() and p.as_posix() == value and
            all(part not in ("", ".", "..") and not part.endswith((".", " ")) and
                not any(c in part for c in ':*?<>|\x00') and
                not any(ord(c) < 32 or ord(c) == 127 for c in part) and
                not re.fullmatch(r"(?i)(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", part)
                for part in p.parts), "INVALID_PATH: unsafe or noncanonical path " + value)
    return value


def no_links(path, leaf_file=False):
    """Check lexical ancestors before any resolve/read; junctions are links too."""
    path = Path(os.path.abspath(path))
    for item in [*reversed(path.parents), path]:
        try:
            info = item.lstat()
        except FileNotFoundError:
            raise ExportError("MISSING_INPUT: " + str(item)) from None
        require(not stat.S_ISLNK(info.st_mode) and not
                (stat.S_ISREG(info.st_mode) and info.st_nlink > 1) and not
                (getattr(info, "st_file_attributes", 0) & 0x400),
                "LINK_REJECTED: " + str(item))
    require(path.is_file() if leaf_file else path.is_dir(),
            "EXPECTED_FILE" if leaf_file else "EXPECTED_DIRECTORY")
    return path


def absolute(value, leaf_file=False):
    p = Path(value)
    require(p.is_absolute(), "ABSOLUTE_PATH_REQUIRED")
    return no_links(p, leaf_file)


def read_json(path):
    no_links(path, True)
    # Developer inventories are explicit bounded JSON inputs, not media payloads.
    require(path.stat().st_size <= 32 * 1024 * 1024, "INVENTORY_TOO_LARGE")
    def pairs(items):
        result = {}
        for k, v in items:
            require(k not in result, "DUPLICATE_JSON_KEY: " + k)
            result[k] = v
        return result
    try:
        return json.loads(path.read_bytes().decode("utf-8"), object_pairs_hook=pairs)
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        raise ExportError("INVALID_JSON: " + str(e)) from None


def hash_value(value):
    require(isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value),
            "MISSING_OR_INVALID_SHA256: refresh the explicit inventory after generation")
    return value


def safe_sdk_path(value):
    relative(value)
    parts = PurePosixPath(value).parts
    require(parts[0].casefold() not in {"ui", "src-tauri", "installer-user-examples", "runtimes", "models"},
            "APPLICATION_TREE_INPUT: " + value)
    require(not any(p.casefold() in {"target", "node_modules", ".git", ".bgx-sdk",
                "logs", "workspaces", "app_data", ".venv", ".env", "keys"} for p in parts),
            "EXCLUDED_SDK_INPUT: " + value)
    require(not any(p.startswith(".") and p != ".gitignore" for p in parts),
            "HIDDEN_SDK_INPUT: " + value)
    require(not value.lower().endswith((".key", ".pem", ".pfx", ".p12", ".log", ".sqlite", ".db")),
            "PRIVATE_OR_DEVELOPMENT_INPUT: " + value)
    require("dist" not in parts or value.startswith("tools/form-reference/dist/"),
            "UNAPPROVED_DIST_INPUT: " + value)
    return value


def load_manifest(sdk):
    value = read_json(sdk / "release-manifest.json")
    require(value.get("schemaVersion") == 1 and value.get("sdkRelease") == "2.0.0",
            "UNSUPPORTED_RELEASE_MANIFEST")
    require(value.get("projectLicense") == "LicenseRef-Proprietary", "OWNER_LICENSE_CHANGED")
    require(isinstance(value.get("files"), list) and value["files"], "EMPTY_RELEASE_INVENTORY")
    seen = set()
    for item in value["files"]:
        source, target = safe_sdk_path(item["source"]), safe_sdk_path(item["target"])
        require(source == target, "SOURCE_TARGET_MAPPING_NOT_SUPPORTED")
        require(target.casefold() not in {"release-manifest.json", *[n.casefold() for n in INVENTORY_EXCLUSIONS]} and target.casefold() not in seen,
                "DUPLICATE_OR_SELF_REFERENTIAL_MANIFEST_ENTRY")
        seen.add(target.casefold())
        require(item.get("required") is True, "OPTIONAL_SOURCE_ENTRIES_NOT_SUPPORTED")
    for role in ("cli", "worker"):
        require(value.get("binaryInputs", {}).get(role) in
                {"bin/bgx-adapter-v2", "bin/bgx-adapter-v2-worker"}, "INVALID_BINARY_TARGET")
    return value


def initial_governance():
    common = {"schemaVersion": 1, "sdkRelease": "2.0.0", "status": "INIT",
              "readiness": "PREPARED_NOT_ACCEPTED", "sealed": False, "sdkReady": False,
              "moduleIntegration": "NOT_ADOPTED", "publication": "NOT_PUBLISHED"}
    gates = {"G%02d" % n: "NOT_RUN" for n in range(7)}
    lock = {**common, "kind": "SDK_RELEASE_LOCK_V1", "candidateInventories": [],
            "gates": gates, "G06": "NOT_RUN", "sourceDigests": None, "builds": [], "candidateTests": []}
    status = {**common, "releaseLockDigest": None, "gates": gates,
              "mainApplicationV2Import": "NOT_SUPPORTED", "projectLicense": "LicenseRef-Proprietary"}
    return {"release-lock.json": lock, "RELEASE-STATUS.json": status}


def governance_identity(root, inventory_sha):
    """Check byte/phase linkage only; external evidence verification is SD31/32."""
    lock = read_json(root / GOVERNANCE_OUTPUTS[0]);status = read_json(root / GOVERNANCE_OUTPUTS[1])
    hashes = {name: digest(no_links(root / name, True)) for name in GOVERNANCE_OUTPUTS}
    for value in [lock, status]:
        require(value.get("schemaVersion") == 1 and value.get("sdkRelease") == "2.0.0"
                and value.get("moduleIntegration") == "NOT_ADOPTED", "INVALID_GOVERNANCE_IDENTITY")
    require(lock.get("kind") == "SDK_RELEASE_LOCK_V1" and status.get("publication") == "NOT_PUBLISHED",
            "INVALID_GOVERNANCE_SCOPE")
    require(status.get("projectLicense") == "LicenseRef-Proprietary"
            and status.get("mainApplicationV2Import") == "NOT_SUPPORTED", "GOVERNANCE_LICENSE_OR_HOST_SCOPE_CHANGED")
    stage = status.get("status")
    if stage == "INIT":
        require({name: read_json(root / name) for name in GOVERNANCE_OUTPUTS} == initial_governance(),
                "INVALID_INITIAL_GOVERNANCE")
    else:
        require(stage in {"RELEASE_CANDIDATE", "SDK_READY"}, "INVALID_GOVERNANCE_STAGE")
        require(lock.get("status") in {"RELEASE_CANDIDATE", "SDK_READY"}
                and lock.get("sealed") is True and status.get("sealed") is True,
                "UNSEALED_GOVERNANCE_OVERLAY")
        require(status.get("releaseLockDigest") == hashes["release-lock.json"], "GOVERNANCE_LOCK_DIGEST_MISMATCH")
        inventories = lock.get("candidateInventories")
        require(isinstance(inventories, list) and len(inventories) == 2
                and all(hash_value(x) == x for x in inventories) and inventory_sha in inventories,
                "GOVERNANCE_CANDIDATE_INVENTORY_MISMATCH")
        expected_gates = {"G%02d" % n: "VERIFIED" for n in range(6)}
        expected_gates["G06"] = "VERIFIED" if stage == "SDK_READY" else "NOT_RUN"
        require(status.get("gates") == expected_gates and status.get("sdkReady") is (stage == "SDK_READY"),
                "GOVERNANCE_STATUS_STAGE_CHAIN_MISMATCH")
        gate_refs = lock.get("gates")
        require(isinstance(gate_refs, dict) and set(gate_refs) in
                [{"G%02d" % n for n in range(6)}, {"G%02d" % n for n in range(7)}],
                "GOVERNANCE_LOCK_GATE_CHAIN_MISSING")
        for ref in gate_refs.values():
            require(isinstance(ref, dict) and set(ref) == {"path", "sha256"}, "GOVERNANCE_GATE_REFERENCE_INVALID")
            relative(ref["path"]);hash_value(ref["sha256"])
        require(type(lock.get("sdkReady")) is bool and lock.get("G06") in {"NOT_RUN", "VERIFIED"},
                "GOVERNANCE_LOCK_STAGE_MISSING")
        if stage == "RELEASE_CANDIDATE":
            require(lock["status"] == stage and lock["sdkReady"] is False and lock["G06"] == "NOT_RUN",
                    "GOVERNANCE_CANDIDATE_STAGE_MISMATCH")
        else:
            require((lock["status"], lock["sdkReady"], lock["G06"]) in
                    [("RELEASE_CANDIDATE", False, "NOT_RUN"), ("SDK_READY", True, "VERIFIED")],
                    "GOVERNANCE_READY_STAGE_MISMATCH")
    return hashes, {"status": stage, "metadataIdentityChecked": True, "evidenceValidated": False,
                    "readinessAttestedByExporter": False,
                    "note": "Governance hashes and phase links only; SD31/SD32 must verify actual execution/evidence."}


def verify_locks(sdk, manifest):
    included = {f["source"] for f in manifest["files"]}
    a = sdk / "templates/template-lock.json"
    b = sdk / "templates/rust/template-lock.json"
    require(no_links(a, True).read_bytes() == no_links(b, True).read_bytes(), "TEMPLATE_LOCK_MIRROR_MISMATCH")
    lock = read_json(a)
    for item in lock["generatedFiles"]:
        target = "templates/rust/" + relative(item["target"])
        require(target in included, "TEMPLATE_GENERATED_FILE_NOT_IN_RELEASE: " + target)
        require(digest(no_links(sdk / target, True)) == hash_value(item["sha256"]), "TEMPLATE_TARGET_HASH_MISMATCH")
        for source in item["sources"]:
            path = relative(source["path"])
            require(path in included, "TEMPLATE_SOURCE_NOT_IN_RELEASE: " + path)
            require(digest(no_links(sdk / path, True)) == hash_value(source["sha256"]), "TEMPLATE_SOURCE_HASH_MISMATCH")
    # Public examples carry their own precise author source/generation provenance.
    author = read_json(sdk / "examples/authoring-lock.json")
    for example in author.get("examples", []):
        for item in example.get("sourceFiles", []) + example.get("sharedSources", []):
            path = relative(item["path"])
            if path in included:
                require(digest(no_links(sdk / path, True)) == hash_value(item["sha256"]), "EXAMPLE_SOURCE_HASH_MISMATCH: " + path)
        for item in example.get("generatedFiles", []):
            path = relative(item["target"])
            if path in included:
                require(digest(no_links(sdk / path, True)) == hash_value(item["sha256"]), "EXAMPLE_GENERATED_HASH_MISMATCH: " + path)
    return {"templateMirrorsMatch": True, "templateGeneratedAndSourceHashesMatch": True,
            "includedExampleSourceAndGenerationHashesMatch": True}


def cargo_source_closure(sdk, manifest):
    """Check included local path edges, without resolving registry features/builds."""
    included = {f["source"] for f in manifest["files"]}
    root = tomllib.loads(no_links(sdk / "Cargo.toml", True).read_text(encoding="utf8"))
    require(root.get("workspace", {}).get("package", {}).get("license") == "LicenseRef-Proprietary",
            "OWNER_WORKSPACE_LICENSE_MISMATCH")
    for package in tomllib.loads(no_links(sdk / "Cargo.lock", True).read_text(encoding="utf8"))["package"]:
        require(package.get("source") in (None, "registry+https://github.com/rust-lang/crates.io-index"),
                "UNREVIEWED_CARGO_LOCK_SOURCE")
    for member in root.get("workspace", {}).get("members", []):
        require(relative(member + "/Cargo.toml") in included, "WORKSPACE_MEMBER_NOT_IN_RELEASE")
    edges = []
    for name in sorted(n for n in included if n.endswith("Cargo.toml")):
        path = no_links(sdk / name, True)
        value = tomllib.loads(path.read_text(encoding="utf8"))
        require(not value.get("patch") and not value.get("replace"), "UNREVIEWED_CARGO_SOURCE_OVERRIDE")
        tables = [value, value.get("workspace", {})] + list(value.get("target", {}).values())
        for table in tables:
            for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                for dependency, rule in table.get(kind, {}).items():
                    require(not isinstance(rule, dict) or ("git" not in rule and "registry" not in rule),
                            "UNREVIEWED_CARGO_DEPENDENCY_SOURCE")
                    if isinstance(rule, dict) and "path" in rule:
                        require(isinstance(rule["path"], str) and not Path(rule["path"]).is_absolute(),
                                "ABSOLUTE_CARGO_PATH_DEPENDENCY")
                        target = Path(os.path.abspath(path.parent / rule["path"] / "Cargo.toml"))
                        require(target.is_relative_to(sdk), "CARGO_PATH_DEPENDENCY_ESCAPE")
                        target_name = target.relative_to(sdk).as_posix()
                        require(target_name in included, "CARGO_PATH_DEPENDENCY_NOT_IN_RELEASE: " + target_name)
                        no_links(target, True)
                        edges.append({"manifest": name, "dependency": dependency, "target": target_name})
    return {"includedLocalPathEdgesChecked": edges, "registryFeatureClosure": "NOT_RESOLVED_NO_BUILD",
            "offlineBuildValidation": "NOT_RUN"}


def license_material(path, root_lock):
    data = read_json(path)
    require(data.get("inputs", {}).get("workspaceLicense") == "LicenseRef-Proprietary", "LICENSE_INVENTORY_OWNER_MISMATCH")
    require(data.get("inputs", {}).get("Cargo.lock") == digest(root_lock), "LICENSE_INVENTORY_LOCK_MISMATCH")
    locked = {(x["name"], x["version"], x["checksum"]) for x in
              tomllib.loads(root_lock.read_text(encoding="utf8"))["package"] if "checksum" in x}
    packages = data.get("rustPackages", [])
    require(len(packages) == len({(x["name"], x["version"], x["cargoLockChecksum"]) for x in packages}),
            "DUPLICATE_LICENSE_PACKAGE")
    require({(x["name"], x["version"], x["cargoLockChecksum"]) for x in packages} == locked,
            "LICENSE_INVENTORY_PACKAGE_SET_MISMATCH")
    objects = data.get("licenseTextObjects", {})
    for key, obj in objects.items():
        raw = obj["utf8Text"].encode("utf8")
        require(sha(raw) == hash_value(key) and len(raw) == obj["bytes"], "LICENSE_TEXT_HASH_MISMATCH")
    generated, summary, missing = {}, [], []
    for package in packages:
        files = []
        for item in package["licenseFiles"]:
            target = relative("THIRD-PARTY/licenses/rust/" + package["name"] + "/" + package["version"] + "/" + item["relativePath"])
            key = hash_value(item["sha256"])
            require(item.get("textObjectSha256") == key and key in objects, "LICENSE_TEXT_NOT_EMBEDDED: " + target)
            raw = objects[key]["utf8Text"].encode("utf8")
            require(sha(raw) == key and len(raw) == item["bytes"], "LICENSE_FILE_BYTE_MISMATCH")
            require(target not in generated, "DUPLICATE_LICENSE_TARGET")
            generated[target] = raw
            files.append({"path": target, "sha256": key, "bytes": len(raw)})
        if not files:
            missing.append(package["name"] + "@" + package["version"])
        summary.append({"name": package["name"], "version": package["version"],
                        "declaredLicenseExpression": package["declaredLicenseExpression"],
                        "cargoLockChecksum": package["cargoLockChecksum"], "files": files,
                        "licenseBranchSelected": None})
    # node_modules/compiler is not shipped. Keep actual compiler notice material for provenance,
    # without calling its license the license of generated/authored SDK code.
    npm = []
    for npm_lock in data.get("npmLocks", []):
        for package in npm_lock["packages"]:
            files = []
            for item in package["licenseFiles"]:
                key = hash_value(item["sha256"])
                require(key in objects, "NPM_LICENSE_TEXT_NOT_EMBEDDED")
                target = relative("THIRD-PARTY/licenses/npm/typescript/" + package["version"] + "/" + item["relativePath"])
                raw = objects[key]["utf8Text"].encode("utf8")
                require(sha(raw) == key and len(raw) == item["bytes"], "NPM_LICENSE_BYTE_MISMATCH")
                require(target not in generated or generated[target] == raw, "NPM_LICENSE_COLLISION")
                generated[target] = raw; files.append({"path": target, "sha256": key, "bytes": len(raw)})
            npm.append({"package": "typescript", "version": package["version"],
                        "sourceLock": relative(npm_lock["lockPath"]), "sourceLockSha256": npm_lock["lockSha256"],
                        "compilerDistributed": False, "files": files})
    index = {"schemaVersion": 1, "scope": "Full Cargo.lock superset, not target/feature-trimmed binary SBOM",
             "projectLicense": "LicenseRef-Proprietary", "sourceInventorySha256": digest(path),
             "legalConfirmation": False, "rustPackages": summary, "compilerProvenance": npm,
             "missingStandaloneLicenseAttachments": missing,
             "noticeClosure": "NOT_ACCEPTED: resolve missing attachments, referenced author attributions and chosen license branches for actual release closure",
             "spdxCanonicalDatabaseTexts": "Reference data, not governing permissions for every license in that database; not copied as governing notices",
             "licenseDoesNotGrantOwnerSDKRights": True}
    generated["THIRD-PARTY/index.json"] = json_bytes(index)
    return generated, missing


def tree_files(root):
    """Only used for explicit vendor input and completed-output verification."""
    no_links(root)
    found = []
    for folder, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            no_links(Path(folder) / name)
        for name in files:
            path = no_links(Path(folder) / name, True)
            found.append(path.relative_to(root).as_posix())
    return sorted(found)


def vendor_material(folder, inventory, cargo_lock):
    if folder is None:
        require(inventory is None, "VENDOR_DIRECTORY_REQUIRED")
        return [], {"supplied": False, "status": "NOT_SUPPLIED", "offlineReady": False}
    require(inventory is not None, "EXPLICIT_VENDOR_INVENTORY_REQUIRED")
    listing = read_json(inventory)
    require(listing.get("schemaVersion") == 1 and isinstance(listing.get("files"), list), "INVALID_VENDOR_INVENTORY")
    entries = listing["files"]
    names = [relative(x["path"]) for x in entries]
    require(len(names) == len(set(n.casefold() for n in names)), "VENDOR_CASE_OR_PATH_COLLISION")
    require(sorted(names) == tree_files(folder), "VENDOR_INVENTORY_FILE_SET_MISMATCH")
    copies = []
    for item in entries:
        name = relative(item["path"]); source = no_links(folder / name, True)
        require(digest(source) == hash_value(item["sha256"]), "VENDOR_FILE_HASH_MISMATCH: " + name)
        require(source.stat().st_size == item["bytes"], "VENDOR_FILE_SIZE_MISMATCH")
        copies.append((source, "THIRD-PARTY/cargo-vendor/" + name, item["sha256"], "explicit-cargo-vendor"))
    actual_packages = set()
    for name in names:
        if name.count("/") == 1 and name.endswith("/.cargo-checksum.json"):
            sub = folder / PurePosixPath(name).parent
            checksum = read_json(sub / ".cargo-checksum.json")
            require(set(checksum["files"]) == {n[len(sub.name)+1:] for n in names
                    if n.startswith(sub.name + "/") and n != name}, "VENDOR_CRATE_CHECKSUM_FILE_SET_MISMATCH")
            for f, expected in checksum["files"].items():
                require(digest(no_links(sub / relative(f), True)) == hash_value(expected), "VENDOR_CRATE_CONTENT_HASH_MISMATCH")
            package = tomllib.loads(no_links(sub / "Cargo.toml", True).read_text(encoding="utf8"))["package"]
            actual_packages.add((package["name"], package["version"], checksum["package"]))
    locked = {(x["name"], x["version"], x["checksum"]) for x in
              tomllib.loads(cargo_lock.read_text(encoding="utf8"))["package"] if "checksum" in x}
    require(actual_packages <= locked, "VENDOR_PACKAGE_OUTSIDE_LOCK")
    require(all(PurePosixPath(n).parts[0] in {PurePosixPath(x).parts[0] for x in names if x.endswith("/.cargo-checksum.json")} for n in names), "VENDOR_UNASSOCIATED_FILE")
    return copies, {"supplied": True, "inventorySha256": digest(inventory),
                    "packageCount": len(actual_packages), "fullLockStored": actual_packages == locked,
                    "missingPackages": [n + "@" + v for n, v, c in sorted(locked - actual_packages)],
                    "status": "BYTES_CHECKED_OFFLINE_BUILD_NOT_VALIDATED", "offlineReady": False,
                    "provenanceLimit": "Explicit extracted-source hashes/checksum documents verified; original crate archive provenance and build source override are not proven by extracted bytes alone",
                    "configuration": "No implicit .cargo/config is generated. Author CLI rejects ancestor config; an explicit supported source-override build path must be separately validated."}


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf8")


def new_file(root, name, raw=None, source=None):
    target = root / relative(name)
    target.parent.mkdir(parents=True, exist_ok=True)
    no_links(target.parent)
    with target.open("xb") as destination:
        if source is not None:
            with no_links(source, True).open("rb") as incoming:
                shutil.copyfileobj(incoming, destination, 1024 * 1024)
        else:
            destination.write(raw)
        destination.flush(); os.fsync(destination.fileno())
    if source is not None:
        target.chmod(stat.S_IMODE(source.stat().st_mode))
    return target


def verify_candidate(root):
    root = absolute(root)
    index = read_json(root / "release-files.json")
    require(index.get("schemaVersion") == 1 and index.get("readiness") == "PREPARED_NOT_ACCEPTED",
            "INVALID_RELEASE_FILES")
    require(index.get("runtimeValidation") == "NOT_RUN" and index.get("sdkReady") is False,
            "EXPORTER_CANNOT_ATTEST_RUNTIME_READINESS")
    names = [relative(x["path"]) for x in index["files"]]
    require(len(names) == len(set(n.casefold() for n in names)) and not
            ({n.casefold() for n in names} & {n.casefold() for n in INVENTORY_EXCLUSIONS}),
            "DUPLICATE_OR_RECURSIVE_FILE_INVENTORY")
    require(tree_files(root) == sorted(names + ["release-files.json", *GOVERNANCE_OUTPUTS]), "CANDIDATE_FILE_SET_MISMATCH")
    for item in index["files"]:
        path = no_links(root / relative(item["path"]), True)
        require(digest(path) == hash_value(item["sha256"]) and path.stat().st_size == item["bytes"],
                "CANDIDATE_HASH_MISMATCH: " + item["path"])
    manifest = load_manifest(root)
    require({x["source"] for x in manifest["files"]} <= set(names), "CANDIDATE_SOURCE_INVENTORY_INCOMPLETE")
    for item in manifest["files"]:
        require(digest(root / item["target"]) == hash_value(item["sha256"]), "CANDIDATE_MANIFEST_HASH_MISMATCH")
    governance_hashes, governance_metadata = governance_identity(root, digest(root / "release-files.json"))
    return {"status": "BYTE_INVENTORY_VERIFIED", "fileCount": len(names),
            "releaseFilesSha256": digest(root / "release-files.json"), "sdkReady": False,
            "governanceSha256": governance_hashes, "governanceMetadataIdentity": governance_metadata,
            "note": "Unkeyed checksums detect mismatch, not authenticity. Runtime/legal/offline acceptance remains separate."}


def export(sdk, out, cli, worker, license_inventory, owner_license=None,
           cargo_vendor=None, cargo_vendor_inventory=None):
    sdk = absolute(sdk);cli = absolute(cli, True);worker = absolute(worker, True)
    license_inventory = absolute(license_inventory, True)
    if owner_license is not None:
        owner_license = absolute(owner_license, True)
        require(owner_license.stat().st_size > 0, "EMPTY_OWNER_TERMS")
    if cargo_vendor is not None:cargo_vendor = absolute(cargo_vendor)
    if cargo_vendor_inventory is not None:cargo_vendor_inventory = absolute(cargo_vendor_inventory, True)
    out = Path(out); require(out.is_absolute(), "ABSOLUTE_OUTPUT_REQUIRED")
    out = Path(os.path.abspath(out)); parent = no_links(out.parent)
    require(not out.exists() and not out.is_symlink(), "OUTPUT_ALREADY_EXISTS")
    require(not out.is_relative_to(sdk) and not sdk.is_relative_to(out), "OUTPUT_MUST_BE_SEPARATE_FROM_SDK")
    for item in [cli, worker, license_inventory, owner_license, cargo_vendor, cargo_vendor_inventory]:
        if item is not None:require(not item.is_relative_to(out), "INPUT_WITHIN_OUTPUT")
    manifest = load_manifest(sdk);copies = []
    for item in manifest["files"]:
        source = no_links(sdk / relative(item["source"]), True)
        require(digest(source) == hash_value(item["sha256"]), "SOURCE_HASH_MISMATCH: " + item["source"])
        copies.append((source, item["target"], item["sha256"], "sealed-sdk-source"))
    lock_results = verify_locks(sdk, manifest)
    cargo_closure = cargo_source_closure(sdk, manifest)
    for role, source in [("cli", cli), ("worker", worker)]:
        require(source.stat().st_size > 0, "EMPTY_BINARY_INPUT")
        target = manifest["binaryInputs"][role] + (".exe" if source.suffix.lower() == ".exe" else "")
        copies.append((source, target, digest(source), "explicit-" + role + "-binary-not-executed"))
    copies.append((sdk / "release-manifest.json", "release-manifest.json", digest(sdk / "release-manifest.json"), "sealed-manifest-input-no-self-hash"))
    generated, missing = license_material(license_inventory, sdk / "Cargo.lock")
    for npm_lock in read_json(license_inventory).get("npmLocks", []):
        path = safe_sdk_path(npm_lock["lockPath"])
        require(digest(no_links(sdk / path, True)) == npm_lock["lockSha256"], "LICENSE_NPM_LOCK_MISMATCH")
    vendor, vendor_status = vendor_material(cargo_vendor, cargo_vendor_inventory, sdk / "Cargo.lock")
    copies.extend(vendor)
    if owner_license is not None:
        copies.append((owner_license, "OWNER-LICENSE.txt", digest(owner_license), "explicit-owner-terms-not-legally-confirmed"))
    names = [x[1] for x in copies] + list(generated)
    require(len(names) == len(set(n.casefold() for n in names)), "RELEASE_TARGET_COLLISION")
    # Claim only a new directory; mkdir racing an existing writer fails without cleanup.
    try:
        out.mkdir(exist_ok=False)
    except FileExistsError:
        raise ExportError("OUTPUT_ALREADY_EXISTS") from None
    try:
        records = []
        for source, name, expected, origin in copies:
            target = new_file(out, name, source=source)
            require(digest(target) == expected, "SOURCE_CHANGED_DURING_COPY: " + name)
            records.append({"path": name, "sha256": expected, "bytes": target.stat().st_size, "origin": origin})
        for name, raw in generated.items():
            new_file(out, name, raw=raw)
            records.append({"path": name, "sha256": sha(raw), "bytes": len(raw), "origin": "generated-from-explicit-license-inventory"})
        inventory = {"schemaVersion": 1, "sdkRelease": manifest["sdkRelease"],
                     "readiness": "PREPARED_NOT_ACCEPTED", "sdkReady": False,
                     "runtimeValidation": "NOT_RUN", "legalConfirmation": False,
                     "projectLicense": "LicenseRef-Proprietary", "ownerTermsSupplied": owner_license is not None,
                     "mainApplicationV2Import": "NOT_SUPPORTED", "offlineReady": False,
                     "cargoVendor": vendor_status, "locks": lock_results, "cargoSourceClosure": cargo_closure,
                     "missingStandaloneLicenseAttachments": missing,
                     "binaryScope": "Explicit developer-provided bytes; OS/toolchain/loader behavior and six-operation execution require independent acceptance",
                     "externalPrerequisites": manifest.get("externalPrerequisites", []),
                     "publication": "Exclusive mkdir then file copies/sync/hash verification. Incomplete window exists; success only after complete byte verification. No atomic-directory claim.",
                     "files": sorted(records, key=lambda x: x["path"])}
        new_file(out, "release-files.json", raw=json_bytes(inventory))
        for name, value in initial_governance().items():
            new_file(out, name, raw=json_bytes(value))
        return verify_candidate(out)
    except BaseException:
        # Only remove the exclusively created directory at its unchanged lexical location.
        no_links(parent);no_links(out)
        require(out.parent == parent and not out.is_symlink(), "UNSAFE_OWNED_OUTPUT_CLEANUP")
        shutil.rmtree(out)
        raise


def refresh(sdk):
    sdk = absolute(sdk);value = load_manifest(sdk)
    # This does not discover paths, repair locks, build assets or update any other file.
    for item in value["files"]:
        item["sha256"] = digest(no_links(sdk / item["source"], True))
    target = sdk / "release-manifest.json"
    with tempfile.NamedTemporaryFile(dir=sdk, prefix=".release-manifest-", suffix=".tmp", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(json_bytes(value));stream.flush();os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink();raise
    try:
        os.replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)
    return {"status": "EXISTING_EXACT_PATH_HASHES_REFRESHED", "files": len(value["files"]), "sha256": digest(target)}


def self_test():
    """Byte/inventory tests only. All binaries/media are explicit stubs, never run."""
    results = []
    with tempfile.TemporaryDirectory(prefix="bgx-sdk-export-self-test-") as temp:
        root = Path(temp).absolute();sdk = root / "sdk";sdk.mkdir()
        files = {"Cargo.toml": b'[workspace.package]\nlicense="LicenseRef-Proprietary"\n',
                 "Cargo.lock": b'version=4\n[[package]]\nname="sample"\nversion="1.0.0"\nsource="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="' + b'1'*64 + b'"\n',
                 "templates/rust/src/lib.rs": b'// stub author fixture\n',
                 "templates/template-lock.json": json_bytes({"generatedFiles": []}),
                 "templates/rust/template-lock.json": json_bytes({"generatedFiles": []}),
                 "examples/authoring-lock.json": json_bytes({"examples": []})}
        for name, raw in files.items():new_file(sdk, name, raw=raw)
        manifest = {"schemaVersion": 1, "sdkRelease": "2.0.0", "projectLicense": "LicenseRef-Proprietary",
                    "binaryInputs": {"cli": "bin/bgx-adapter-v2", "worker": "bin/bgx-adapter-v2-worker"},
                    "files": [{"source": n, "target": n, "sha256": sha(b), "required": True} for n,b in files.items()]}
        (sdk/"release-manifest.json").write_bytes(json_bytes(manifest))
        text=b'Copyright fixture owner. Stub attribution; no production grant.\n'; key=sha(text)
        licenses = {"inputs": {"workspaceLicense": "LicenseRef-Proprietary", "Cargo.lock": digest(sdk/"Cargo.lock")},
                    "rustPackages": [{"name": "sample", "version": "1.0.0", "cargoLockChecksum": "1"*64,
                                      "declaredLicenseExpression": "LicenseRef-TestFixture", "licenseFiles": [
                                          {"relativePath": "LICENSE", "sha256": key, "bytes": len(text), "textObjectSha256": key}]}],
                    "licenseTextObjects": {key: {"utf8Text": text.decode(), "bytes": len(text)}}, "npmLocks": []}
        inventory=root/"license-inventory.json";inventory.write_bytes(json_bytes(licenses))
        cli=root/"stub-cli";worker=root/"stub-worker";cli.write_bytes(b'STUB-NOT-EXECUTABLE-CLI');worker.write_bytes(b'STUB-NOT-EXECUTABLE-WORKER')
        def run(name="out"):
            return export(sdk,root/name,cli,worker,inventory)
        def negative(label, call, code):
            try:call()
            except ExportError as e:
                require(code in str(e), "SELF_TEST_WRONG_REJECTION: " + str(e));results.append({"case":label,"status":"PASS","actualError":str(e).split(':')[0]})
            else:raise ExportError("SELF_TEST_ACCEPTED_INVALID_CASE: " + label)
        first=run();results.append({"case":"complete-explicit-stub-byte-export","status":"PASS","runtimeExecuted":False})
        require(set(first['governanceSha256'])==set(GOVERNANCE_OUTPUTS) and first['sdkReady'] is False,
                'SELF_TEST_GOVERNANCE_HASHES_MISSING')
        governed=root/'governed';run('governed')
        index=read_json(governed/'release-files.json')
        require(not set(x['path'] for x in index['files']) & INVENTORY_EXCLUSIONS,'SELF_TEST_CYCLIC_INVENTORY')
        results.append({'case':'acyclic-inventory-and-explicit-initial-governance','status':'PASS','runtimeExecuted':False})
        saved_index=(governed/'release-files.json').read_bytes()
        for name in INVENTORY_EXCLUSIONS:
            bad=json.loads(saved_index);bad['files'].append({'path':name,'sha256':'1'*64,'bytes':1})
            (governed/'release-files.json').write_bytes(json_bytes(bad))
            negative('reject-inventory-recursion-'+name,lambda:verify_candidate(governed),'DUPLICATE_OR_RECURSIVE')
        (governed/'release-files.json').write_bytes(saved_index)
        lock_path=governed/'release-lock.json';status_path=governed/'RELEASE-STATUS.json'
        initial=initial_governance();lock_path.unlink()
        negative('missing-generated-governance',lambda:verify_candidate(governed),'CANDIDATE_FILE_SET_MISMATCH')
        lock_path.write_bytes(json_bytes(initial['release-lock.json']))
        bad=dict(initial['RELEASE-STATUS.json'],sdkReady=True);status_path.write_bytes(json_bytes(bad))
        negative('initial-readiness-boolean-not-evidence',lambda:verify_candidate(governed),'INVALID_INITIAL_GOVERNANCE')
        status_path.write_bytes(json_bytes(initial['RELEASE-STATUS.json']))
        # Synthetic phase metadata fixtures test only byte linkage. No gates,
        # runtime executions or release acceptance are claimed by these stubs.
        gate_refs={'G%02d'%n:{'path':'synthetic-gate-%d.json'%n,'sha256':'2'*64} for n in range(6)}
        lock=dict(initial['release-lock.json'],status='RELEASE_CANDIDATE',sealed=True,
                  candidateInventories=[digest(governed/'release-files.json')]*2,gates=gate_refs)
        lock_path.write_bytes(json_bytes(lock))
        status=dict(initial['RELEASE-STATUS.json'],status='RELEASE_CANDIDATE',sealed=True,
                    releaseLockDigest=digest(lock_path),gates={**{g:'VERIFIED' for g in gate_refs},'G06':'NOT_RUN'})
        status_path.write_bytes(json_bytes(status))
        checked=verify_candidate(governed)
        require(checked['sdkReady'] is False and checked['governanceMetadataIdentity']['evidenceValidated'] is False,
                'SELF_TEST_METADATA_MUST_NOT_ATTEST_READINESS')
        results.append({'case':'synthetic-sealed-phase-byte-identity-only','status':'PASS','runtimeExecuted':False,'actualEvidenceVerified':False})
        bad=dict(status,releaseLockDigest='3'*64);status_path.write_bytes(json_bytes(bad))
        negative('governance-lock-digest-tamper',lambda:verify_candidate(governed),'GOVERNANCE_LOCK_DIGEST_MISMATCH')
        status_path.write_bytes(json_bytes(status));wrong=dict(lock,candidateInventories=['4'*64]*2)
        lock_path.write_bytes(json_bytes(wrong));status['releaseLockDigest']=digest(lock_path);status_path.write_bytes(json_bytes(status))
        negative('governance-inventory-unbound',lambda:verify_candidate(governed),'GOVERNANCE_CANDIDATE_INVENTORY_MISMATCH')
        lock_path.write_bytes(json_bytes(lock));status['releaseLockDigest']=digest(lock_path)
        status['gates']['G06']='VERIFIED';status_path.write_bytes(json_bytes(status))
        negative('candidate-premature-g06',lambda:verify_candidate(governed),'GOVERNANCE_STATUS_STAGE_CHAIN_MISMATCH')
        status.update(status='SDK_READY',sdkReady=True);status_path.write_bytes(json_bytes(status))
        checked=verify_candidate(governed)
        require(checked['sdkReady'] is False and not checked['governanceMetadataIdentity']['readinessAttestedByExporter'],
                'SELF_TEST_READY_OVERLAY_NOT_AUTHORITY')
        results.append({'case':'synthetic-ready-overlay-identity-never-runtime-attestation','status':'PASS','runtimeExecuted':False})
        negative('existing-output-preserved',run,'OUTPUT_ALREADY_EXISTS')
        before=digest(root/'out/bin/bgx-adapter-v2');negative('existing-output-still-preserved',run,'OUTPUT_ALREADY_EXISTS');require(digest(root/'out/bin/bgx-adapter-v2')==before,'SELF_TEST_OVERWRITE')
        source=sdk/'templates/rust/src/lib.rs';raw=source.read_bytes();source.unlink()
        negative('missing-inventory-source',lambda:run('missing'),'MISSING_INPUT');source.write_bytes(raw)
        source.write_bytes(b'tampered');negative('source-tamper',lambda:run('tamper'),'SOURCE_HASH_MISMATCH');source.write_bytes(raw)
        (root/'out/bin/bgx-adapter-v2').write_bytes(b'tampered');negative('candidate-tamper',lambda:verify_candidate(root/'out'),'CANDIDATE_HASH_MISMATCH')
        saved=(sdk/'release-manifest.json').read_bytes();bad=json.loads(saved);bad['files'][0]['source']='../escape';(sdk/'release-manifest.json').write_bytes(json_bytes(bad));negative('path-escape',lambda:run('escape'),'INVALID_PATH');(sdk/'release-manifest.json').write_bytes(saved)
        for name in INVENTORY_EXCLUSIONS:
            bad=json.loads(saved);bad['files'].append({'source':name,'target':name,'required':True,'sha256':'1'*64})
            (sdk/'release-manifest.json').write_bytes(json_bytes(bad))
            negative('governance-source-cycle-'+name,lambda:load_manifest(sdk),'DUPLICATE_OR_SELF_REFERENTIAL')
        (sdk/'release-manifest.json').write_bytes(saved)
        bad=json.loads(saved);bad['files'].append(dict(bad['files'][0]));(sdk/'release-manifest.json').write_bytes(json_bytes(bad));negative('duplicate-path',lambda:run('duplicate'),'DUPLICATE');(sdk/'release-manifest.json').write_bytes(saved)
        bad=json.loads(saved);bad['files'][0]['sha256']=None;(sdk/'release-manifest.json').write_bytes(json_bytes(bad));negative('pending-null-hash',lambda:run('pending'),'MISSING_OR_INVALID_SHA256');(sdk/'release-manifest.json').write_bytes(saved)
        with (sdk/'Cargo.toml').open('ab') as f:f.write(b'\n[workspace.dependencies]\nprivate={path="../private"}\n')
        negative('local-cargo-dependency-escape',lambda:cargo_source_closure(sdk,manifest),'CARGO_PATH_DEPENDENCY_ESCAPE');(sdk/'Cargo.toml').write_bytes(files['Cargo.toml'])
        broken=json.loads(inventory.read_bytes());broken['licenseTextObjects'][key]['utf8Text']='tampered';inventory.write_bytes(json_bytes(broken));negative('license-byte-tamper',lambda:run('license-tamper'),'LICENSE_TEXT_HASH_MISMATCH');inventory.write_bytes(json_bytes(licenses))
        # An explicit extracted Cargo source archive is not proof of an offline build.
        vendor=root/'vendor';vendor.mkdir();crate=vendor/'sample-1.0.0';crate.mkdir()
        crate_manifest=b'[package]\nname="sample"\nversion="1.0.0"\n'
        (crate/'Cargo.toml').write_bytes(crate_manifest)
        (crate/'.cargo-checksum.json').write_bytes(json_bytes({'package':'1'*64,'files':{'Cargo.toml':sha(crate_manifest)}}))
        vendor_index=root/'vendor-inventory.json'
        vendor_listing={'schemaVersion':1,'files':[{'path':n,'sha256':digest(vendor/n),'bytes':(vendor/n).stat().st_size} for n in tree_files(vendor)]}
        vendor_index.write_bytes(json_bytes(vendor_listing))
        export(sdk,root/'vendored',cli,worker,inventory,cargo_vendor=vendor,cargo_vendor_inventory=vendor_index)
        receipt=read_json(root/'vendored/release-files.json')
        require(receipt['offlineReady'] is False and receipt['cargoVendor']['fullLockStored'] is True and not (root/'vendored/.cargo').exists(), 'SELF_TEST_FALSE_OFFLINE_OR_CONFIG')
        results.append({'case':'explicit-vendor-byte-closure-no-offline-claim-no-implicit-config','status':'PASS','runtimeExecuted':False})
        (crate/'Cargo.toml').write_bytes(b'tampered')
        negative('vendor-hash-mismatch',lambda:export(sdk,root/'bad-vendor',cli,worker,inventory,cargo_vendor=vendor,cargo_vendor_inventory=vendor_index),'VENDOR_FILE_HASH_MISMATCH')
        (crate/'Cargo.toml').write_bytes(crate_manifest);(crate/'extra').write_bytes(b'not inventoried')
        negative('vendor-extra-file',lambda:export(sdk,root/'extra-vendor',cli,worker,inventory,cargo_vendor=vendor,cargo_vendor_inventory=vendor_index),'VENDOR_INVENTORY_FILE_SET_MISMATCH')
        (crate/'extra').unlink()
        # A Windows junction does not need symlink privilege. CPython's stdlib API
        # creates it only within this owned test directory; no shell/process is used.
        if os.name=='nt':
            try:
                import _winapi
                junction=root/'junction-sdk';_winapi.CreateJunction(str(sdk),str(junction))
            except (OSError,AttributeError) as e:
                results.append({'case':'native-windows-junction','status':'NOT_RUN_PLATFORM_CAPABILITY','errorType':type(e).__name__})
            else:
                try:negative('native-windows-junction',lambda:export(junction,root/'junction-out',cli,worker,inventory),'LINK_REJECTED')
                finally:os.rmdir(junction)
        # Exercise failure cleanup after output ownership has actually been claimed.
        original=globals()['new_file']
        def fail_write(destination, name, raw=None, source=None):
            if name=='bin/bgx-adapter-v2-worker':raise ExportError('INJECTED_WRITE_FAILURE')
            return original(destination,name,raw=raw,source=source)
        globals()['new_file']=fail_write
        try:negative('owned-incomplete-output-cleaned',lambda:run('write-fail'),'INJECTED_WRITE_FAILURE');require(not (root/'write-fail').exists(),'PARTIAL_OUTPUT_REMAINS')
        finally:globals()['new_file']=original
        # Native symlinks may require a Windows privilege; report that distinction.
        link=root/'linked-cli'
        try:link.symlink_to(cli)
        except OSError as e:results.append({'case':'native-symlink','status':'NOT_RUN_PLATFORM_CAPABILITY','errno':e.errno,'winerror':getattr(e,'winerror',None)})
        else:negative('native-symlink',lambda:export(sdk,root/'linked',link,worker,inventory),'LINK_REJECTED')
        hardlink=root/'hardlinked-cli'
        try:os.link(cli,hardlink)
        except OSError as e:results.append({'case':'native-hardlink','status':'NOT_RUN_PLATFORM_CAPABILITY','errno':e.errno})
        else:
            try:negative('native-hardlink',lambda:export(sdk,root/'hardlinked',hardlink,worker,inventory),'LINK_REJECTED')
            finally:hardlink.unlink()
        require(not any((root/n).exists() for n in ['missing','tamper','escape','duplicate','pending','license-tamper','write-fail','linked','hardlinked','bad-vendor','extra-vendor','junction-out']), 'SELF_TEST_UNEXPECTED_OUTPUT')
    return {"status":"SELF_TEST_BYTE_CASES_PASSED", "runtimeValidation":"NOT_RUN_STUB_INPUTS", "sdkReady":False, "cases":results, "ownedTemporaryDirectoryRemoved":True}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    mode=parser.add_mutually_exclusive_group();mode.add_argument('--self-test',action='store_true');mode.add_argument('--verify',type=Path);mode.add_argument('--refresh-manifest',action='store_true')
    for arg in ['sdk','out','cli','worker','license-inventory','owner-license','cargo-vendor','cargo-vendor-inventory']:
        parser.add_argument('--'+arg,type=Path)
    args=parser.parse_args()
    try:
        if args.self_test:
            require(not any(getattr(args,x) for x in ['sdk','out','cli','worker','license_inventory','owner_license','cargo_vendor','cargo_vendor_inventory']), 'SELF_TEST_TAKES_NO_EXTERNAL_INPUTS');result=self_test()
        elif args.verify:
            require(not any(getattr(args,x) for x in ['sdk','out','cli','worker','license_inventory','owner_license','cargo_vendor','cargo_vendor_inventory']), 'VERIFY_TAKES_ONLY_CANDIDATE');result=verify_candidate(args.verify)
        elif args.refresh_manifest:
            require(args.sdk is not None and not any(getattr(args,x) for x in ['out','cli','worker','license_inventory','owner_license','cargo_vendor','cargo_vendor_inventory']), 'REFRESH_REQUIRES_ONLY_SDK');result=refresh(args.sdk)
        else:
            require(all(getattr(args,x) is not None for x in ['sdk','out','cli','worker','license_inventory']), 'EXPORT_REQUIRES_SDK_OUT_CLI_WORKER_LICENSE_INVENTORY')
            result=export(args.sdk,args.out,args.cli,args.worker,args.license_inventory,args.owner_license,args.cargo_vendor,args.cargo_vendor_inventory)
        print(json.dumps(result,ensure_ascii=False));return 0
    except (ExportError,OSError,ValueError,KeyError,TypeError) as e:
        print(json.dumps({'status':'FAILED','error':str(e)},ensure_ascii=False),file=sys.stderr);return 1


if __name__=='__main__':
    raise SystemExit(main())
