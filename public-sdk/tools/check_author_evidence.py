#!/usr/bin/env python3
"""Verify independent-author measured artifacts, never infer C36 from examples.

Proof kind INDEPENDENT_AUTHOR_EVIDENCE_V1. References are closed objects
{root,path,sha256}; roots are explicitly authorized by CLI --artifact-root NAME=ABS.
The implicit root 'evidence' is --evidence-root. See AUTHOR-TASK.zh-CN.md.
This checks unkeyed evidence integrity and structured observations, not a signed
process attestation. An independent author identity is established by the caller.
"""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import math
import os
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("author_export", Path(__file__).with_name("export_sdk.py"))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)
require = exporter.require

def closed(v, keys, name):
    require(isinstance(v, dict) and set(v) == set(keys), "INVALID_FIELDS: " + name)

def reference(root, ref, roots):
    closed(ref, ["root", "path", "sha256"], "artifact reference")
    authorized = {"evidence": Path(root), **roots}
    require(ref["root"] in authorized, "UNAUTHORIZED_ARTIFACT_ROOT")
    base = exporter.absolute(authorized[ref["root"]])
    path = exporter.no_links(base / exporter.relative(ref["path"]), True)
    require(exporter.digest(path) == exporter.hash_value(ref["sha256"]), "ARTIFACT_HASH_DRIFT")
    return path

def read(root, ref, roots):
    return exporter.read_json(reference(root, ref, roots))

def payload(response):
    require(isinstance(response, dict) and response.get("protocolVersion") == 2 and "error" not in response
            and isinstance(response.get("dataJson"), str), "EXPECTED_PUBLIC_SUCCESS")
    value = json.loads(response["dataJson"])
    require(isinstance(value, dict), "EXPECTED_PUBLIC_OBJECT_PAYLOAD")
    return value

def new_operation(catalog, operation_id):
    matches = [(c, op) for c in catalog["capabilityContracts"] for op in c["operations"] if op["id"] == operation_id]
    require(len(matches) == 1 and "/" in operation_id, "NEW_NAMESPACED_OPERATION_REQUIRED")
    contract, operation = matches[0]
    schema = next(s for s in contract["schemas"] if s["id"] == operation["inputSchema"]["id"])
    require(exporter.sha(schema["schemaJson"].encode()) == operation["inputSchema"]["digest"], "INPUT_SCHEMA_HASH_DRIFT")
    value = json.loads(schema["schemaJson"])
    require("includeAudio" in value.get("required", []) and value.get("properties", {}).get("includeAudio", {}).get("type") == "boolean", "NEW_REQUIRED_BOOLEAN_MISSING")
    controls = operation["uiSchema"]["controls"]
    require(any(c["fieldPointer"] == "/includeAudio" for c in controls), "NEW_FIELD_NOT_DECLARED_IN_UI")
    return contract, operation


RUN_OPTIONS = {"--sdk", "--worker", "--package", "--workspace", "--config", "--input", "--output", "--request-id"}
TRUST_OPTIONS = {"--publisher-id", "--key-id", "--plugin-prefix", "--public-key"}
RUN_VERBS = {"capabilities", "probe", "upload", "submit", "query", "cancel"}


def cli_arguments(argv):
    """The published Args/Arguments grammar, not a shell or a flag search."""
    require(isinstance(argv, list) and len(argv) >= 2 and all(isinstance(x, str) and x for x in argv), "INVALID_CLI_ARGV")
    verb = "capabilities" if argv[1] == "caps" else argv[1]
    specifications = {
        "init": ({"--sdk", "--dir"}, set(), {"--sdk", "--dir"}),
        "build": ({"--sdk", "--project"}, {"--offline"}, {"--sdk", "--project"}),
        "check": ({"--sdk", "--project", "--worker"} | TRUST_OPTIONS, set(), {"--sdk", "--project", "--worker"}),
        "pack": ({"--sdk", "--project", "--out", "--signing-key", "--password-file"} | TRUST_OPTIONS, {"--unencrypted-key"}, {"--sdk", "--project", "--out"}),
    }
    if verb in RUN_VERBS:
        allowed, flags, required = RUN_OPTIONS, {"--create-workspace"}, RUN_OPTIONS - {"--request-id"}
    else:
        require(verb in specifications, "UNSUPPORTED_AUTHOR_CLI_COMMAND")
        allowed, flags, required = specifications[verb]
    values, present = {}, set()
    index = 2
    while index < len(argv):
        name = argv[index]
        require(name in allowed or name in flags, "UNKNOWN_CLI_OPTION")
        require(name not in present, "DUPLICATE_CLI_OPTION")
        present.add(name)
        if name in flags:
            index += 1
            continue
        require(index + 1 < len(argv) and not argv[index + 1].startswith("--"), "MISSING_CLI_OPTION_VALUE")
        values[name] = argv[index + 1]
        index += 2
    require(required <= set(values), "REQUIRED_CLI_OPTION_MISSING")
    path_options = {"--sdk", "--dir", "--project", "--worker", "--out", "--package", "--workspace",
                    "--config", "--input", "--output", "--public-key", "--signing-key", "--password-file"}
    require(all(Path(value).is_absolute() for name, value in values.items() if name in path_options), "ABSOLUTE_COMMAND_PATH_REQUIRED")
    if verb in RUN_VERBS:
        require((verb in {"query", "cancel"}) == ("--request-id" in values), "ORIGINAL_REQUEST_ID_OPTION_MISMATCH")
        require(not ("--request-id" in values and "--create-workspace" in present), "RESUME_CANNOT_CREATE_WORKSPACE")
    if verb in {"check", "pack"} and TRUST_OPTIONS & present:
        require(TRUST_OPTIONS <= present, "INCOMPLETE_TRUST_OPTIONS")
    if verb == "pack":
        signing = "--signing-key" in values
        modes = int("--password-file" in values) + int("--unencrypted-key" in present)
        require((signing and modes == 1) or (not signing and modes == 0), "INVALID_SIGNING_OPTIONS")
    return verb, values, present


