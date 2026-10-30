#!/usr/bin/env python3
"""Public SDK evidence runner. Default full acceptance fails closed on missing proof.

--self-test executes infrastructure tests only. It never certifies C01-C36.
--execute-suite executes one fixed public suite and retains structured execution
metadata. Imported suite evidence remains explicitly labelled; historical task
status and test names in stdout are never interpreted as runtime assertions.
Joined chains use fixed structured artifact validators. Missing real captures stay
NOT_RUN. --self-test verifies infrastructure only and never closes a global case.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

SDK = Path(__file__).resolve().parents[1]
CASE_IDS = tuple(f"C{i:02d}" for i in range(1, 37))
MAX_JSON = 16 * 1024 * 1024
SUFFIXES = {".rs", ".toml", ".lock", ".json", ".wit", ".ts", ".py", ".html"}
SKIP = {"target", "node_modules", "dist", ".git", ".bgx-sdk", "__pycache__"}


def joined_module():
    sys.dont_write_bytecode = True
    path = Path(__file__).with_name("conformance_proofs.py")
    spec = importlib.util.spec_from_file_location("sdk_joined_proofs", path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


class Invalid(ValueError):
    pass


def require(ok, message):
    if not ok:
        raise Invalid(message)


def closed(value, fields, name):
    require(isinstance(value, dict), f"{name}: expected object")
    require(set(value) == set(fields), f"{name}: missing or unknown fields")
    return value


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def read_json(path):
    require(path.is_file(), f"Missing JSON file: {path}")
    require(path.stat().st_size <= MAX_JSON, "JSON byte budget exceeded")
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise Invalid(f"Invalid JSON: {path.name}") from error


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def object_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def no_links(path):
    """Check lexical ancestors before resolve; Windows junctions are reparse points."""
    require(path.is_absolute(), "Path must be explicit and absolute")
    for item in [path, *path.parents]:
        try:
            info = item.lstat()
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(info.st_mode), "Symlink is forbidden")
        require(not (getattr(info, "st_file_attributes", 0) & 0x400), "Reparse point is forbidden")


def inside(root, relative, must_exist=True):
    require(isinstance(relative, str) and bool(relative), "Relative path required")
    p = PurePosixPath(relative)
    require(not p.is_absolute() and ".." not in p.parts and "\\" not in relative and ":" not in relative, "Unsafe relative path")
    result = root.joinpath(*p.parts)
    no_links(result)
    require(result.resolve().is_relative_to(root.resolve()), "Path escaped evidence root")
    if must_exist:
        require(result.is_file(), f"Missing evidence file: {relative}")
    return result


def claim_json(path, value):
    no_links(path)
    require(path.suffix == ".json", "Output must be a new JSON file")
    path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive create, including a dangling-link/reparse refusal above. No existing
    # receipt is ever replaced; interruption leaves a visibly incomplete file.
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def workspace(path, create=False):
    require(path.is_absolute(), "Evidence root must be absolute")
    no_links(path)
    marker = path / ".bgx-conformance-workspace.json"
    if create:
        require(not path.exists() or (path.is_dir() and not any(path.iterdir())), "New evidence root must be absent or empty")
        path.mkdir(parents=True, exist_ok=True)
        claim_json(marker, {"schemaVersion": 1, "purpose": "SDK_CONFORMANCE", "workspaceId": str(uuid.uuid4())})
    require(path.is_dir(), "Evidence root does not exist; use --create-evidence-root")
    no_links(marker)
    m = closed(read_json(marker), ["schemaVersion", "purpose", "workspaceId"], "workspace marker")
    require(m["schemaVersion"] == 1 and m["purpose"] == "SDK_CONFORMANCE", "Wrong workspace purpose")
    uuid.UUID(m["workspaceId"])
    return path


def load_definitions():
    catalog = read_json(SDK / "protocol/conformance-cases.json")
    require(tuple(c["id"] for c in catalog["cases"]) == CASE_IDS, "Public catalog must contain exactly C01-C36 in order")
    require(all(c["required"] is True for c in catalog["cases"]), "Public required case cannot be removed")
    cases = read_json(SDK / "fixtures/conformance/cases.json")
    expected = read_json(SDK / "fixtures/conformance/expected.json")
    closed(cases, ["schemaVersion", "catalog", "cases", "suites", "chains"], "case definitions")
    closed(expected, ["schemaVersion", "requiredCaseIds", "required", "executionModes", "globalPassRequires", "phaseSelfTestScope", "execution", "proofs", "runnerSelfTest"], "expectations")
    require(cases["schemaVersion"] == expected["schemaVersion"] == 1, "Unknown definitions version")
    require(cases["catalog"] == "protocol/conformance-cases.json", "Catalog path is fixed")
    require(expected["requiredCaseIds"] == list(CASE_IDS) and expected["required"] is True, "Required set is fixed")
    require(expected["executionModes"] == ["MOCK"], "This runner has no LIVE authorization")
    require(tuple(c["id"] for c in cases["cases"]) == CASE_IDS, "Case list is closed and ordered")
    chain_ids = [c["id"] for c in cases["chains"]]
    require(len(chain_ids) == len(set(chain_ids)), "Duplicate chain ID")
    for c, public in zip(cases["cases"], catalog["cases"]):
        closed(c, ["id", "title", "required", "prerequisites", "inputs", "faults", "expected", "cleanup", "evidence", "scope", "lateCase"], c["id"])
        require(c["title"] == public["title"] and c["required"] is True and c["scope"] == "MOCK", "Changed public case identity")
        closed(c["evidence"], ["suites", "chains"], "case evidence requirements")
        require(len(c["evidence"]["suites"]) == len(set(c["evidence"]["suites"])), "Duplicate required suite")
        require(set(c["evidence"]["suites"]) <= set(cases["suites"]), "Unknown required suite")
        require(set(c["evidence"]["chains"]) <= set(chain_ids), "Unknown required chain")
        require(c["evidence"]["suites"] or c["evidence"]["chains"], "Empty acceptance requirements")
    for suite in cases["suites"].values():
        closed(suite, ["command", "cwd", "sourceDirectories", "sourceFiles", "scope"], "suite")
        require(suite["scope"] == "TASK_LOCAL", "Suite alone cannot claim global runtime coverage")
        require(isinstance(suite["command"], list) and all(isinstance(x, str) and x for x in suite["command"]), "Invalid argv")
    for chain in cases["chains"]:
        closed(chain, ["id", "cases", "implementation", "status", "requiredArtifacts", "acceptance", "commandTemplates", "note"], "chain")
        require(chain["implementation"] == "STRUCTURED_PROOF_V1" and chain["status"] == "NOT_RUN", "Definitions cannot claim runtime success")
        require(set(chain["cases"]) <= set(CASE_IDS), "Unknown chain case")
    closed(expected["runnerSelfTest"], ["positiveJson", "mutatedJson", "faultExitCode"], "runner self-test expectations")
    closed(expected["proofs"], ["suiteKinds", "suiteScope", "caseProof", "chainPolicy"], "proof expectations")
    limits = closed(expected["execution"], ["timeoutSeconds", "maxLogBytes"], "execution limits")
    require(type(limits["timeoutSeconds"]) is int and 1 <= limits["timeoutSeconds"] <= 3600, "Invalid timeout budget")
    require(type(limits["maxLogBytes"]) is int and 1024 <= limits["maxLogBytes"] <= MAX_JSON, "Invalid log budget")
    return cases, expected


def sources(suite):
    paths = set(suite["sourceFiles"])
    # Literal include inputs belong to the fixed target definition. Unrelated
    # example guests/caches are not compiled by every Rust integration target.
    # Compiled/public-input closure only: new evaluator source is not retroactively
    # claimed as a compiled dependency of a historical imported suite execution.
    for relative in suite["sourceDirectories"]:
        parts = PurePosixPath(relative).parts
        if len(parts) >= 2 and parts[0] == "crates":
            paths.add(f"crates/{parts[1]}/Cargo.toml")
        directory = inside(SDK, relative, False)
        require(directory.is_dir(), f"Missing source directory: {relative}")
        for current, dirs, files in os.walk(directory, followlinks=False):
            dirs[:] = sorted(d for d in dirs if d not in SKIP)
            for name in dirs:
                no_links(Path(current) / name)
            for name in files:
                p = Path(current) / name
                if p.suffix in SUFFIXES and p.relative_to(SDK).as_posix() != "schemas/adoption-evidence.schema.json":
                    paths.add(p.relative_to(SDK).as_posix())
    paths.discard("schemas/adoption-evidence.schema.json")
    return [{"path": name, "sha256": digest(inside(SDK, name))} for name in sorted(paths)]


def expand(suite, cargo=None, node=None):
    values = {"{sdk}": str(SDK), "{cargo}": str(cargo) if cargo else "{cargo}", "{node}": str(node) if node else "{node}"}
    def sub(value):
        for key, replacement in values.items():
            value = value.replace(key, replacement)
        require(not any(key in value for key in values), "Explicit executable path missing")
        return value
    command = [sub(s) for s in suite["command"]]
    executable = Path(command[0])
    no_links(executable)
    require(executable.is_file(), "Explicit suite executable missing")
    return command, Path(sub(suite["cwd"]))


def stop_process(process):
    if process.poll() is not None:
        return
    if os.name == "nt":
        system_root = Path(os.environ["SystemRoot"])
        subprocess.run([str(system_root / "System32/taskkill.exe"), "/PID", str(process.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW, timeout=15, check=False)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.kill() if process.poll() is None else None
    process.wait(timeout=15)


def execute(command, cwd, folder, timeout, budget):
    """No shell. Bounded logs, real exit code, kill/reap on timeout/overflow."""
    folder.mkdir(parents=True, exist_ok=False)
    no_links(folder)
    flags = subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
    started = time.time()
    overflow = threading.Event()
    errors = []
    with (folder / "stdout.log").open("xb") as out, (folder / "stderr.log").open("xb") as err:
        process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False, creationflags=flags, start_new_session=os.name != "nt")
        def drain(pipe, target):
            try:
                count = 0
                while True:
                    chunk = pipe.read(65536)
                    if not chunk:
                        break
                    count += len(chunk)
                    if count > budget:
                        overflow.set()
                        break
                    target.write(chunk)
                target.flush()
                os.fsync(target.fileno())
            except Exception as error:
                errors.append(type(error).__name__)
                overflow.set()
            finally:
                pipe.close()
        threads = [threading.Thread(target=drain, args=(process.stdout, out), daemon=True), threading.Thread(target=drain, args=(process.stderr, err), daemon=True)]
        for t in threads:
            t.start()
        timed_out = False
        try:
            while process.poll() is None:
                if overflow.is_set() or time.time() - started > timeout:
                    timed_out = not overflow.is_set()
                    stop_process(process)
                    break
                time.sleep(0.02)
            process.wait(timeout=15)
        finally:
            if process.poll() is None:
                stop_process(process)
            for t in threads:
                t.join(timeout=15)
        require(not any(t.is_alive() for t in threads), "Subprocess pipe did not close")
    return {"exitCode": process.returncode, "timedOut": timed_out, "logBudgetExceeded": overflow.is_set(), "captureErrors": errors, "durationSeconds": round(time.time() - started, 6), "stdoutSha256": digest(folder / "stdout.log"), "stderrSha256": digest(folder / "stderr.log")}


def run_suite(root, suite_id, suite, limits, cargo, node):
    command, cwd = expand(suite, cargo, node)
    before = sources(suite)
    executable_before = digest(Path(command[0]))
    relative = f"executions/{uuid.uuid4()}"
    folder = inside(root, relative, False)
    actual = execute(command, cwd, folder, limits["timeoutSeconds"], limits["maxLogBytes"])
    try:
        after = sources(suite)
        executable_after = digest(Path(command[0]))
    except (Invalid, OSError):
        # Preserve the real command exit/logs even when a command deleted or
        # changed its source closure. An incomplete snapshot always fails.
        after, executable_after = [], None
        actual["captureErrors"].append("SOURCE_OR_EXECUTABLE_SNAPSHOT_FAILED")
    result = {"schemaVersion": 1, "kind": "EXECUTED_SUITE", "suiteId": suite_id, "mode": "MOCK", "scope": "TASK_LOCAL", "command": command, "cwd": str(cwd), "executableSha256": executable_after, "sourcesBefore": before, "sourcesAfter": after, "sourceDigest": object_digest(before), "stdout": {"path": relative + "/stdout.log", "sha256": actual.pop("stdoutSha256")}, "stderr": {"path": relative + "/stderr.log", "sha256": actual.pop("stderrSha256")}, **actual}
    result["sourceUnchanged"] = before == after and executable_before == result["executableSha256"]
    claim_json(folder / "execution.json", result)
    return {"path": relative + "/execution.json", "sha256": digest(folder / "execution.json")}, result


def file_ref(root, value):
    closed(value, ["path", "sha256"], "file reference")
    require(isinstance(value["sha256"], str) and len(value["sha256"]) == 64 and all(c in "0123456789abcdef" for c in value["sha256"]), "Invalid SHA256")
    path = inside(root, value["path"])
    require(digest(path) == value["sha256"], "Evidence file hash mismatch")
    return path


def typed_suite_command(command, cwd, sdk_at_execution, suite):
    """Closed target mapping, never test names or success text from stdout."""
    require(isinstance(command, list) and len(command) > 1 and all(isinstance(x, str) and x for x in command), "Missing original argv")
    require(isinstance(sdk_at_execution, str) and Path(sdk_at_execution).is_absolute(), "Missing original SDK location")
    expected = suite["command"]
    if expected[0] == "{node}":
        suffix = "tools/form-reference/tests/forms.test.ts"
        if Path(command[0]).stem == "node":
            require(command[1:] == ["--test", str(Path(sdk_at_execution) / suffix)], "Different Node test target")
        else:
            require(Path(command[0]).name.lower() == "cmd.exe" and command[1:] == ["/D", "/C", "npm", "--prefix",
                    str(Path(sdk_at_execution) / "tools/form-reference"), "run", "test"], "Different npm test script")
            require(read_json(SDK / "tools/form-reference/package.json")["scripts"]["test"] == "node --test tests/forms.test.ts", "Unknown npm test expansion")
        return {"kind": "NODE_TEST_TARGET", "target": suffix}
    require(Path(command[0]).stem == "cargo" and command[1] == "test", "Historical execution is not cargo test")
    values, switches = {}, set(); index = 2
    while index < len(command):
        item = command[index]
        if item == "--":
            require(all(x == "--nocapture" or re.fullmatch(r"--test-threads=[1-9][0-9]*", x) for x in command[index + 1:]), "Filtered/unexecuted historical target")
            break
        if item in ("--locked", "--offline"):
            require(item not in switches, "Repeated Cargo switch"); switches.add(item); index += 1; continue
        if item.startswith("--message-format="):
            require(item in ("--message-format=json", "--message-format=json-render-diagnostics"), "Unknown Cargo message format")
            index += 1; continue
        require(item in ("--manifest-path", "-p", "--package", "--test") and index + 1 < len(command), "Unknown/filtering Cargo argument")
        key = "-p" if item == "--package" else item
        if key == "--test":
            require(command[index + 1] not in values.setdefault(key, []), "Repeated Cargo integration target")
            values[key].append(command[index + 1])
        else:
            require(key not in values, "Repeated Cargo option"); values[key] = command[index + 1]
        index += 2
    require("--locked" in switches and "--manifest-path" in values and "-p" in values, "Missing locked explicit Cargo target")
    manifest = Path(values["--manifest-path"])
    if not manifest.is_absolute():
        require(isinstance(cwd, str) and Path(cwd).is_absolute(), "Relative original manifest has no measured cwd")
        manifest = Path(cwd) / manifest
    require(os.path.normcase(os.path.normpath(manifest)) == os.path.normcase(os.path.normpath(Path(sdk_at_execution) / "Cargo.toml")), "Cargo manifest is not original public SDK")
    package = expected[expected.index("-p") + 1]; target = expected[expected.index("--test") + 1]
    require(values["-p"] == package and target in values.get("--test", [target]), "Cargo package/integration target differs")
    return {"kind": "CARGO_INTEGRATION_TARGET", "package": package, "target": target,
            "selection": "EXPLICIT_TARGET" if "--test" in values else "UNFILTERED_PACKAGE_ALL_TARGETS"}


def checkpoint_sources(context, selector, needed, bindings):
    """Verify retained Git commit/tree/blob objects without invoking Git."""
    proof = closed(context.select(selector), ("kind", "commit", "commitObject", "trees", "files"), "checkpoint source proof")
    require(proof["kind"] == "REVIEWED_GIT_CHECKPOINT_V1" and re.fullmatch(r"[0-9a-f]{40}", proof["commit"]), "Invalid Git checkpoint identity")
    def payload(reference, kind, oid):
        data = context.file(reference).read_bytes()
        require(hashlib.sha1(kind.encode() + b" " + str(len(data)).encode() + b"\0" + data).hexdigest() == oid, "Git object identity mismatch")
        return data
    commit = payload(proof["commitObject"], "commit", proof["commit"])
    first = commit.split(b"\n", 1)[0]
    require(re.fullmatch(b"tree [0-9a-f]{40}", first), "Invalid commit root tree")
    root_tree = first[5:].decode("ascii"); trees = {}
    for row in proof["trees"]:
        closed(row, ("oid", "file"), "Git tree")
        require(row["oid"] not in trees, "Duplicate Git tree")
        raw = payload(row["file"], "tree", row["oid"]); offset = 0; entries = {}
        while offset < len(raw):
            end = raw.index(b"\0", offset); mode, name = raw[offset:end].split(b" ", 1)
            require(end + 21 <= len(raw), "Truncated Git tree")
            name = name.decode("utf-8"); require(name not in entries and "/" not in name, "Invalid Git tree entry")
            entries[name] = (mode.decode("ascii"), raw[end + 1:end + 21].hex()); offset = end + 21
        trees[row["oid"]] = entries
    safe_relative = joined_module().safe_relative
    def find(name):
        safe_relative(name)
        tree = root_tree; parts = name.split("/")
        for part in parts[:-1]:
            require(tree in trees and part in trees[tree], "Missing checkpoint ancestor tree")
            mode, tree = trees[tree][part]; require(mode == "40000", "Git path ancestor is not a tree")
        require(tree in trees and parts[-1] in trees[tree], "Missing checkpoint file")
        mode, oid = trees[tree][parts[-1]]; require(mode in ("100644", "100755"), "Linked/nonregular checkpoint source")
        return oid
    files = {}
    for row in proof["files"]:
        closed(row, ("path", "sha256", "oid", "file", "checkoutEncoding", "checkoutSha256"), "Git source file")
        require(row["path"] not in files and row["oid"] == find(row["path"]), "Duplicate/unbound checkpoint source")
        raw = payload(row["file"], "blob", row["oid"])
        require(hashlib.sha256(raw).hexdigest() == row["sha256"], "Checkpoint source SHA256 mismatch")
        require(row["checkoutEncoding"] in ("IDENTITY", "LF_TO_CRLF"), "Unknown Git checkout encoding")
        if row["checkoutEncoding"] == "LF_TO_CRLF":
            require(b"\r" not in raw and b"\0" not in raw, "LF checkout transform requires plain LF text")
            raw = raw.replace(b"\n", b"\r\n")
        require(hashlib.sha256(raw).hexdigest() == row["checkoutSha256"], "Checkout encoding/hash mismatch")
        files[row["path"]] = row["checkoutSha256"]
    for row in needed:
        require(files.get("packages/vendor-adapter-contract/v2/" + row["path"]) == row["sha256"], "Current compiled source differs from checkpoint: " + row["path"])
    for binding in bindings:
        require(files.get(binding["path"]) == binding["sha256"], "Original execution/source report is not bound to checkpoint")
    return proof["commit"]


def verify_reviewed_suite(root, reference, proof, suite_id, suite, artifact_roots):
    fields = ("schemaVersion", "kind", "suiteId", "mode", "scope", "execution", "originalCommand", "originalCwd",
              "sdkAtExecution", "executableSha256", "executable", "binaryMeasurement", "sourceReview")
    closed(proof, fields, "reviewed suite proof")
    require(proof["schemaVersion"] == 1 and proof["suiteId"] == suite_id and proof["mode"] == "MOCK" and proof["scope"] == "TASK_LOCAL", "Wrong reviewed suite binding")
    joined = joined_module(); context = joined.Context(root, SDK, artifact_roots)
    original = context.select(proof["execution"])
    command = original.get("command", original.get("argv"))
    require(proof["originalCommand"] == command and proof["originalCwd"] == original.get("cwd"), "Original command/cwd was rewritten")
    context.executions = [{"record": proof["execution"], "expectedExitCode": 0}]
    context.records()
    direct = original.get("kind") == "ACTUAL_REUSED_TEST_BINARY_CURRENT_CLI_REGRESSION"
    if direct:
        require(suite_id == "author" and suite["command"][suite["command"].index("--test") + 1] == "author" and
                re.fullmatch(r"author-[0-9a-f]{16}", Path(command[0]).stem) and command[1:] == ["--nocapture"], "Unknown retained direct test target")
        require(original["inputsBefore"] == original["inputsAfter"] and
                original["inputsBefore"].get(command[0]) == proof["executableSha256"], "Retained test binary was not measured unchanged")
        for name, expected in original["inputsBefore"].items():
            if name == command[0]: continue
            context.legacy(name, expected)
        target = {"kind": "RETAINED_AUTHOR_TEST_EXECUTION", "target": "author", "externalRuntime": "Captured current CLI and worker"}
    else:
        target = typed_suite_command(command, proof["originalCwd"], proof["sdkAtExecution"], suite)
    if proof["binaryMeasurement"] == "UNMEASURED":
        require(proof["executableSha256"] is proof["executable"] is None and original.get("executableSha256") is None, "Unknown binary hash was invented")
    else:
        require(proof["binaryMeasurement"] == "CAPTURED" and (direct or proof["executableSha256"] == original.get("executableSha256"))
                and proof["executable"]["sha256"] == proof["executableSha256"], "Measured executable identity differs")
        context.file(proof["executable"])
    review = closed(context.select(proof["sourceReview"]), ("kind", "sources", "sourceDigest", "capturedSource", "checkpoint", "bindings", "unmeasuredAtExecution", "analysis"), "suite source review")
    now = sources(suite)
    require(review["kind"] == "SUITE_SOURCE_REVIEW_V1" and review["sources"] == now and review["sourceDigest"] == object_digest(now), "Reviewed compiled source closure changed")
    captured = context.select(review["capturedSource"])
    require(isinstance(captured, dict) and bool(captured), "Original measured source map missing")
    prefix = "packages/vendor-adapter-contract/v2/"
    current = {row["path"]: row["sha256"] for row in now}; captured_names = set()
    for name, measured in captured.items():
        absolute = joined.ordinary_path(name)
        if absolute.is_absolute():
            at_execution = joined.ordinary_path(proof["sdkAtExecution"])
            if not absolute.is_relative_to(at_execution): continue
            name = absolute.relative_to(at_execution).as_posix()
        else: name = name.replace("\\", "/")
        if name.startswith(prefix): name = name[len(prefix):]
        if name in current:
            require(current[name] == measured, "Measured compiled source changed: " + name); captured_names.add(name)
    require(bool(captured_names), "No compiled input was measured in original report")
    require(isinstance(review["analysis"], str) and bool(review["analysis"].strip()) and isinstance(review["unmeasuredAtExecution"], list)
            and bool(review["unmeasuredAtExecution"]), "Historical measurement limits must remain explicit")
    bindings = review["bindings"]
    require(isinstance(bindings, list) and len(bindings) == 2, "Execution and captured source report checkpoint bindings required")
    for selector, binding in zip((proof["execution"], review["capturedSource"]), bindings):
        closed(binding, ("path", "sha256"), "checkpoint report binding")
        require(binding["sha256"] == digest(context.file(selector["file"])), "Report binding differs from original selected bytes")
    # Original captured exact-byte hashes are authoritative. Reconstruct only
    # inputs that were not measured then. Git's text checkout representation
    # must never replace an already captured byte identity (including mixed
    # line endings), nor excuse a mismatch in that original measurement.
    missing_capture = [row for row in now if row["path"] not in captured_names]
    checkpoint = checkpoint_sources(context, review["checkpoint"], missing_capture, bindings)
    return {"suiteId": suite_id, "source": proof["kind"], "scope": "TASK_LOCAL", "execution": reference, "target": target,
            "checkpoint": checkpoint, "binaryMeasurement": proof["binaryMeasurement"], "originalCwd": proof["originalCwd"],
            "limitations": review["unmeasuredAtExecution"]}


def verify_suite(root, reference, suite_id, suite, artifact_roots=None):
    proof = read_json(file_ref(root, reference))
    if proof.get("kind") == "REVIEWED_EXISTING_SUITE_EXECUTION":
        return verify_reviewed_suite(root, reference, proof, suite_id, suite, artifact_roots)
    fields = ["schemaVersion", "kind", "suiteId", "mode", "scope", "command", "cwd", "executableSha256", "sourcesBefore", "sourcesAfter", "sourceDigest", "stdout", "stderr", "exitCode", "timedOut", "logBudgetExceeded", "captureErrors", "durationSeconds", "sourceUnchanged"]
    closed(proof, fields, "suite execution proof")
    require(proof["schemaVersion"] == 1 and proof["kind"] in {"EXECUTED_SUITE", "IMPORTED_SUITE_EXECUTION"}, "Unknown execution proof kind")
    require(proof["suiteId"] == suite_id and proof["mode"] == "MOCK" and proof["scope"] == "TASK_LOCAL", "Wrong suite binding")
    require(isinstance(proof["command"], list) and bool(proof["command"]), "Missing actual command")
    executable = Path(proof["command"][0])
    cargo = executable if suite["command"][0] == "{cargo}" else None
    node = executable if suite["command"][0] == "{node}" else None
    command, cwd = expand(suite, cargo, node)
    require(proof["command"] == command and proof["cwd"] == str(cwd), "Actual command differs from fixed suite")
    require(digest(executable) == proof["executableSha256"], "Executable identity changed")
    now = sources(suite)
    require(proof["sourcesBefore"] == now == proof["sourcesAfter"] and proof["sourceUnchanged"] is True, "Source closure changed or missing")
    require(object_digest(now) == proof["sourceDigest"], "Source closure digest mismatch")
    require(type(proof["exitCode"]) is int and proof["exitCode"] == 0, "Actual suite exit was not zero")
    require(proof["timedOut"] is False and proof["logBudgetExceeded"] is False and proof["captureErrors"] == [], "Incomplete actual execution")
    file_ref(root, proof["stdout"])
    file_ref(root, proof["stderr"])
    return {"suiteId": suite_id, "source": proof["kind"], "scope": "TASK_LOCAL", "execution": reference}


def evaluate(root, definitions, artifact_roots=None):
    results = []
    cache = {}
    joined = joined_module()
    chains = {chain["id"]: chain for chain in definitions["chains"]}
    case_dir = root / "cases"
    no_links(case_dir)
    if case_dir.exists():
        require(case_dir.is_dir(), "Case evidence location must be a directory")
        require(all(p.name in {f"{case_id}.json" for case_id in CASE_IDS} and p.is_file() for p in case_dir.iterdir()), "Unknown case evidence file")
    for case in definitions["cases"]:
        row = {"caseId": case["id"], "required": True, "status": "NOT_RUN", "verifiedSuites": [], "verifiedChains": [], "missingChains": list(case["evidence"]["chains"]), "reasons": []}
        path = inside(root, f"cases/{case['id']}.json", False)
        if not path.exists():
            row["reasons"].append("Required case evidence missing")
            results.append(row)
            continue
        try:
            proof = closed(read_json(path), ["schemaVersion", "kind", "caseId", "suites", "chains"], "case evidence")
            require(proof["schemaVersion"] == 1 and proof["kind"] == "CASE_EVIDENCE_V1" and proof["caseId"] == case["id"], "Wrong case evidence binding")
            require(isinstance(proof["suites"], dict) and set(proof["suites"]) == set(case["evidence"]["suites"]), "Missing or extra suite proof")
            require(isinstance(proof["chains"], dict) and set(proof["chains"]) <= set(case["evidence"]["chains"]), "Unknown chain proof")
            for suite_id, reference in proof["suites"].items():
                key = (suite_id, object_digest(reference))
                if key not in cache:
                    cache[key] = verify_suite(root, reference, suite_id, definitions["suites"][suite_id], artifact_roots)
                row["verifiedSuites"].append(cache[key])
            for chain_id, reference in proof["chains"].items():
                key = ("joined", chain_id, object_digest(reference))
                try:
                    if key not in cache:
                        cache[key] = joined.verify(root, reference, chains[chain_id], SDK, artifact_roots)
                    row["verifiedChains"].append(cache[key])
                    row["missingChains"].remove(chain_id)
                except joined.MissingProof as error:
                    row["reasons"].append(f"{chain_id}: NOT_RUN {error}")
            if row["missingChains"]:
                row["reasons"].append("Required actual joined chain is NOT_RUN")
            elif row["verifiedSuites"] or row["verifiedChains"]:
                row["status"] = "PASS"
        except (Invalid, OSError, KeyError, TypeError, ValueError) as error:
            row["status"] = "FAIL"
            row["reasons"].append(str(error))
        results.append(row)
    return {"schemaVersion": 1, "kind": "CONFORMANCE_REPORT", "mode": "MOCK", "live": {"status": "NOT_RUN", "executed": False}, "globalPassed": all(r["status"] == "PASS" for r in results), "sdkReady": False, "phaseSelfTestPassed": None, "cases": results, "evidenceInterpretation": "Suite proofs are TASK_LOCAL. Imported proofs retain their explicit source; declarations alone do not establish joined runtime behavior."}


def reviewed_suite_self_test(root, definitions):
    """Controlled object/target fixtures, never historical SDK execution claims."""
    checks = []
    suite = definitions["suites"]["strict"]
    aggregate = ["cargo", "test", "--manifest-path", str(SDK / "Cargo.toml"), "--locked", "-p", "bgx-adapter-v2-validator"]
    require(typed_suite_command(aggregate, None, str(SDK), suite)["selection"] == "UNFILTERED_PACKAGE_ALL_TARGETS", "Aggregate Cargo target mapping failed")
    checks.append("reviewed_unfiltered_package_maps_fixed_integration_target")
    for label, suffix in [("no_run", ["--no-run"]), ("filter", ["some_test"]), ("list", ["--", "--list"])]:
        try: typed_suite_command(aggregate + suffix, None, str(SDK), suite)
        except Invalid: checks.append("reviewed_" + label + "_rejected")
        else: raise Invalid("Unexecuted/filtered target accepted")
    folder = root / "reviewed-object-fixture"; folder.mkdir()
    def ref(path): return {"path": path.relative_to(root).as_posix(), "sha256": digest(path)}
    def retain(kind, raw):
        oid = hashlib.sha1(kind.encode() + b" " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
        path = folder / oid
        if not path.exists(): path.write_bytes(raw)
        return oid, ref(path)
    source = "protocol/schema-policy.json"; raw = (SDK / source).read_bytes()
    blob, reference = retain("blob", raw)
    row = {"path": "packages/vendor-adapter-contract/v2/" + source, "sha256": hashlib.sha256(raw).hexdigest(),
           "oid": blob, "file": reference, "checkoutEncoding": "IDENTITY", "checkoutSha256": hashlib.sha256(raw).hexdigest()}
    trees = []; child = blob
    for index, part in enumerate(reversed(row["path"].split("/"))):
        oid, reference = retain("tree", ("100644" if index == 0 else "40000").encode() + b" " + part.encode() + b"\0" + bytes.fromhex(child))
        trees.append({"oid": oid, "file": reference}); child = oid
    commit, reference = retain("commit", b"tree " + child.encode() + b"\n\nInfrastructure fixture, not actual SDK execution\n")
    proof = {"kind": "REVIEWED_GIT_CHECKPOINT_V1", "commit": commit, "commitObject": reference, "trees": trees, "files": [row]}
    path = folder / "checkpoint.json"; claim_json(path, proof)
    context = joined_module().Context(root, SDK)
    selected = {"file": ref(path), "pointer": ""}
    needed = [{"path": source, "sha256": row["sha256"]}]
    require(checkpoint_sources(context, selected, needed, []) == commit, "Positive retained Git object chain failed")
    checks.append("reviewed_git_commit_tree_blob_and_current_sha256_bound")
    for label, modified in [("commit_identity", {**proof, "commit": "0" * 40}),
                            ("source_identity", {**proof, "files": [{**row, "checkoutSha256": "0" * 64}]})]:
        corrupted = folder / (label + ".json"); claim_json(corrupted, modified)
        try: checkpoint_sources(context, {"file": ref(corrupted), "pointer": ""}, needed, [])
        except Invalid: checks.append("reviewed_git_" + label + "_rejected")
        else: raise Invalid("Mutated Git reconstruction accepted")
    try: checkpoint_sources(context, selected, needed, [{"path": "unmeasured-original-report.json", "sha256": "0" * 64}])
    except Invalid: checks.append("reviewed_uncaptured_report_binding_rejected")
    else: raise Invalid("Unbound original report accepted")
    return checks


def infrastructure_catalog():
    value = read_json(SDK / "protocol/runner-self-test-cases.json")
    require(isinstance(value, dict) and set(value) == {"schemaVersion", "kind", "requiredChecks", "exclusiveChecks"}
            and type(value.get("schemaVersion")) is int and value["schemaVersion"] == 1
            and value.get("kind") == "RUNNER_INFRASTRUCTURE_CASES_V1"
            and isinstance(value.get("requiredChecks"), list) and value["requiredChecks"]
            and isinstance(value.get("exclusiveChecks"), list), "INFRA_CATALOG_INVALID")
    required = value["requiredChecks"]
    require(all(isinstance(v, str) and v for v in required), "INFRA_CATALOG_INVALID")
    identities, outcomes = list(required), list(required)
    for item in value["exclusiveChecks"]:
        require(isinstance(item, dict) and set(item) == {"id", "passed", "notRun"}
                and all(isinstance(v, str) and v for v in item.values()), "INFRA_CATALOG_INVALID")
        identities.append(item["id"]); outcomes.extend([item["passed"], item["notRun"]])
    require(len(identities) == len(set(identities)) and len(outcomes) == len(set(outcomes)), "INFRA_CATALOG_INVALID")
    return value


def validate_self_test_result(value):
    """Validate exact infrastructure identities, never global SDK readiness."""
    catalog = infrastructure_catalog()
    require(isinstance(value, dict) and type(value.get("schemaVersion")) is int and value["schemaVersion"] == 1
            and value.get("kind") == "RUNNER_SELF_TEST" and value.get("phaseSelfTestPassed") is True
            and value.get("globalPassed") is False and value.get("sdkReady") is False
            and value.get("globalCasesExecuted") == [] and value.get("C35") == value.get("C36") == value.get("live") == "NOT_RUN",
            "INFRA_RESULT_FLAGS_INVALID")
    checks = value.get("checks")
    require(isinstance(checks, list) and all(isinstance(v, str) and v for v in checks), "INFRA_CHECK_SET_MISMATCH")
    observed = set(checks)
    required = set(catalog["requiredChecks"])
    alternatives = catalog["exclusiveChecks"]
    allowed = required | {r[k] for r in alternatives for k in ("passed", "notRun")}
    require(len(checks) == len(observed) and required <= observed <= allowed
            and all(len(observed & {r["passed"], r["notRun"]}) == 1 for r in alternatives), "INFRA_CHECK_SET_MISMATCH")
    not_run = [r["notRun"] for r in alternatives if r["notRun"] in observed]
    return {"caseCount": len(required) + len(alternatives), "passedCount": len(observed) - len(not_run), "notRunChecks": not_run}


def self_test():
    checks = []
    with tempfile.TemporaryDirectory(prefix="bgx-sdk-conformance-selftest-") as temporary:
        root = workspace(Path(temporary) / "evidence", True)
        definitions, expectations = load_definitions()
        expected = expectations["runnerSelfTest"]
        report = evaluate(root, definitions)
        require(not report["globalPassed"] and len(report["cases"]) == 36, "Missing cases must fail full acceptance")
        require(all(r["status"] == "NOT_RUN" for r in report["cases"]), "Absent evidence cannot pass")
        checks.append("fixed_36_missing_required_rejected")
        full = execute([sys.executable, "-I", "-B", str(Path(__file__).resolve()), "--evidence-root", str(root), "--output", str(root / "receipts/full.json")], SDK, root / "full-command", 10, MAX_JSON)
        require(full["exitCode"] == 1 and read_json(root / "receipts/full.json")["globalPassed"] is False, "Default full command must actually exit nonzero")
        checks.append("actual_full_cli_missing_required_exit1")
        for label, command, expected_exit in [("positive", [sys.executable, "-I", "-c", "import json; print(json.dumps({'value':7}))"], 0), ("fault", [sys.executable, "-I", "-c", "raise SystemExit(9)"], expected["faultExitCode"])]:
            result = execute(command, root, root / label, 5, 4096)
            require(result["exitCode"] == expected_exit, "Actual child exit not retained")
            if label == "positive":
                payload = read_json(root / label / "stdout.log")
                require(payload == expected["positiveJson"], "Actual positive result differs from fixed expected JSON")
                try:
                    require(payload == expected["mutatedJson"], "Deliberately mutated JSON expectation")
                except Invalid:
                    checks.append("mutated_structured_expectation_rejected")
                else:
                    raise Invalid("Mutated structured expectation was accepted")
            checks.append(label + "_actual_subprocess")
        fixture = {"command": ["{node}", "-I", "-c", "import json; print(json.dumps({'value':7}))"], "cwd": "{sdk}", "sourceDirectories": [], "sourceFiles": ["protocol/conformance-cases.json"], "scope": "TASK_LOCAL"}
        reference, proof = run_suite(root, "selftest-python-only", fixture, {"timeoutSeconds": 5, "maxLogBytes": 4096}, None, Path(sys.executable))
        verify_suite(root, reference, "selftest-python-only", fixture)
        require(read_json(file_ref(root, proof["stdout"])) == expected["positiveJson"], "Actual recorded output not preserved")
        checks.append("actual_execution_source_exit_log_binding")
        corrupt = dict(proof)
        corrupt["sourcesBefore"] = [{"path": "protocol/conformance-cases.json", "sha256": "0" * 64}]
        corrupt_path = root / "corrupt-proof.json"
        claim_json(corrupt_path, corrupt)
        try:
            verify_suite(root, {"path": "corrupt-proof.json", "sha256": digest(corrupt_path)}, "selftest-python-only", fixture)
        except Invalid:
            checks.append("mutated_source_closure_rejected")
        else:
            raise Invalid("Changed compiled-source proof was accepted")
        output = root / "receipts" / "result.json"
        claim_json(output, {"value": 7})
        reference = {"path": "receipts/result.json", "sha256": digest(output)}
        file_ref(root, reference)
        failures = [("mutated_hash", lambda: file_ref(root, {**reference, "sha256": "0" * 64})), ("overwrite", lambda: claim_json(output, {})), ("path_escape", lambda: inside(root, "../escape.json")), ("unknown_fields", lambda: closed({"x": 1, "status": "PASS"}, ["x"], "self-test"))]
        for name, action in failures:
            try:
                action()
            except (Invalid, FileExistsError):
                checks.append(name + "_rejected")
            else:
                raise Invalid(f"Self-test negative did not fail: {name}")
        link = root / "link.json"
        try:
            link.symlink_to(output)
        except OSError:
            checks.append("symlink_creation_unavailable_NOT_RUN")
        else:
            try:
                inside(root, "link.json")
            except Invalid:
                checks.append("symlink_rejected")
            else:
                raise Invalid("Symlink accepted")
        timeout = execute([sys.executable, "-I", "-c", "import time; time.sleep(10)"], root, root / "timeout", 0.1, 4096)
        require(timeout["timedOut"] and timeout["exitCode"] != 0, "Timeout must kill and fail")
        checks.append("timeout_kill_reap")
        checks.extend(joined_module().self_test(root, SDK))
        checks.extend(reviewed_suite_self_test(root, definitions))
    result = {"schemaVersion": 1, "kind": "RUNNER_SELF_TEST", "phaseSelfTestPassed": True, "globalPassed": False, "sdkReady": False, "globalCasesExecuted": [], "C35": "NOT_RUN", "C36": "NOT_RUN", "live": "NOT_RUN", "checks": checks}
    validate_self_test_result(result)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("--create-evidence-root", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--execute-suite")
    parser.add_argument("--cargo", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--artifact-root", action="append", default=[], metavar="NAME=ABSOLUTE_DIRECTORY")
    args = parser.parse_args(argv)
    try:
        if args.self_test:
            require(not any([args.evidence_root, args.output, args.execute_suite, args.cargo, args.node, args.create_evidence_root, args.artifact_root]), "Self-test has no user workspace or command arguments")
            print(json.dumps(self_test(), ensure_ascii=False))
            return 0
        require(args.evidence_root is not None and args.output is not None, "Full/run mode requires explicit --evidence-root and --output")
        root = workspace(args.evidence_root, args.create_evidence_root)
        require(args.output.is_absolute(), "Output must be absolute")
        no_links(args.output)
        require(args.output.parent == root / "receipts" and args.output.suffix == ".json", "Output must be new evidence-root/receipts/*.json")
        require(not args.output.exists(), "Output already exists")
        definitions, expected = load_definitions()
        artifact_roots = {}
        for value in args.artifact_root:
            require("=" in value, "Artifact root requires NAME=ABSOLUTE_DIRECTORY")
            name, folder = value.split("=", 1)
            require(name not in artifact_roots, "Duplicate artifact root name")
            artifact_roots[name] = Path(folder)
        joined_module().Context(root, SDK, artifact_roots)
        if args.execute_suite:
            require(args.execute_suite in definitions["suites"], "Unknown suite ID")
            reference, actual = run_suite(root, args.execute_suite, definitions["suites"][args.execute_suite], expected["execution"], args.cargo, args.node)
            report = {"schemaVersion": 1, "kind": "TASK_LOCAL_SUITE_REPORT", "suiteId": args.execute_suite, "execution": reference, "globalPassed": False, "sdkReady": False, "exitCode": actual["exitCode"], "sourceUnchanged": actual["sourceUnchanged"]}
            success = actual["exitCode"] == 0 and actual["sourceUnchanged"] and not actual["timedOut"] and not actual["logBudgetExceeded"] and not actual["captureErrors"]
        else:
            require(args.cargo is None and args.node is None, "Executable flags require --execute-suite")
            report = evaluate(root, definitions, artifact_roots)
            success = report["globalPassed"]
        claim_json(args.output, report)
        print(json.dumps(report, ensure_ascii=False))
        return 0 if success else 1
    except (Invalid, OSError, KeyError, TypeError, ValueError, subprocess.SubprocessError) as error:
        print(json.dumps({"kind": "CONFORMANCE_ERROR", "code": "INVALID_EVIDENCE_OR_EXECUTION", "message": str(error), "globalPassed": False, "sdkReady": False}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
