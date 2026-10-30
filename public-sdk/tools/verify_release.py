#!/usr/bin/env python3
"""Verify explicit SDK release inputs; never build, publish, sign, or mark SDK_READY.

Python 3.11+, stdlib. --self-test uses only new temporary fixtures. Normal usage:
  verify_release.py --candidate-a ABS --candidate-b ABS --evidence-root ABS \
      --evidence ABS_JSON --out ABS_NEW_DIRECTORY [--repro-proof]

The explicit evidence JSON has schemaVersion=1, sdkRelease, builds=[A,B],
binaryDifferences=[], gates={G00:ref,...,G05:ref}, candidateTests=[A,B]. Refs
are {path: POSIX-relative-to-evidence-root, sha256}. Builds contain sourceRoot
(absolute), sourcesBefore/sourcesAfter (the full sealed source rows), toolchain
(hashed file ref), executions (execution refs), and binaries={cli,worker}, whose
values are absolute output paths. Candidate tests contain releaseFilesSha256,
executions, conformanceRoot (relative directory under evidence-root).

An SDK_RELEASE_EXECUTION_V1 receipt has role BUILD/CANDIDATE_TEST/GATE, scope
SDK_RELEASE, command, cwd, executableSha256, inputsBefore/inputsAfter [{path:
absolute,sha256}], outputs [{path:absolute,sha256}], stdout/stderr refs,
exitCode, timedOut, durationSeconds. The caller captures actual process results;
this verifier checks the measured files/logs/closure, never stdout keywords.
Unkeyed imported receipts are evidence, not cryptographic process attestation.

Each SDK_GATE_EVIDENCE_V1 has gate, status=VERIFIED, sourceDigest, tasks (exact
SD00..30 gate members), taskEvidence (taskId to hashed task report), executions.
A historical VERIFIED label alone is not
accepted. All source-bound execution receipts must still match measured bytes.

--repro-proof verifies two actual builds and writes only measured reproduction
proof. It does not need C35/G06 or declare RELEASE_CANDIDATE. The conformance
runner can then consume that proof for C35. Final sealing separately reevaluates
both candidates' actual C01-C36 evidence with the shared runner, checks G00-G05,
and writes external release-lock/status files. G06 is left for SD32. Neither
mode modifies the candidate trees or initialized source lock/status files.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import tomllib

sys.dont_write_bytecode = True
TOOLS = Path(__file__).absolute().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


# Share the exporter's existing strict path/inventory/mirror validators.
exporter = module("sdk_release_export", TOOLS / "export_sdk.py")
require = exporter.require
digest = exporter.digest
read_json = exporter.read_json
absolute = exporter.absolute
relative = exporter.relative
no_links = exporter.no_links
GATES = {"G00": range(0, 2), "G01": range(2, 9), "G02": range(9, 20),
         "G03": range(20, 26), "G04": range(26, 29), "G05": range(29, 31)}
VERSION = "2.0.0"


def object_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=False).encode("utf-8")).hexdigest()


def closed(value, keys, name):
    require(isinstance(value, dict) and set(value) == set(keys), "INVALID_FIELDS: " + name)
    return value


def release_version(value):
    require(value == VERSION, "RELEASE_VERSION_DRIFT")


def reference(root, value):
    closed(value, ("path", "sha256"), "evidence reference")
    path = no_links(root / relative(value["path"]), True)
    require(digest(path) == exporter.hash_value(value["sha256"]), "EVIDENCE_HASH_DRIFT: " + value["path"])
    return path


def measured_absolute_rows(rows, name):
    require(isinstance(rows, list) and bool(rows), "EMPTY_MEASURED_INPUTS: " + name)
    result = {}
    for item in rows:
        closed(item, ("path", "sha256"), name)
        path = absolute(Path(item["path"]), True)
        key = os.path.normcase(str(path))
        require(key not in result, "DUPLICATE_MEASURED_PATH")
        require(digest(path) == exporter.hash_value(item["sha256"]), "MEASURED_FILE_DRIFT: " + str(path))
        result[key] = item["sha256"]
    return result


def execution(root, ref, role):
    value = read_json(reference(root, ref))
    closed(value, ("schemaVersion", "kind", "scope", "role", "command", "cwd",
                   "executableSha256", "inputsBefore", "inputsAfter", "outputs",
                   "stdout", "stderr", "exitCode", "timedOut", "durationSeconds"), "execution")
    require(value["schemaVersion"] == 1 and value["kind"] == "SDK_RELEASE_EXECUTION_V1"
            and value["scope"] == "SDK_RELEASE" and value["role"] == role, "EXECUTION_SCOPE_MISMATCH")
    command = value["command"]
    require(isinstance(command, list) and len(command) >= 2 and
            all(isinstance(x, str) and x and "\x00" not in x for x in command), "EXECUTION_COMMAND_MISSING")
    executable = absolute(Path(command[0]), True)
    require(digest(executable) == exporter.hash_value(value["executableSha256"]), "EXECUTABLE_DRIFT")
    absolute(Path(value["cwd"]))
    require(type(value["exitCode"]) is int and value["exitCode"] == 0 and
            value["timedOut"] is False, "EXECUTION_NOT_SUCCESSFUL")
    require(type(value["durationSeconds"]) in (int, float) and
            math.isfinite(value["durationSeconds"]) and value["durationSeconds"] > 0,
            "EXECUTION_NOT_RECORDED")
    before = measured_absolute_rows(value["inputsBefore"], "inputsBefore")
    require(before == measured_absolute_rows(value["inputsAfter"], "inputsAfter"), "EXECUTION_SOURCE_CHANGED")
    require(isinstance(value["outputs"], list), "OUTPUTS_MUST_BE_LIST")
    outputs = measured_absolute_rows(value["outputs"], "outputs") if value["outputs"] else {}
    reference(root, value["stdout"]); reference(root, value["stderr"])
    return value, before, outputs


def source_rows(candidate, manifest):
    names = sorted([item["source"] for item in manifest["files"]] + ["release-manifest.json"])
    return [{"path": name, "sha256": digest(no_links(candidate / name, True))} for name in names]


def subset_hash(rows, predicate):
    selected = [row for row in rows if predicate(row["path"])]
    require(bool(selected), "MISSING_SOURCE_CATEGORY")
    return object_hash(selected)


def candidate(root):
    root = absolute(root)
    byte_check = exporter.verify_candidate(root)
    manifest = exporter.load_manifest(root)
    release_version(manifest["sdkRelease"])
    protocol = read_json(root / "protocol/vendor-adapter-protocol.json")
    cargo = tomllib.loads(no_links(root / "Cargo.toml", True).read_text(encoding="utf-8"))
    require(protocol.get("contractRelease") == VERSION and
            cargo.get("workspace", {}).get("package", {}).get("version") == VERSION,
            "PROTOCOL_OR_CARGO_VERSION_DRIFT")
    exporter.verify_locks(root, manifest)
    exporter.cargo_source_closure(root, manifest)
    index = read_json(root / "release-files.json")
    # Internal candidate readiness is not legal publication clearance. Keep the
    # actual upstream omissions visible rather than invent missing LICENSE files.
    notices = read_json(root / "THIRD-PARTY/index.json")
    locked = {(p["name"], p["version"], p["checksum"]) for p in
              tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))["package"] if "checksum" in p}
    packages = notices.get("rustPackages", [])
    require(len(packages) == len(locked) and
            {(p["name"], p["version"], p["cargoLockChecksum"]) for p in packages} == locked,
            "LICENSE_INVENTORY_CLOSURE_MISMATCH")
    missing = sorted(p["name"] + "@" + p["version"] for p in packages if not p["files"])
    require(missing == sorted(notices.get("missingStandaloneLicenseAttachments", [])) ==
            sorted(index.get("missingStandaloneLicenseAttachments", [])), "LICENSE_OMISSION_FACTS_DRIFT")
    require(notices.get("legalConfirmation") is False and index.get("legalConfirmation") is False and
            notices.get("projectLicense") == "LicenseRef-Proprietary", "UNSUPPORTED_LICENSE_CLEARANCE_CLAIM")
    rows = source_rows(root, manifest)
    binaries = {}
    for role, name in manifest["binaryInputs"].items():
        matches = [x for x in index["files"] if x["path"] in (name, name + ".exe")]
        require(len(matches) == 1, "EXACT_BINARY_INPUT_REQUIRED: " + role)
        binaries[role] = matches[0]
    require(set(binaries) == {"cli", "worker"}, "CLI_AND_WORKER_REQUIRED")
    categories = {"source": object_hash(rows),
                  "schema": subset_hash(rows, lambda p: p.startswith("schemas/")),
                  "template": subset_hash(rows, lambda p: p.startswith("templates/")),
                  "wit": subset_hash(rows, lambda p: p.startswith("wit/")),
                  "dependencyLocks": subset_hash(rows, lambda p: p.endswith(("Cargo.lock", "package-lock.json"))),
                  "toolchain": digest(no_links(root / "rust-toolchain.toml", True))}
    return {"root": str(root), "inventorySha256": byte_check["releaseFilesSha256"],
            "inventoryMetadata": {k: v for k, v in index.items() if k != "files"},
            "files": index["files"], "sources": rows, "digests": categories, "binaries": binaries}


def compare(a, b, exceptions):
    require(a["digests"] == b["digests"] and a["sources"] == b["sources"], "LOCKED_SOURCE_DRIFT")
    require(a["inventoryMetadata"] == b["inventoryMetadata"], "INVENTORY_METADATA_DRIFT")
    left = {x["path"]: x for x in a["files"]}; right = {x["path"]: x for x in b["files"]}
    require(set(left) == set(right), "CANDIDATE_FILE_SET_DRIFT")
    binary_paths = {item["path"] for item in a["binaries"].values()}
    require(binary_paths == {item["path"] for item in b["binaries"].values()}, "BINARY_NAME_DRIFT")
    require(isinstance(exceptions, list), "BINARY_DIFFERENCES_MUST_BE_LIST")
    declared = {}
    for item in exceptions:
        closed(item, ("path", "sha256A", "sha256B", "reason", "investigation"), "binary difference")
        name = relative(item["path"])
        require(name in binary_paths and name not in declared, "UNAPPROVED_BINARY_DIFFERENCE")
        require(isinstance(item["reason"], str) and bool(item["reason"].strip()), "BINARY_DIFFERENCE_REASON_REQUIRED")
        declared[name] = item
    actual = []
    for name in sorted(left):
        x, y = left[name], right[name]
        require({k: v for k, v in x.items() if k not in ("sha256", "bytes")} ==
                {k: v for k, v in y.items() if k not in ("sha256", "bytes")},
                "INVENTORY_FILE_METADATA_DRIFT")
        if (x["sha256"], x["bytes"]) == (y["sha256"], y["bytes"]):
            continue
        require(name in binary_paths, "NON_BINARY_BYTES_DRIFT: " + name)
        require(name in declared and declared[name]["sha256A"] == x["sha256"] and
                declared[name]["sha256B"] == y["sha256"], "UNRECORDED_BINARY_DIFFERENCE: " + name)
        actual.append(declared[name])
    require(set(declared) == {x["path"] for x in actual}, "STALE_BINARY_DIFFERENCE_DECLARATION")
    return actual


def verify_build(root, value, measured):
    closed(value, ("sourceRoot", "sourcesBefore", "sourcesAfter", "toolchain", "executions", "binaries"), "build")
    source = absolute(Path(value["sourceRoot"]))
    require(value["sourcesBefore"] == value["sourcesAfter"] == measured["sources"], "BUILD_LOCKED_INPUT_DRIFT")
    for row in measured["sources"]:
        require(digest(no_links(source / relative(row["path"]), True)) == row["sha256"], "BUILD_SOURCE_ROOT_DRIFT")
    toolchain = reference(root, value["toolchain"])
    require(bool(value["executions"]) and isinstance(value["executions"], list), "BUILD_NOT_EXECUTED")
    expected_outputs = {}
    require(set(value["binaries"]) == {"cli", "worker"}, "BUILD_BINARY_OUTPUTS_MISSING")
    for role, path in value["binaries"].items():
        p = absolute(Path(path), True)
        require(digest(p) == measured["binaries"][role]["sha256"], "BUILD_BINARY_NOT_EXPORTED")
        expected_outputs[os.path.normcase(str(p))] = digest(p)
    outputs = {}; commands = []; compilers = set()
    for ref in value["executions"]:
        record, inputs, produced = execution(root, ref, "BUILD")
        command = record["command"]
        require(Path(command[0]).stem.casefold() == "cargo" and command[1] == "build" and
                "--locked" in command and "--release" in command, "LOCKED_CARGO_BUILD_REQUIRED")
        require("--manifest-path" in command and command.count("--manifest-path") == 1,
                "EXPLICIT_BUILD_MANIFEST_REQUIRED")
        manifest_index = command.index("--manifest-path") + 1
        require(manifest_index < len(command) and absolute(Path(command[manifest_index]), True) == source / "Cargo.toml",
                "WRONG_BUILD_MANIFEST")
        expected_inputs = {os.path.normcase(str(source / row["path"])): row["sha256"] for row in measured["sources"]}
        require(all(inputs.get(k) == h for k, h in expected_inputs.items()), "INCOMPLETE_BUILD_INPUT_CLOSURE")
        outputs.update(produced); commands.append(ref)
        compilers.add(record["executableSha256"])
    require(all(outputs.get(k) == h for k, h in expected_outputs.items()), "BUILD_ARTIFACT_NOT_BOUND_TO_EXECUTION")
    return {"sourceRoot": str(source), "toolchainSha256": digest(toolchain), "executions": commands,
            "buildExecutableSha256": sorted(compilers), "binaries": expected_outputs}


def verify_gates(root, gates, source_digest):
    require(isinstance(gates, dict) and set(gates) == set(GATES), "G00_G05_REQUIRED_G06_IS_NOT_A_PRECONDITION")
    output = {}
    for gate, task_numbers in GATES.items():
        item = read_json(reference(root, gates[gate]))
        closed(item, ("schemaVersion", "kind", "gate", "status", "sourceDigest", "tasks", "taskEvidence", "executions"), "gate")
        require(item["schemaVersion"] == 1 and item["kind"] == "SDK_GATE_EVIDENCE_V1" and
                item["gate"] == gate and item["status"] == "VERIFIED", "GATE_NOT_VERIFIED: " + gate)
        require(item["sourceDigest"] == source_digest, "GATE_CANDIDATE_SOURCE_DRIFT")
        require(item["tasks"] == [f"SD{i:02d}" for i in task_numbers], "GATE_TASK_SET_MISMATCH")
        require(isinstance(item["taskEvidence"], dict) and set(item["taskEvidence"]) == set(item["tasks"]),
                "GATE_TASK_EXECUTION_EVIDENCE_MISSING")
        for task, ref in item["taskEvidence"].items():
            report = read_json(reference(root, ref))
            require(report.get("taskId") == task and report.get("status") == "VERIFIED" and
                    report.get("blockedReasons") == [], "TASK_NOT_VERIFIED: " + task)
            commands, codes = report.get("commands"), report.get("exitCodes")
            require(isinstance(commands, list) and bool(commands) and isinstance(codes, list) and
                    len(commands) == len(codes) and all(type(code) is int and code == 0 for code in codes),
                    "TASK_ACTUAL_EXIT_EVIDENCE_MISSING: " + task)
            require(all(isinstance(command, list) and command and
                        all(isinstance(arg, str) and arg for arg in command) for command in commands),
                    "TASK_ACTUAL_COMMAND_MISSING: " + task)
            latest = report.get("latestAttempt", {})
            require(latest.get("taskId") == task and type(latest.get("exitCode")) is int and
                    latest["exitCode"] == 0 and latest.get("command") in commands,
                    "TASK_LATEST_ATTEMPT_NOT_SUCCESSFUL: " + task)
            log = relative(latest.get("log"))
            hashes = report.get("artifactHashes", {})
            require(isinstance(hashes, dict) and log in hashes, "TASK_ACTUAL_LOG_HASH_MISSING: " + task)
            reference(root, {"path": log, "sha256": hashes[log]})
        require(isinstance(item["executions"], list) and bool(item["executions"]), "GATE_EXECUTIONS_MISSING")
        for ref in item["executions"]:
            execution(root, ref, "GATE")
        output[gate] = gates[gate]
    return output


def verify_tests(root, item, measured, artifact_roots=None):
    closed(item, ("releaseFilesSha256", "executions", "conformanceRoot"), "candidate tests")
    require(item["releaseFilesSha256"] == measured["inventorySha256"], "TESTED_CANDIDATE_INVENTORY_DRIFT")
    require(isinstance(item["executions"], list) and bool(item["executions"]), "CANDIDATE_TESTS_NOT_EXECUTED")
    seen = set()
    for ref in item["executions"]:
        record, inputs, _ = execution(root, ref, "CANDIDATE_TEST")
        for role, binary in measured["binaries"].items():
            path = os.path.normcase(str(Path(measured["root"]) / binary["path"]))
            if inputs.get(path) == binary["sha256"]:
                seen.add(role)
        index = os.path.normcase(str(Path(measured["root"]) / "release-files.json"))
        require(inputs.get(index) == measured["inventorySha256"], "TEST_RECEIPT_NOT_BOUND_TO_CANDIDATE")
    require(seen == {"cli", "worker"}, "CANDIDATE_BINARIES_NOT_IN_TEST_CLOSURE")
    evidence = no_links(root / relative(item["conformanceRoot"]))
    # Execute only the trusted shared evaluator, never candidate-provided Python.
    runner = module("sdk_release_conformance", TOOLS / "run_conformance.py")
    runner.SDK = Path(measured["root"])
    require(digest(TOOLS / "run_conformance.py") == digest(runner.SDK / "tools/run_conformance.py"),
            "CONFORMANCE_EVALUATOR_SOURCE_DRIFT")
    result = runner.evaluate(evidence, runner.load_definitions()[0], artifact_roots)
    require(result.get("globalPassed") is True and len(result.get("cases", [])) == 36 and
            all(row.get("required") is True and row.get("status") == "PASS" for row in result["cases"]),
            "CANDIDATE_CONFORMANCE_NOT_COMPLETE_C01_C36")
    return {"inventorySha256": measured["inventorySha256"], "executions": item["executions"],
            "conformanceSha256": object_hash(result)}


def assess(a_root, b_root, evidence_root, evidence_path, reproduction_only=False, artifact_roots=None):
    root = absolute(evidence_root)
    evidence = read_json(absolute(evidence_path, True))
    closed(evidence, ("schemaVersion", "sdkRelease", "builds", "binaryDifferences", "gates", "candidateTests"), "release evidence")
    require(evidence["schemaVersion"] == 1, "RELEASE_EVIDENCE_SCHEMA_VERSION_DRIFT")
    release_version(evidence["sdkRelease"])
    a, b = candidate(a_root), candidate(b_root)
    require(Path(a["root"]) != Path(b["root"]) and not Path(a["root"]).is_relative_to(Path(b["root"])) and
            not Path(b["root"]).is_relative_to(Path(a["root"])), "TWO_INDEPENDENT_CANDIDATE_DIRECTORIES_REQUIRED")
    differences = compare(a, b, evidence["binaryDifferences"])
    for item in differences:
        reference(root, item["investigation"])
    require(isinstance(evidence["builds"], list) and len(evidence["builds"]) == 2, "TWO_ACTUAL_BUILDS_REQUIRED")
    builds = [verify_build(root, item, measured) for item, measured in zip(evidence["builds"], (a, b))]
    build_roots = [Path(x["sourceRoot"]) for x in builds]
    require(not build_roots[0].is_relative_to(build_roots[1]) and
            not build_roots[1].is_relative_to(build_roots[0]), "TWO_INDEPENDENT_BUILD_ROOTS_REQUIRED")
    require(builds[0]["toolchainSha256"] == builds[1]["toolchainSha256"], "TOOLCHAIN_DRIFT")
    require(builds[0]["buildExecutableSha256"] == builds[1]["buildExecutableSha256"], "BUILD_EXECUTABLE_DRIFT")
    for role in ("cli", "worker"):
        require(Path(evidence["builds"][0]["binaries"][role]) != Path(evidence["builds"][1]["binaries"][role]),
                "TWO_INDEPENDENT_BUILD_OUTPUTS_REQUIRED")
    proof = {"schemaVersion": 1, "kind": "SDK_REPRODUCTION_PROOF_V1", "sdkRelease": VERSION,
             "status": "MEASURED_REPRODUCTION_PROOF", "sdkReady": False,
             "sourceDigests": a["digests"], "builds": builds,
             "candidateInventories": [a["inventorySha256"], b["inventorySha256"]],
             "byteIdentical": not differences, "binaryDifferences": differences,
             "evidenceSha256": digest(evidence_path),
             "trust": "Explicit unkeyed local execution evidence; no cryptographic attestation or release signature.",
             "G06": "NOT_RUN", "moduleIntegration": "NOT_ADOPTED"}
    if reproduction_only:
        return {"reproduction-proof.json": proof}
    gates = verify_gates(root, evidence["gates"], a["digests"]["source"])
    require(isinstance(evidence["candidateTests"], list) and len(evidence["candidateTests"]) == 2,
            "BOTH_CANDIDATES_REQUIRE_OWN_TESTS")
    tests = [verify_tests(root, item, measured, artifact_roots) for item, measured in zip(evidence["candidateTests"], (a, b))]
    lock = {**proof, "kind": "SDK_RELEASE_LOCK_V1", "status": "RELEASE_CANDIDATE",
            "sealed": True, "gates": gates, "candidateTests": tests,
            "inventories": [{"releaseFilesSha256": x["inventorySha256"], "files": x["files"]} for x in (a, b)]}
    status = {"schemaVersion": 1, "sdkRelease": VERSION, "status": "RELEASE_CANDIDATE",
              "sealed": True, "sdkReady": False, "releaseLockDigest": hashlib.sha256(exporter.json_bytes(lock)).hexdigest(),
              "gates": {**{g: "VERIFIED" for g in GATES}, "G06": "NOT_RUN"},
              "moduleIntegration": "NOT_ADOPTED", "mainApplicationV2Import": "NOT_SUPPORTED",
              "publication": "NOT_PUBLISHED", "projectLicense": "LicenseRef-Proprietary",
              "publicationEligibility": "NOT_CLEARED_OWNER_TERMS_AND_NOTICE_REVIEW_SEPARATE",
              "next": "SD32 must verify its own completed evidence and G00-G06 before SDK_READY."}
    return {"release-lock.json": lock, "RELEASE-STATUS.json": status}


def write_outputs(out, values, inputs):
    require(out.is_absolute(), "ABSOLUTE_NEW_OUTPUT_REQUIRED")
    no_links(out.parent)
    require(not out.exists() and not out.is_symlink(), "OUTPUT_ALREADY_EXISTS")
    require(all(not out.is_relative_to(p) and not p.is_relative_to(out) for p in inputs),
            "OUTPUT_MUST_BE_SEPARATE_FROM_INPUTS")
    out.mkdir(exist_ok=False)
    for name, value in values.items():
        exporter.new_file(out, name, raw=exporter.json_bytes(value))
    return {"status": next(iter(values.values()))["status"], "sdkReady": False,
            "output": str(out), "files": [{"path": name, "sha256": digest(out / name)} for name in values]}


def self_test():
    checks = []
    with tempfile.TemporaryDirectory(prefix="bgx-sdk-release-selftest-") as temp:
        root = absolute(Path(temp))
        source = root / "fixture.txt"; source.write_bytes(b"fixture inputs, not SDK source")
        exe = absolute(Path(sys.executable), True)
        started = time.monotonic()
        completed = subprocess.run([str(exe), "-I", "-c", "print(7)"], cwd=root,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
                                   creationflags=0x08000000 if os.name == "nt" else 0)
        (root / "stdout.log").write_bytes(completed.stdout)
        (root / "stderr.log").write_bytes(completed.stderr)
        def ref(name): return {"path": name, "sha256": digest(root / name)}
        inputs = [{"path": str(source), "sha256": digest(source)}]
        receipt = {"schemaVersion": 1, "kind": "SDK_RELEASE_EXECUTION_V1", "scope": "SDK_RELEASE",
                   "role": "GATE", "command": [str(exe), "-I", "-c", "print(7)"], "cwd": str(root),
                   "executableSha256": digest(exe), "inputsBefore": inputs, "inputsAfter": inputs,
                   "outputs": [], "stdout": ref("stdout.log"), "stderr": ref("stderr.log"),
                   "exitCode": completed.returncode, "timedOut": False,
                   "durationSeconds": time.monotonic() - started}
        def save(): (root / "execution.json").write_bytes(exporter.json_bytes(receipt))
        save(); execution(root, ref("execution.json"), "GATE")
        checks.append("actual_fixture_process_exit_logs_and_input_hashes_bound")
        def reject(name, action):
            try: action()
            except (exporter.ExportError, OSError, KeyError, TypeError, ValueError): checks.append(name)
            else: raise exporter.ExportError("SELF_TEST_FALSE_ACCEPT: " + name)
        reject("missing_gates_rejected", lambda: verify_gates(root, {}, "0" * 64))
        reject("release_version_drift_rejected", lambda: release_version("2.0.1"))
        reject("candidate_tests_not_executed_rejected", lambda: verify_tests(root,
            {"releaseFilesSha256": "0"*64, "executions": [], "conformanceRoot": "empty"},
            {"inventorySha256": "0"*64}))
        receipt["exitCode"] = 7; save()
        reject("nonzero_actual_exit_rejected", lambda: execution(root, ref("execution.json"), "GATE"))
        receipt["exitCode"] = 0; receipt["durationSeconds"] = 0; save()
        reject("not_executed_receipt_rejected", lambda: execution(root, ref("execution.json"), "GATE"))
        receipt["durationSeconds"] = 0.01; save()
        (root / "stdout.log").write_bytes(b"changed")
        reject("actual_log_drift_rejected", lambda: execution(root, ref("execution.json"), "GATE"))
        (root / "stdout.log").write_bytes(completed.stdout)
        source.write_bytes(b"changed")
        reject("source_hash_drift_rejected", lambda: execution(root, ref("execution.json"), "GATE"))
        base = {"sources": inputs, "digests": {"source": "a"}, "inventoryMetadata": {}, "files": [
            {"path": "src/a.rs", "sha256": "0" * 64, "bytes": 1},
            {"path": "bin/cli", "sha256": "1" * 64, "bytes": 2}],
            "binaries": {"cli": {"path": "bin/cli"}}}
        other = json.loads(json.dumps(base)); compare(base, other, [])
        checks.append("equal_fixture_inventory_comparison")
        other["digests"]["source"] = "b"
        reject("locked_input_drift_rejected", lambda: compare(base, other, []))
        other = json.loads(json.dumps(base)); other["files"][0]["sha256"] = "f" * 64
        reject("nonbinary_drift_rejected", lambda: compare(base, other, []))
        other = json.loads(json.dumps(base)); other["files"][1]["sha256"] = "f" * 64
        reject("unrecorded_binary_difference_rejected", lambda: compare(base, other, []))
        diff = [{"path": "bin/cli", "sha256A": "1" * 64, "sha256B": "f" * 64,
                 "reason": "controlled fixture byte difference", "investigation": ref("execution.json")}]
        require(len(compare(base, other, diff)) == 1, "BINARY_DIFF_RECORD_LOST")
        checks.append("binary_difference_recorded_without_claiming_equality")
        out = root / "new"; write_outputs(out, {"proof.json": {"status": "FIXTURE_ONLY"}}, [source])
        before = digest(out / "proof.json")
        reject("existing_output_rejected", lambda: write_outputs(out, {}, [source]))
        require(before == digest(out / "proof.json"), "EXISTING_OUTPUT_CHANGED")
        reject("path_escape_rejected", lambda: reference(root, {"path": "../private.json", "sha256": "0" * 64}))
    return {"schemaVersion": 1, "kind": "RELEASE_VERIFIER_SELF_TEST", "phaseSelfTestPassed": True,
            "scope": "FIXTURE_INFRASTRUCTURE_ONLY", "actualSdkBuilds": "NOT_RUN",
            "releaseStatus": "NOT_RUN", "sdkReady": False, "checks": checks}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--repro-proof", action="store_true")
    parser.add_argument("--artifact-root", action="append", default=[], metavar="NAME=ABS",
                        help="Explicitly authorize an external retained conformance artifact root; never discovered from proof JSON")
    for name in ("candidate-a", "candidate-b", "evidence-root", "evidence", "out"):
        parser.add_argument("--" + name, type=Path)
    args = parser.parse_args()
    try:
        values = [args.candidate_a, args.candidate_b, args.evidence_root, args.evidence, args.out]
        if args.self_test:
            require(not any(values) and not args.repro_proof and not args.artifact_root, "SELF_TEST_TAKES_NO_EXTERNAL_INPUTS")
            result = self_test()
        else:
            require(all(values), "TWO_CANDIDATES_EXPLICIT_EVIDENCE_AND_NEW_OUTPUT_REQUIRED")
            require(all(path.is_absolute() for path in values), "EXPLICIT_ABSOLUTE_PATHS_REQUIRED")
            artifact_roots = {}
            for item in args.artifact_root:
                name, path = item.split("=", 1)
                require(name and name != "evidence" and name not in artifact_roots, "DUPLICATE_ARTIFACT_ROOT")
                artifact_roots[name] = absolute(Path(path))
            result = assess(*values[:4], reproduction_only=args.repro_proof, artifact_roots=artifact_roots)
            result = write_outputs(args.out, result, [args.candidate_a, args.candidate_b,
                                                     args.evidence])
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (exporter.ExportError, OSError, KeyError, TypeError, ValueError, IndexError) as error:
        print("RELEASE_VERIFICATION_FAILED: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