def same_path(a, b):
    return os.path.normcase(os.path.abspath(a)) == os.path.normcase(os.path.abspath(b))


def authorized_path(value, root, roots, file=False):
    p = Path(value)
    require(p.is_absolute(), "ABSOLUTE_COMMAND_PATH_REQUIRED")
    p = Path(os.path.abspath(p))
    bases = [Path(os.path.abspath(root)), *[Path(os.path.abspath(x)) for x in roots.values()]]
    require(any(p.is_relative_to(base) for base in bases), "UNAUTHORIZED_COMMAND_ARTIFACT_PATH")
    return exporter.no_links(p, file)


def binary(candidate, role):
    manifest = exporter.load_manifest(candidate)
    logical = manifest["binaryInputs"][role]
    inventory = exporter.read_json(candidate / "release-files.json")["files"]
    matches = [x for x in inventory if x["path"] in {logical, logical + ".exe"}
               and x["origin"] == "explicit-" + role + "-binary-not-executed"]
    require(len(matches) == 1, "UNIQUE_CANDIDATE_BINARY_REQUIRED")
    path = exporter.no_links(candidate / matches[0]["path"], True)
    require(exporter.digest(path) == matches[0]["sha256"], "CANDIDATE_BINARY_HASH_DRIFT")
    return path


def runtime_artifacts(command, options, root, roots):
    paths = {name: authorized_path(value, root, roots, name in {"--input", "--output", "--config", "--worker"})
             for name, value in options.items() if name != "--request-id"}
    require(paths["--output"].is_relative_to(paths["--workspace"] / "receipts"), "RECEIPT_OUTSIDE_COMMAND_WORKSPACE")
    invocation = exporter.read_json(paths["--input"])
    require(invocation.get("protocolVersion") == 2 and invocation.get("operation") == command["verb"], "COMMAND_INPUT_OPERATION_MISMATCH")
    request_id = invocation.get("requestId")
    require(isinstance(request_id, str) and request_id, "COMMAND_INPUT_REQUEST_ID_MISSING")
    directory = paths["--workspace"] / "cli" / exporter.sha(request_id.encode("utf-8"))
    input_identity = exporter.read_json(exporter.no_links(directory / "input.identity.json", True))
    execution_identity = exporter.read_json(exporter.no_links(directory / "execution.identity.json", True))
    original = exporter.read_json(exporter.no_links(directory / "input.invocation.json", True))
    executed = exporter.read_json(exporter.no_links(directory / "execution.invocation.json", True))
    require(original == invocation and input_identity.get("requestSha256") == exporter.digest(paths["--input"]), "COMMAND_INPUT_IDENTITY_MISMATCH")
    require(input_identity.get("payloadSha256") == exporter.sha(invocation["payloadJson"].encode("utf-8")), "COMMAND_INPUT_PAYLOAD_IDENTITY_MISMATCH")
    require(executed.get("requestId") == request_id and executed.get("operation") == command["verb"]
            and execution_identity.get("payloadSha256") == exporter.sha(executed["payloadJson"].encode("utf-8")), "COMMAND_EXECUTION_IDENTITY_MISMATCH")
    require(exporter.digest(paths["--output"]) == exporter.digest(command["stdoutPath"]), "COMMAND_OUTPUT_STDOUT_BYTES_MISMATCH")
    response = exporter.read_json(paths["--output"])
    require(response.get("protocolVersion") == 2 and response.get("operation") == command["verb"], "COMMAND_OUTPUT_OPERATION_MISMATCH")
    require((command["value"]["exitCode"] == 0) == ("error" not in response), "COMMAND_ERROR_EXIT_MISMATCH")
    return {"paths": paths, "invocation": invocation, "response": response, "directory": directory}


def bind_commands(root, proof, roots, candidate, catalog, inv_path, invocation, cp1, cp2, commands):
    cli, worker = binary(candidate, "cli"), binary(candidate, "worker")
    rows = []
    for value in commands:
        verb, options, flags = cli_arguments(value["command"])
        executable = reference(root, value["executable"], roots)
        require(same_path(executable, cli) and same_path(value["command"][0], cli)
                and value["executable"]["sha256"] == exporter.digest(cli), "COMMAND_NOT_CANDIDATE_CLI")
        require(same_path(options["--sdk"], candidate), "COMMAND_SDK_MISMATCH")
        if "--worker" in options:
            require(same_path(options["--worker"], worker), "COMMAND_WORKER_MISMATCH")
        row = {"value": value, "verb": verb, "options": options, "flags": flags,
               "stdoutPath": reference(root, value["stdout"], roots)}
        if verb in RUN_VERBS:
            row.update(runtime_artifacts(row, options, root, roots))
        else:
            require(value["exitCode"] == 0, "AUTHOR_BUILD_FLOW_FAILED")
            row["stdout"] = exporter.read_json(row["stdoutPath"])
        rows.append(row)
    by_verb = {verb: [r for r in rows if r["verb"] == verb] for verb in {r["verb"] for r in rows}}
    require({"init", "build", "check", "pack", "capabilities", "submit", "query"} <= set(by_verb), "PUBLIC_AUTHOR_FLOW_NOT_EXECUTED")
    projects = {os.path.normcase(os.path.abspath(r["options"]["--dir"])) for r in by_verb["init"]}
    require(len(projects) == 1, "AUTHOR_PROJECT_LINEAGE_AMBIGUOUS")
    project = authorized_path(next(iter(projects)), root, roots)
    for row in by_verb["init"]:
        require(row["stdout"].get("status") == "CREATED" and same_path(row["stdout"]["directory"], project), "INIT_RESULT_PROJECT_MISMATCH")
    for row in by_verb["build"]:
        require(same_path(row["options"]["--project"], project) and row["stdout"].get("status") == "BUILT"
                and same_path(row["stdout"]["project"], project), "BUILD_PROJECT_LINEAGE_MISMATCH")
    packages = {os.path.normcase(os.path.abspath(r["options"]["--out"])) for r in by_verb["pack"]}
    require(len(packages) == 1, "AUTHOR_PACKAGE_LINEAGE_AMBIGUOUS")
    package = authorized_path(next(iter(packages)), root, roots)
    for row in by_verb["pack"]:
        require(same_path(row["options"]["--project"], project) and row["stdout"].get("status") == "PACKED"
                and same_path(row["stdout"]["directory"], package), "PACK_PROJECT_LINEAGE_MISMATCH")
    checked = set()
    for row in by_verb["check"]:
        require(row["stdout"].get("status") == "VALIDATED", "ACTUAL_AUTHOR_CHECK_FAILED")
        target = authorized_path(row["options"]["--project"], root, roots)
        require(same_path(target, project) or same_path(target, package), "CHECK_PROJECT_LINEAGE_MISMATCH")
        checked.add(os.path.normcase(str(target)))
    require({os.path.normcase(str(project)), os.path.normcase(str(package))} <= checked, "SOURCE_AND_PACKED_CHECK_REQUIRED")
    manifest = exporter.read_json(exporter.no_links(package / "manifest.json", True))
    require(manifest["pluginId"] == catalog["pluginId"] and manifest["version"] == catalog["pluginVersion"], "CATALOG_PACKAGE_IDENTITY_MISMATCH")
    entry = exporter.relative(manifest["entry"])
    for name in ("manifest.json", "manifest.sha256", entry):
        require(exporter.digest(exporter.no_links(project / name, True)) == exporter.digest(exporter.no_links(package / name, True)), "BUILT_PACKED_BYTES_MISMATCH")
    require(exporter.digest(package / entry) == manifest["artifactSha256"], "PACKED_ARTIFACT_HASH_MISMATCH")
    package_identity = {"pluginId": manifest["pluginId"], "version": manifest["version"], "digest": exporter.digest(package / "manifest.sha256")}
    require(cp1["package"] == cp2["package"] == package_identity, "CHECKPOINT_PACKAGE_LINEAGE_MISMATCH")
    source_paths = {os.path.normcase(str(reference(root, r, roots))) for r in proof["authorSources"]}
    require({os.path.normcase(str(project / n)) for n in ("manifest.json", "Cargo.toml", "Cargo.lock")} <= source_paths
            and any(Path(p).is_relative_to(project) and Path(p).suffix == ".rs" for p in source_paths), "AUTHOR_PROJECT_SOURCE_REFERENCES_MISSING")
    runtime = [r for r in rows if r["verb"] in RUN_VERBS]
    require(all(same_path(r["paths"]["--package"], package) for r in runtime), "RUNTIME_PACKAGE_LINEAGE_MISMATCH")
    caps = [r for r in by_verb["capabilities"] if r["value"]["exitCode"] == 0 and payload(r["response"]) == catalog]
    require(caps, "CATALOG_NOT_BOUND_TO_CAPABILITIES_EXECUTION")
    accepted_path = reference(root, proof["accepted"], roots)
    completed_path = reference(root, proof["completed"], roots)
    failure_path = reference(root, proof["failure"], roots)
    submits = [r for r in by_verb["submit"] if r["value"]["exitCode"] == 0]
    require(len(submits) == 1, "UNIQUE_ACCEPTED_SUBMIT_REQUIRED")
    submit = submits[0]
    require(same_path(submit["paths"]["--input"], inv_path) and exporter.digest(submit["paths"]["--input"]) == proof["invocation"]["sha256"], "SUBMIT_BROWSER_INPUT_MISMATCH")
    require(same_path(submit["paths"]["--output"], accepted_path), "SUBMIT_ACCEPTED_OUTPUT_MISMATCH")
    require(same_path(reference(root, proof["inputIdentity"], roots), submit["directory"] / "input.identity.json")
            and same_path(reference(root, proof["executionIdentity"], roots), submit["directory"] / "execution.identity.json"), "PROOF_IDENTITY_WORKSPACE_MISMATCH")
    downloads = [o for o in read(root, proof["browser"], roots)["observations"] if o.get("kind") == "download"]
    require(downloads and all(o.get("requestId") == invocation["requestId"] for o in downloads), "BROWSER_REQUEST_ID_MISMATCH")
    accepted = payload(submit["response"])
    queries = [r for r in by_verb["query"] if r["value"]["exitCode"] == 0]
    matching = []
    for query in queries:
        require(query["value"]["pid"] != submit["value"]["pid"] and query["options"]["--request-id"] == invocation["requestId"], "NEW_PROCESS_ORIGINAL_ID_QUERY_REQUIRED")
        require(same_path(query["paths"]["--workspace"], submit["paths"]["--workspace"])
                and same_path(query["paths"]["--config"], submit["paths"]["--config"]), "QUERY_WORKSPACE_CONFIGURATION_MISMATCH")
        task = json.loads(query["invocation"]["payloadJson"])
        require(task["vendorTaskId"] == accepted["vendorTaskId"] and task["resumeToken"] == cp1["resumeToken"]
                and task["operationId"] == proof["operationId"] and query["invocation"]["requestId"] != invocation["requestId"], "QUERY_ORIGINAL_TASK_MISMATCH")
        if same_path(query["paths"]["--output"], completed_path): matching.append(query)
    require(len(matching) == 1, "COMPLETED_OUTPUT_NOT_BOUND_TO_QUERY")
    require(cp1["result"] == accepted and cp2["result"] == payload(matching[0]["response"]), "CHECKPOINT_RESULT_RECEIPT_MISMATCH")
    failures = [r for r in by_verb["submit"] if r["value"]["exitCode"] != 0 and same_path(r["paths"]["--output"], failure_path)]
    require(len(failures) == 1, "FAILURE_OUTPUT_NOT_BOUND_TO_NONZERO_SUBMIT")
    negative = failures[0]
    require(negative["invocation"]["requestId"] != invocation["requestId"]
            and json.loads(negative["invocation"]["payloadJson"])["operationId"] == proof["operationId"]
            and same_path(negative["paths"]["--workspace"], submit["paths"]["--workspace"]), "NEGATIVE_SUBMIT_CONTEXT_MISMATCH")

def verify_evidence(root, proof, artifact_roots=None):
    roots = artifact_roots or {}
    require("evidence" not in roots, "RESERVED_EVIDENCE_ROOT_OVERRIDE")
    closed(proof, ["schemaVersion", "kind", "author", "candidateRoot", "candidateBefore", "candidateAfter",
        "operationId", "catalog", "executions", "browser", "invocation", "inputIdentity", "executionIdentity",
        "accepted", "completed", "failure", "checkpointBefore", "checkpointAfter", "serviceBefore", "serviceAfter",
        "media", "authorSources", "cleanup"], "independent author proof")
    require(proof["schemaVersion"] == 1 and proof["kind"] == "INDEPENDENT_AUTHOR_EVIDENCE_V1", "INVALID_AUTHOR_PROOF")
    closed(proof["author"], ["identity", "independent", "providedInputs", "forbiddenInputsRead"], "author")
    require(proof["author"]["independent"] is True and proof["author"]["identity"] and proof["author"]["forbiddenInputsRead"] == [], "INDEPENDENT_AUTHOR_REQUIRED")
    require(proof["author"]["providedInputs"] == ["PUBLIC_CANDIDATE_SDK", "PUBLIC_SERVICE_API", "RESERVED_AUTHOR_TASK"], "AUTHOR_INPUT_BOUNDARY")
    require(proof["candidateRoot"] in roots, "EXPLICIT_CANDIDATE_ROOT_REQUIRED")
    candidate = exporter.absolute(roots[proof["candidateRoot"]])
    verified = exporter.verify_candidate(candidate)
    before, after = read(root, proof["candidateBefore"], roots), read(root, proof["candidateAfter"], roots)
    require(before == after == {"releaseFiles": exporter.read_json(candidate / "release-files.json"),
            "governance": verified["governanceSha256"]}, "CANDIDATE_CHANGED_DURING_AUTHOR_TASK")
    catalog = read(root, proof["catalog"], roots)
    contract, operation = new_operation(catalog, proof["operationId"])
    # A completed public example is not a new reserved operation.
    existing = {exporter.read_json(p)["pluginId"] for p in (candidate / "examples").glob("*/manifest.json")}
    existing.add(exporter.read_json(candidate / "templates/rust/manifest.json")["pluginId"])
    require(catalog["pluginId"] not in existing, "TEAM_EXAMPLE_IS_NOT_AN_INDEPENDENT_OPERATION")
    require(operation["id"].startswith(catalog["pluginId"] + "/"), "NEW_OPERATION_MUST_USE_AUTHOR_NAMESPACE")
    inv_path = reference(root, proof["invocation"], roots)
    invocation = exporter.read_json(inv_path)
    request = json.loads(invocation["payloadJson"])
    require(invocation["operation"] == "submit" and request["operationId"] == operation["id"] and request["input"].get("includeAudio") is True, "AUTHOR_INVOCATION_MISMATCH")
    require(request["capabilityKey"] == contract["capabilityKey"] and request["inputSchemaDigest"] == operation["inputSchema"]["digest"], "AUTHOR_INPUT_CONTRACT_DRIFT")
    input_id = read(root, proof["inputIdentity"], roots)
    execution_id = read(root, proof["executionIdentity"], roots)
    require(input_id["requestSha256"] == exporter.digest(inv_path), "BROWSER_FILE_REWRITTEN")
    require(input_id["payloadSha256"] == execution_id["payloadSha256"] == exporter.sha(invocation["payloadJson"].encode()), "PAYLOAD_TRANSLATED")
    browser = read(root, proof["browser"], roots)
    require(browser.get("kind") == "ACTUAL_AUTHOR_BROWSER_EXPORT" and browser.get("exportSha256") == exporter.digest(inv_path), "ACTUAL_BROWSER_EXPORT_REQUIRED")
    require(browser.get("requiredFieldsVisible") is True and browser.get("includeAudioEdited") is True, "NEW_REQUIRED_FIELD_NOT_EXERCISED")
    require(browser.get("apiRequests") == [] and isinstance(browser.get("observations"), list) and browser["observations"], "BROWSER_OBSERVATIONS_REQUIRED")
    accepted = payload(read(root, proof["accepted"], roots))
    completed = payload(read(root, proof["completed"], roots))
    require(accepted.get("kind") == "accepted" and accepted.get("vendorTaskId"), "ACTUAL_ACCEPTED_ID_REQUIRED")
    require(completed.get("status") == "succeeded", "ACTUAL_QUERY_COMPLETION_REQUIRED")
    cp1, cp2 = read(root, proof["checkpointBefore"], roots), read(root, proof["checkpointAfter"], roots)
    require(cp1["requestId"] == cp2["requestId"] == invocation["requestId"] and cp1["vendorTaskId"] == cp2["vendorTaskId"] == accepted["vendorTaskId"], "ORIGINAL_CHECKPOINT_ID_CHANGED")
    state1, state2 = read(root, proof["serviceBefore"], roots), read(root, proof["serviceAfter"], roots)
    require(state1["submitCount"] == state2["submitCount"] == 1 and set(state1["jobs"]) == set(state2["jobs"]) == {accepted["vendorTaskId"]}, "RECOVERY_RESUBMITTED")
    require(state2["jobs"][accepted["vendorTaskId"]]["status"] == "succeeded", "REMOTE_JOB_NOT_COMPLETED")
    failure = read(root, proof["failure"], roots)
    require(failure.get("protocolVersion") == 2 and failure.get("error", {}).get("code") == "UPSTREAM_ERROR", "ACTUAL_STRUCTURED_SERVICE_FAILURE_REQUIRED")
    require(len(proof["media"]) >= 2, "ACTUAL_MULTIPLE_MEDIA_REQUIRED")
    outputs = {o["media"]["id"]: o for o in completed["outputs"] if o["kind"] == "media"}
    kinds = set()
    for item in proof["media"]:
        closed(item, ["mediaId", "file"], "actual media")
        path = reference(root, item["file"], roots)
        out = outputs[item["mediaId"]]
        require(path.stat().st_size == int(out["media"]["byteLength"]), "MEDIA_BYTES_MISMATCH")
        identities = [m for m in cp2["mediaIdentities"] if m["id"] == item["mediaId"]]
        require(len(identities) == 1 and identities[0]["digest"] == exporter.digest(path), "DURABLE_MEDIA_IDENTITY_MISMATCH")
        raw = path.read_bytes()[:12]
        kind = out["media"]["kind"]
        require((kind == "image" and raw[:8] == b"\x89PNG\r\n\x1a\n") or (kind == "audio" and raw[:4] == b"RIFF" and raw[8:12] == b"WAVE"), "ACTUAL_MEDIA_SIGNATURE_REQUIRED")
        kinds.add(kind)
    require({"image", "audio"} <= kinds, "IMAGE_AUDIO_PAIR_REQUIRED")
    commands = []
    for ref in proof["executions"]:
        value = read(root, ref, roots)
        require(value.get("kind") == "ACTUAL_AUTHOR_COMMAND" and isinstance(value.get("command"), list) and value["command"], "ACTUAL_COMMAND_REQUIRED")
        reference(root, value["stdout"], roots); reference(root, value["stderr"], roots)
        duration = value.get("durationSeconds")
        require(type(value.get("exitCode")) is int and value.get("timedOut") is False
                and type(duration) in (int, float) and math.isfinite(duration) and duration > 0, "COMMAND_RESULT_REQUIRED")
        require(type(value.get("pid")) is int and value["pid"] > 0, "ACTUAL_PROCESS_ID_REQUIRED")
        executable = reference(root, value["executable"], roots)
        require(os.path.normcase(str(executable)) == os.path.normcase(str(Path(value["command"][0]))), "COMMAND_EXECUTABLE_MISMATCH")
        commands.append(value)
    verbs = {c["command"][1] for c in commands if len(c["command"]) > 1 and c["exitCode"] == 0}
    require({"init", "build", "check", "pack", "capabilities", "submit", "query"} <= verbs, "PUBLIC_AUTHOR_FLOW_NOT_EXECUTED")
    require(any(c["exitCode"] != 0 and len(c["command"]) > 1 and c["command"][1] == "submit" for c in commands), "NONZERO_ERROR_EXECUTION_REQUIRED")
    submits = [c for c in commands if c["exitCode"] == 0 and len(c["command"]) > 1 and c["command"][1] == "submit"]
    queries = [c for c in commands if c["exitCode"] == 0 and len(c["command"]) > 1 and c["command"][1] == "query"]
    require(any(a["pid"] != b["pid"] and "--request-id" in b["command"] and b["command"][b["command"].index("--request-id")+1] == invocation["requestId"] for a in submits for b in queries), "NEW_PROCESS_ORIGINAL_ID_QUERY_REQUIRED")
    require(proof["authorSources"], "AUTHOR_SOURCE_PROVENANCE_REQUIRED")
    for ref in proof["authorSources"]: reference(root, ref, roots)
    bind_commands(root, proof, roots, candidate, catalog, inv_path, invocation, cp1, cp2, commands)
    cleanup = read(root, proof["cleanup"], roots)
    require(cleanup.get("allProcessesWaited") is True and cleanup.get("portsReleased") is True and cleanup.get("errors") == [], "AUTHOR_PROCESS_CLEANUP_INCOMPLETE")
    return {"caseId": "C36", "status": "PASS", "source": "INDEPENDENT_AUTHOR_ACTUAL_ARTIFACTS", "candidateInventorySha256": verified["releaseFilesSha256"], "operationId": operation["id"], "sdkReady": False}

def self_test():
    checks = []
    receipt_cases = []
    with tempfile.TemporaryDirectory(prefix="bgx-author-check-") as temp:
        root = Path(temp)
        file = root / "observed.json"; file.write_text('{"n":1}', encoding="utf-8")
        ref = {"root":"evidence", "path":"observed.json", "sha256":exporter.digest(file)}
        require(read(root, ref, {}) == {"n":1}, "SELF_TEST_READ"); checks.append("hashed-reference-positive")
        def rejected(name, action):
            try: action()
            except (exporter.ExportError, ValueError, KeyError, OSError): checks.append(name)
            else: raise AssertionError("Counterfactual fixture accepted: " + name)
        rejected("hash-drift", lambda: read(root, dict(ref, sha256="0" * 64), {}))
        cli, sdk, project = str(root / "cli"), str(root / "sdk"), str(root / "project")
        runtime = [cli, "submit", "--sdk", sdk, "--worker", str(root / "worker"), "--package", str(root / "package"),
                   "--workspace", str(root / "workspace"), "--config", str(root / "config.json"),
                   "--input", str(root / "input.json"), "--output", str(root / "workspace/receipts/result.json")]
        positive = [
            [cli,"init","--sdk",sdk,"--dir",project],
            [cli,"build","--sdk",sdk,"--project",project,"--offline"],
            [cli,"check","--sdk",sdk,"--worker",str(root/"worker"),"--project",project],
            [cli,"pack","--sdk",sdk,"--project",project,"--out",str(root/"package")],
            runtime, [cli,"query",*runtime[2:],"--request-id","original"],
            [cli,"caps",*runtime[2:],"--create-workspace"],
        ]
        for argv in positive: cli_arguments(argv); checks.append("closed-argv-positive-" + argv[1])
        rejected("duplicate-value-option",lambda:cli_arguments(runtime+["--input",str(file)]))
        rejected("duplicate-boolean-option",lambda:cli_arguments(positive[1]+["--offline"]))
        rejected("unknown-option",lambda:cli_arguments(runtime+["--made-up", "x"]))
        rejected("missing-option-value",lambda:cli_arguments(runtime[:-1]))
        rejected("missing-required-option",lambda:cli_arguments(runtime[:2]+runtime[4:]))
        rejected("relative-path",lambda:cli_arguments([cli,"init","--sdk","relative","--dir",project]))
        rejected("submit-with-original-id",lambda:cli_arguments(runtime+["--request-id","x"]))
        rejected("query-without-original-id",lambda:cli_arguments([cli,"query",*runtime[2:]]))
        rejected("query-creating-workspace",lambda:cli_arguments(positive[5]+["--create-workspace"]))
        rejected("incomplete-trust-options",lambda:cli_arguments(positive[2]+["--publisher-id","x"]))
        rejected("signing-mode-without-key",lambda:cli_arguments(positive[3]+["--unencrypted-key"]))

        # Synthetic file/receipt controls exercise the actual association check.
        # They are not SDK, worker, package or upstream service executions.
        def receipt_fixture(case_id, failure):
            home = root / "receipt-fixtures" / case_id
            home.mkdir(parents=True)
            paths = {"--sdk": home / "sdk", "--package": home / "package",
                     "--workspace": home / "workspace", "--worker": home / "worker.bin",
                     "--config": home / "config.json", "--input": home / "input.json",
                     "--output": home / "workspace/receipts/result.json"}
            for name in ("--sdk", "--package", "--workspace"):
                paths[name].mkdir()
            paths["--output"].parent.mkdir()
            paths["--worker"].write_bytes(b"SYNTHETIC_NOT_EXECUTABLE")
            paths["--config"].write_text("{}", encoding="utf-8")
            schema_json = json.dumps({"type": "object", "properties": {}, "required": [], "additionalProperties": False})
            request = {"schemaVersion": 2, "modelId": "fixture-model", "capabilityKey": "local.fixture/receipt",
                       "operationId": "local.fixture/submit", "operationVersion": 1,
                       "inputSchemaDigest": exporter.sha(schema_json.encode("utf-8")),
                       "input": {}, "media": [], "configuration": {}, "uploads": []}
            invocation = {"protocolVersion": 2, "operation": "submit", "profileId": "fixture-binding",
                          "capabilitySlot": "generic", "requestId": case_id, "budgetMs": 1000,
                          "payloadJson": json.dumps(request), "bindingId": "fixture-binding",
                          "negotiatedVersion": 2, "enabledFeatures": []}
            raw_input = json.dumps(invocation).encode("utf-8")
            paths["--input"].write_bytes(raw_input)
            directory = paths["--workspace"] / "cli" / exporter.sha(case_id.encode("utf-8"))
            directory.mkdir(parents=True)
            for name in ("input.invocation.json", "execution.invocation.json"):
                (directory / name).write_bytes(raw_input)
            payload_sha = exporter.sha(invocation["payloadJson"].encode("utf-8"))
            (directory / "input.identity.json").write_text(json.dumps({
                "requestSha256": exporter.digest(paths["--input"]), "payloadSha256": payload_sha}), encoding="utf-8")
            (directory / "execution.identity.json").write_text(json.dumps({"payloadSha256": payload_sha}), encoding="utf-8")
            if failure:
                response = {"protocolVersion": 2, "operation": "submit", "error": {
                    "code": "UPSTREAM_ERROR", "message": "Synthetic failure fixture", "retryable": False,
                    "vendorCode": None, "vendorRequestId": None}}
            else:
                response = {"protocolVersion": 2, "operation": "submit", "dataJson": json.dumps({
                    "kind": "accepted", "vendorTaskId": "synthetic-job", "appliedParams": None,
                    "resumeToken": None, "usage": None})}
            raw_output = json.dumps(response).encode("utf-8")
            paths["--output"].write_bytes(raw_output)
            stdout = home / "stdout.json"; stdout.write_bytes(raw_output)
            command = {"verb": "submit", "value": {"exitCode": 1 if failure else 0}, "stdoutPath": stdout}
            options = {name: str(path) for name, path in paths.items()}
            observed = runtime_artifacts(command, options, home, {})
            require(observed["response"] == response, "RECEIPT_FIXTURE_BASELINE_REJECTED")
            return home, command, options

        definitions = [
            ("B01-T01", "receipt-valid-success", False, None),
            ("B01-T02", "receipt-valid-failure", True, None),
            ("B01-T03", "receipt-byte-mismatch", False, "COMMAND_OUTPUT_STDOUT_BYTES_MISMATCH"),
            ("B01-T04", "receipt-outside-workspace", False, "RECEIPT_OUTSIDE_COMMAND_WORKSPACE"),
            ("B01-T05", "receipt-missing-output", False, "MISSING_OUTPUT_PATH"),
            ("B01-T06", "receipt-operation-mismatch", False, "COMMAND_OUTPUT_OPERATION_MISMATCH"),
            ("B01-T07", "receipt-zero-exit-error", True, "COMMAND_ERROR_EXIT_MISMATCH"),
            ("B01-T08", "receipt-nonzero-exit-success", False, "COMMAND_ERROR_EXIT_MISMATCH"),
        ]
        for case_id, name, failure, expected in definitions:
            home, command, options = receipt_fixture(case_id, failure)
            output = Path(options["--output"])
            if case_id == "B01-T03":
                output.write_bytes(b" " + output.read_bytes())
                require(exporter.read_json(output) == exporter.read_json(command["stdoutPath"]),
                        "BYTE_MISMATCH_FIXTURE_CHANGED_JSON_MEANING")
            elif case_id == "B01-T04":
                outside = home / "outside-result.json"; outside.write_bytes(output.read_bytes())
                options["--output"] = str(outside)
            elif case_id == "B01-T05":
                output.unlink(); expected = "MISSING_INPUT: " + str(output)
            elif case_id == "B01-T06":
                response = exporter.read_json(output); response["operation"] = "query"
                raw = json.dumps(response).encode("utf-8")
                output.write_bytes(raw); command["stdoutPath"].write_bytes(raw)
            elif case_id == "B01-T07":
                command["value"]["exitCode"] = 0
            elif case_id == "B01-T08":
                command["value"]["exitCode"] = 1
            error = None
            try:
                runtime_artifacts(command, options, home, {})
            except exporter.ExportError as exc:
                error = str(exc)
                require(expected is not None and error == expected, "UNEXPECTED_RECEIPT_REJECTION: " + name + ": " + error)
            else:
                require(expected is None, "INVALID_RECEIPT_ACCEPTED: " + name)
            checks.append(name)
            receipt_cases.append({"caseId": case_id, "testId": name, "baselineVerified": True,
                                  "status": "ACCEPTED" if expected is None else "EXPECTED_REJECTION",
                                  "observedError": error, "outputPath": options["--output"]})
        fixture_bytes = sum(p.stat().st_size for p in root.rglob("*") if p.is_file())
        require(fixture_bytes <= 20 * 1024 * 1024, "SELF_TEST_FIXTURE_BUDGET_EXCEEDED")
    print(json.dumps({"scope":"COMMAND_ARGUMENT_REFERENCE_AND_RECEIPT_FIXTURES_ONLY", "checks":len(checks),
                      "cases":checks, "receiptCases":receipt_cases, "fixtureBytes":fixture_bytes,
                      "runtimeExecuted":False, "C36":"NOT_RUN", "sdkReady":False}))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--evidence-root", type=Path); p.add_argument("--proof", type=Path)
    p.add_argument("--artifact-root", action="append", default=[])
    p.add_argument("--self-test", action="store_true")
    args = p.parse_args()
    if args.self_test:
        self_test(); return
    require(args.evidence_root is not None and args.proof is not None, "EXPLICIT_AUTHOR_PROOF_REQUIRED")
    roots = {}
    for item in args.artifact_root:
        name, path = item.split("=", 1)
        require(name and name != "evidence" and name not in roots, "DUPLICATE_ARTIFACT_ROOT")
        roots[name] = exporter.absolute(Path(path))
    result = verify_evidence(exporter.absolute(args.evidence_root), exporter.read_json(exporter.absolute(args.proof, True)), roots)
    print(json.dumps(result, ensure_ascii=False))

if __name__ == "__main__":
    try: main()
    except (exporter.ExportError, KeyError, ValueError, OSError, TypeError, IndexError, StopIteration) as e:
        print(json.dumps({"status":"NOT_VERIFIED", "error":str(e), "sdkReady":False}), file=sys.stderr); raise SystemExit(1)
