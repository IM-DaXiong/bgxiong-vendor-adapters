"""Strict joined-evidence comparisons; no subprocess/network/service execution.

JOINED_CHAIN_PROOF_V1 contains schemaVersion, kind, chainId, mode=MOCK,
provenance (CAPTURED_EXECUTION or REVIEWED_EXISTING_EXECUTION), sourceClosure,
executions, artifacts. sourceClosure has current (fixed SDK snapshot), captured
(same rows at execution, or an explicitly reviewed subset), review (selector or
null). A selector is {file:{[root,]path,sha256},pointer: RFC6901-or-empty}; root
names are authorized only by CLI --artifact-root NAME=ABS, never by evidence JSON.
When a selector imports a self-contained proof, that selector's authorized
file.root is the imported proof's implicit evidence root. It is never inferred
from the file's parent. Explicit root aliases retain their caller authorization;
the reserved outer evidence alias cannot override this local interpretation.

executions are {record:selector,expectedExitCode:int[,retainedBinaries:[
{originalPath,target,file:ref}]]}; selectors point directly
to original harness command rows, preserving actual argv/exit/stdout/stderr/log.
artifacts has exactly the chain's requiredArtifacts, each a selector. Structured
artifacts may link more selectors; raw media/fixture uses selector.file. No test
name, status=PASS, declared count of assertions, or stdout substring is proof.

Per-chain shapes are explicit in the validators below. References/links select
raw original records; they do not create retrospective measurements. Reviewed
source evidence must name unmeasuredAtExecution and remains labelled REVIEWED.
Its review.capturedSource selector binds the selected source subset to an
unchanged original captured source map; selected hashes cannot be invented.
Missing raw facts raise MissingProof and remain NOT_RUN. Invalid/mutated facts
fail. Public runtime/schema validation continues to belong to the shared Rust
validator/host; these comparisons check observed identities, counters and causal
ordering, rather than implement another adapter or lifecycle state machine.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
from decimal import Decimal

IDS = ("real-probe", "three-packages", "upload", "multi-output", "stream-events",
       "recover", "unknown-acceptance", "cancel-isolation", "replay-current",
       "profile-parity", "form-cli", "matrix", "candidate", "external-author")
SKIP = {"target", ".bgx-sdk", "node_modules", "dist", ".git", "__pycache__", ".sdk-test-workspaces"}
SOURCE_SUFFIXES = {".rs", ".toml", ".lock", ".json", ".wit", ".ts", ".py", ".html"}


class InvalidProof(ValueError):
    pass


class MissingProof(InvalidProof):
    pass


def require(ok, message):
    if not ok:
        raise InvalidProof(message)


def need(ok, message):
    if not ok:
        raise MissingProof(message)


def closed(value, fields, name):
    require(isinstance(value, dict) and set(value) == set(fields), "Invalid fields: " + name)
    return value


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def object_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def ordinary_path(value):
    value = str(value)
    if os.name == "nt":
        if value.startswith("\\\\?\\UNC\\"): value = "\\\\" + value[8:]
        elif value.startswith("\\\\?\\"): value = value[4:]
    return Path(value)


def no_links(path):
    path = ordinary_path(os.path.abspath(path))
    for item in [*reversed(path.parents), path]:
        try: info = item.lstat()
        except FileNotFoundError: raise MissingProof("Missing retained artifact: " + str(item)) from None
        # Cargo legitimately hard-links named executables and deps artifacts.
        # This reader never writes a referenced inode; exact bytes are hashed.
        # Symlink/reparse traversal still cannot escape an authorized root.
        require(not stat.S_ISLNK(info.st_mode) and not (getattr(info, "st_file_attributes", 0) & 0x400),
                "Linked evidence forbidden")
    return path


def safe_relative(value):
    require(isinstance(value, str) and value and "\\" not in value, "POSIX relative artifact path required")
    path = PurePosixPath(value)
    require(path.as_posix() == value and not path.is_absolute() and
            all(p not in ("", ".", "..") and not p.endswith((".", " ")) and
                not any(c in p for c in ':*?<>|\x00') and not any(ord(c) < 32 for c in p)
                for p in path.parts), "Unsafe artifact path")
    return value


def pairs(items):
    value = {}
    for key, item in items:
        require(key not in value, "Duplicate JSON key")
        value[key] = item
    return value


def read_json(path):
    path = no_links(path)
    need(path.is_file(), "Missing JSON artifact")
    require(path.stat().st_size <= 32 * 1024 * 1024, "JSON artifact too large")
    return json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=pairs,
                      parse_constant=lambda _: (_ for _ in ()).throw(InvalidProof("Nonfinite JSON number")))


def pointer(value, path):
    require(isinstance(path, str) and (path == "" or path.startswith("/")), "RFC6901 pointer required")
    for part in path.split("/")[1:]:
        require(re.search(r"~(?:[^01]|$)", part) is None, "Invalid pointer escape")
        part = part.replace("~1", "/").replace("~0", "~")
        if isinstance(value, dict):
            need(part in value, "Missing raw pointer member: " + part)
            value = value[part]
        elif isinstance(value, list):
            require(re.fullmatch("0|[1-9][0-9]*", part) is not None, "Invalid array pointer")
            need(int(part) < len(value), "Missing raw array item")
            value = value[int(part)]
        else: raise MissingProof("Pointer does not select a retained raw fact")
    return value


def source_snapshot(sdk):
    """Fixed compiled/runtime public input closure, including include! JSON/templates.

    No cache traversal or inferred historical closure. Whole relevant source
    categories make literal/concat includes and generated guest strings visible.
    """
    names = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "operation-contracts.json"}
    for category in ("crates", "sdk", "schemas", "protocol", "wit", "templates", "examples", "fixtures", "tools"):
        folder = no_links(sdk / category)
        for current, directories, files in os.walk(folder, followlinks=False):
            directories[:] = sorted(x for x in directories if x not in SKIP and not x.startswith("."))
            for name in directories: no_links(Path(current) / name)
            for name in files:
                p = Path(current) / name
                if p.suffix in SOURCE_SUFFIXES and p.relative_to(sdk).as_posix() != "schemas/adoption-evidence.schema.json":
                    no_links(p); names.add(p.relative_to(sdk).as_posix())
    return [{"path": name, "sha256": sha(no_links(sdk / name))} for name in sorted(names)]


def rooted_author_reference(ref):
    """The author path has one explicit rooted shape; never a legacy retry."""
    require(isinstance(ref, dict) and set(ref) == {"root", "path", "sha256"}, "AUTHOR_ROOTED_REFERENCE_REQUIRED")
    require(isinstance(ref["root"], str) and re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,31}", ref["root"]),
            "AUTHOR_REFERENCE_ROOT_NAME_INVALID")
    safe_relative(ref["path"])
    require(isinstance(ref["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", ref["sha256"]),
            "AUTHOR_REFERENCE_DIGEST_REQUIRED")
    return ref


def rooted_author_selector(selector):
    require(isinstance(selector, dict) and set(selector) == {"file", "pointer"}, "AUTHOR_ROOTED_SELECTOR_REQUIRED")
    rooted_author_reference(selector["file"])
    require(isinstance(selector["pointer"], str), "AUTHOR_SELECTOR_POINTER_REQUIRED")
    return selector


class Context:
    def __init__(self, root, sdk, artifact_roots=None):
        self.author_roots_explicit = all(Path(p).is_absolute() and ".." not in Path(p).parts
                                         for p in [root, sdk, *(artifact_roots or {}).values()])
        self.root, self.sdk = no_links(root), no_links(sdk)
        self.roots = {"evidence": self.root, "sdk": self.sdk}
        for name, folder in (artifact_roots or {}).items():
            require(re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,31}", name) is not None and name not in self.roots,
                    "Duplicate/invalid artifact root name")
            require(Path(folder).is_absolute(), "Explicit artifact roots must be absolute")
            self.roots[name] = no_links(folder)
        self.artifacts = {}
        self.executions = []
        self.record_contexts = {}

    def file(self, ref):
        require(isinstance(ref, dict) and set(ref) in ({"path", "sha256"}, {"root", "path", "sha256"}),
                "Invalid file reference")
        name = ref.get("root", "evidence")
        require(name in self.roots, "Artifact root not explicitly authorized: " + str(name))
        path = no_links(self.roots[name] / safe_relative(ref["path"]))
        need(path.is_file(), "Missing artifact file")
        require(isinstance(ref["sha256"], str) and re.fullmatch("[0-9a-f]{64}", ref["sha256"]) and
                sha(path) == ref["sha256"], "Artifact SHA256 mismatch")
        return path

    def select(self, selector):
        closed(selector, ("file", "pointer"), "selector")
        return pointer(read_json(self.file(selector["file"])), selector["pointer"])

    def nested(self, selector):
        """Read an imported document with its authorized, fixed local root.

        The reference is validated in the caller before creating a context.
        Only the caller's existing root grants are inherited. A JSON document
        cannot add a directory grant or choose a filesystem parent as its root.
        """
        closed(selector, ("file", "pointer"), "nested selector")
        path = self.file(selector["file"])
        name = selector["file"].get("root", "evidence")
        inherited = {key: value for key, value in self.roots.items()
                     if key not in {"evidence", "sdk"}}
        child = Context(self.roots[name], self.sdk, inherited)
        return child, pointer(read_json(path), selector["pointer"])

    def author_index(self, selector):
        """Build selectors to untouched author files, without executing commands.

        Only new outer selectors name the import alias. Original proof and
        command references remain local to their explicitly selected root.
        This index is not an independent-author/C36 validation result.
        """
        require(self.author_roots_explicit, "EXPLICIT_AUTHOR_CONTEXT_ROOTS_REQUIRED")
        rooted_author_selector(selector)
        local, proof = self.nested(selector)
        require(isinstance(proof, dict) and type(proof.get("schemaVersion")) is int and proof["schemaVersion"] == 1
                and proof.get("kind") == "INDEPENDENT_AUTHOR_EVIDENCE_V1", "AUTHOR_REFERENCE_PROOF_KIND_REQUIRED")
        require(isinstance(proof.get("executions"), list) and proof["executions"], "AUTHOR_EXECUTIONS_REQUIRED")

        def outer(ref):
            rooted_author_reference(ref)
            original_path = local.file(ref)
            selected = {**ref, "root": selector["file"]["root"] if ref["root"] == "evidence" else ref["root"]}
            require(self.file(selected) == original_path, "AUTHOR_REFERENCE_CONTEXT_MISMATCH")
            return {"file": selected, "pointer": ""}

        executions = []
        for ref in proof["executions"]:
            selected = outer(ref)
            record = read_json(local.file(ref))
            require(isinstance(record, dict) and record.get("kind") == "ACTUAL_AUTHOR_COMMAND"
                    and type(record.get("exitCode")) is int and "argv" not in record
                    and isinstance(record.get("command"), list) and len(record["command"]) >= 2
                    and all(isinstance(v, str) and v for v in record["command"]), "AUTHOR_REFERENCE_COMMAND_REQUIRED")
            for field in ("stdout", "stderr", "executable"):
                rooted_author_reference(record.get(field))
                local.file(record[field])
            executions.append({"record": selected, "expectedExitCode": record["exitCode"]})
        invocation = outer(proof.get("invocation"))
        observer = Context(self.root, self.sdk, {k: v for k, v in self.roots.items() if k not in {"evidence", "sdk"}})
        observer.executions = executions
        observer.records()
        return {"schemaVersion": 1, "kind": "AUTHOR_JOINED_REFERENCE_INDEX_V1",
                "independentAuthorEvidence": {"file": dict(selector["file"]), "pointer": selector["pointer"]},
                "executions": executions, "newOperationInvocation": invocation,
                "sdkReady": False, "authorCommandsReexecuted": False}

    def legacy(self, value, expected_hash=None):
        """Original harness paths are data, never permission to read another root."""
        require(isinstance(value, str) and value, "Missing original artifact path")
        original = ordinary_path(value)
        if original.is_absolute():
            candidates = [original] if any(original.is_relative_to(p) for p in self.roots.values()) else []
        else:
            parts = value.replace("\\", "/")
            safe_relative(parts)
            candidates = [p / parts for p in self.roots.values() if (p / parts).is_file()]
        require(bool(candidates), "Original path is outside explicitly authorized roots")
        candidates = list(dict.fromkeys(candidates))
        require(len(candidates) == 1, "Ambiguous legacy relative path")
        path = no_links(candidates[0]); need(path.is_file(), "Missing original file")
        if expected_hash is not None: require(sha(path) == expected_hash, "Original artifact hash drift")
        return path

    def raw(self, name):
        need(name in self.artifacts, "Missing required artifact: " + name)
        return self.select(self.artifacts[name])

    def blob(self, name):
        selector = self.artifacts[name]
        require(selector["pointer"] == "", "Raw byte artifact cannot use JSON pointer")
        return self.file(selector["file"])

    def records(self):
        require(bool(self.executions), "No actual execution records")
        self.record_contexts = {}
        records = []
        for item in self.executions:
            require(isinstance(item, dict) and set(item) in ({"record", "expectedExitCode"},
                    {"record", "expectedExitCode", "retainedBinaries"}), "Invalid execution selector")
            nested, record = self.nested(item["record"])
            require(isinstance(record, dict), "Execution record must be object")
            require(("command" in record) != ("argv" in record), "Ambiguous original argv field")
            argv, code = record.get("command", record.get("argv")), record.get("exitCode")
            require(isinstance(argv, list) and len(argv) >= 2 and all(isinstance(s, str) and s for s in argv),
                    "Actual argv not captured")
            require(type(code) is int and type(item["expectedExitCode"]) is int and code == item["expectedExitCode"],
                    "Actual exit differs from required exit")
            if isinstance(record.get("stdout"), dict) and isinstance(record.get("stderr"), dict):
                for stream in ("stdout", "stderr"):
                    self.stream(record, stream, nested)
            elif isinstance(record.get("stdout"), str) and isinstance(record.get("stderr"), str):
                for stream in ("stdout", "stderr"):
                    need(stream + "Sha256" in record, "Original stream hash missing")
                    self.legacy(record[stream], record[stream + "Sha256"])
            else:
                need("log" in record and "sha256" in record, "Actual execution log/hash not captured")
                self.legacy(record["log"], record["sha256"])
            require(record.get("timedOut", False) is False and record.get("captureErrors", []) == []
                    and record.get("logBudgetExceeded", False) is False, "Incomplete execution capture")
            if "executedTestBinaries" in record:
                need(bool(record["executedTestBinaries"]), "Actual test binary identity was not captured")
                stdout = self.stream(record, "stdout", nested)
                artifacts = []
                for line in stdout.read_text(encoding="utf-8").splitlines():
                    try: value = json.loads(line)
                    except json.JSONDecodeError: continue
                    if isinstance(value, dict) and value.get("reason") == "compiler-artifact" and value.get("executable"):
                        artifacts.append(value)
                for binary in record["executedTestBinaries"]:
                    retained = [x for x in item.get("retainedBinaries", []) if x["originalPath"] == binary["path"]
                                and x["target"] == binary["target"]]
                    if retained:
                        require(len(retained) == 1 and retained[0]["file"]["sha256"] == binary["sha256"], "Retained binary identity changed")
                        self.file(retained[0]["file"])
                    elif "retainedPath" in binary:
                        # This path is measured by the original execution
                        # collector, not inserted into a historical record.
                        # It still needs an explicitly authorized artifact root.
                        self.legacy(binary["retainedPath"], binary["sha256"])
                    else:
                        self.legacy(binary["path"], binary["sha256"])
                    require(any(Path(a["executable"]) == Path(binary["path"]) and a["target"]["name"] == binary["target"]
                                for a in artifacts), "Test binary identity not bound to actual Cargo compiler-artifact")
            observed = {**record, "command": argv}
            self.record_contexts[id(observed)] = nested
            records.append(observed)
        return records

    def stream(self, record, name, nested=None):
        """Resolve typed streams in their original command selector's root.

        Two-field/string legacy streams keep the existing path authorization
        rule. Three-field references have one fixed local evidence context;
        failure never triggers a search in another root.
        """
        value = record[name]
        if isinstance(value, dict):
            require(set(value) in ({"path", "sha256"}, {"root", "path", "sha256"}),
                    "Invalid fields: original stream")
            if "root" in value:
                context = nested if nested is not None else self.record_contexts.get(id(record))
                require(context is not None, "Original command context required for typed stream")
                return context.file(value)
            return self.legacy(value["path"], value["sha256"])
        require(isinstance(value, str), "Invalid original stream")
        need(name + "Sha256" in record, "Original stream hash missing")
        return self.legacy(value, record[name + "Sha256"])

    def public(self, selector):
        response = self.select(selector)
        require(isinstance(response, dict) and response.get("protocolVersion") == 2 and
                isinstance(response.get("operation"), str), "Missing raw public Response")
        require(("error" in response) != ("dataJson" in response), "Invalid public response union")
        # Runtime chain receipts must be products of actual public CLI commands,
        # not free-standing JSON facts or stdout from a synthetic Python fixture.
        bound = False
        for record in self.checked_records:
            command = record["command"]
            if not self.runtime_command(record, response) or "--output" not in command:
                continue
            require(command.count("--output") == 1 and command.index("--output") + 1 < len(command), "Invalid CLI output flag")
            output = self.legacy(command[command.index("--output") + 1])
            if read_json(output) == response:
                require((record["exitCode"] != 0) == ("error" in response), "CLI exit/Response error disagree")
                bound = True
        need(bound, "Public Response has no matching actual CLI command/output capture")
        if "error" in response:
            require(isinstance(response["error"], dict) and isinstance(response["error"].get("code"), str), "Missing public error")
            return response, None
        require(isinstance(response["dataJson"], str), "Public dataJson must remain original string")
        return response, json.loads(response["dataJson"], object_pairs_hook=pairs)

    def runtime_command(self, record, response):
        """Resolve documented CLI aliases and the actual record execution path."""
        command = record["command"]
        if Path(command[0]).stem != "bgx-adapter-v2": return False
        verbs = {"capabilities": {"caps", "capabilities"}}
        if command[1] in verbs.get(response["operation"], {response["operation"]}): return True
        if command[1] != "record": return False
        for flag in ("--input", "--transcript"):
            require(command.count(flag) == 1 and command.index(flag) + 1 < len(command), "Invalid actual record flag")
        invocation = read_json(self.legacy(command[command.index("--input") + 1]))
        tape = read_json(self.legacy(command[command.index("--transcript") + 1]))
        require(tape["sha256"] == object_hash(tape["transcript"]), "Actual record tape digest differs")
        return invocation["operation"] == response["operation"] and any(
            item["invocation"] == invocation and item["response"] == response for item in tape["transcript"]["invocations"])

    def report_output(self, selector, verb):
        value = self.select(selector)
        for record in self.checked_records:
            command = record["command"]
            if Path(command[0]).stem == "bgx-adapter-v2" and command[1] == verb and "--output" in command:
                require(command.count("--output") == 1 and command.index("--output") + 1 < len(command), "Invalid actual output flag")
                if read_json(self.legacy(command[command.index("--output") + 1])) == value:
                    return value
        raise MissingProof("Structured report lacks actual CLI " + verb + " execution binding")

    def input_bindings(self, selector):
        response = self.select(selector)
        found = []
        for record in self.checked_records:
            command = record["command"]
            if not self.runtime_command(record, response):
                continue
            if "--output" not in command or "--input" not in command:
                continue
            require(command.count("--input") == command.count("--output") == 1, "Ambiguous actual input/output")
            output = self.legacy(command[command.index("--output") + 1])
            if read_json(output) == response:
                invocation = read_json(self.legacy(command[command.index("--input") + 1]))
                require(invocation["operation"] == response["operation"], "Command/input/response operation differs")
                found.append((record, invocation))
        need(bool(found), "Original command-bound Invocation not captured")
        return found

    def response_entries(self, name="responses"):
        rows = self.raw(name)
        require(isinstance(rows, list) and bool(rows), "Missing response entries")
        values = []
        for item in rows:
            closed(item, ("invocation", "response", "execution"), "response entry")
            invocation = self.select(item["invocation"])
            response, data = self.public(item["response"])
            require(response["operation"] == invocation["operation"], "Response operation differs from original input")
            index = item["execution"]
            require(type(index) is int and 0 <= index < len(self.checked_records), "Response has no actual execution binding")
            command = self.checked_records[index]["command"]
            require(self.runtime_command(self.checked_records[index], response) and
                    "--input" in command and "--output" in command, "Runtime request must bind an actual public CLI invocation")
            # CLI artifacts must match exact original invocation/output bytes.
            for flag, selected in (("--input", item["invocation"]), ("--output", item["response"])):
                if flag in command:
                    require(command.count(flag) == 1 and command.index(flag) + 1 < len(command), "Invalid command flag")
                    original = self.legacy(command[command.index(flag) + 1])
                    require(sha(original) == sha(self.file(selected["file"])), "CLI artifact differs from command-bound bytes")
            values.append((invocation, response, data))
        return values


def binary_identities(c, name="binaryIdentities"):
    value = c.raw(name)
    closed(value, ("before", "after"), "binary identities")
    require(isinstance(value["before"], dict) and value["before"] == value["after"] and
            {"cli", "worker"} <= set(value["before"]), "CLI/worker identities not captured unchanged")
    for role, ref in value["before"].items(): c.file(ref)
    return value


def media_facts(c, name="mediaFacts"):
    rows = c.raw(name)
    require(isinstance(rows, list), "Media facts must be list")
    identities = []
    for row in rows:
        closed(row, ("file", "stat", "public"), "media facts")
        path = c.file(row["file"]); measured, public = row["stat"], row["public"]
        require(measured["sha256"] == sha(path) and int(measured["byteLength"]) == path.stat().st_size,
                "Media bytes disagree with broker stat")
        require(measured["kind"] == public["kind"] and int(public["byteLength"]) == path.stat().st_size,
                "Public media differs from broker stat")
        for field in ("width", "height", "durationMs"):
            if field in public: require(public[field] == measured.get(field), "Public geometry/time differs from broker")
        identities.append((sha(path), measured["kind"]))
    return identities


def service_pair(value):
    closed(value, ("before", "after"), "service snapshots")
    require(isinstance(value["before"], dict) and isinstance(value["after"], dict), "Missing raw service states")
    return value["before"], value["after"]


def check_real_probe(c):
    fixture = c.blob("fixture"); require(fixture.stat().st_size > 0, "Empty probe fixture")
    ff = c.raw("ffprobeJson")
    pts = [Decimal(x["best_effort_timestamp_time"]) for x in ff["frames"] if "best_effort_timestamp_time" in x]
    require(len(pts) >= 3 and len({pts[i + 1] - pts[i] for i in range(len(pts) - 1)}) > 1,
            "Real ffprobe PTS do not prove VFR")
    facts = c.raw("brokerFacts")
    require(facts["timestampsMs"] == [int((x - pts[0]) * 1000) for x in pts],
            "Broker timestamps differ from real ffprobe facts")
    videos = [x for x in ff["streams"] if x["codec_type"] == "video"]
    require(len(videos) == 1 and facts["width"] == videos[0]["width"] and facts["height"] == videos[0]["height"],
            "Broker geometry differs from real probe")
    duration = videos[0].get("duration", ff.get("format", {}).get("duration"))
    need(duration is not None, "Raw real duration not captured")
    require(facts["durationMs"] == int(Decimal(str(duration)) * 1000), "Broker duration differs from real probe")
    capture = c.raw("validationResponse")
    require(capture["kind"] == "REAL_PROBE_GEOMETRY_EXECUTION" and capture["fixture"]["sha256"] == sha(fixture)
            and capture["measured"] == facts, "Broker imported different fixture bytes")
    cases = {x["widget"]: x for x in capture["checks"]}
    require(set(cases) == {"point", "box", "mask", "time_span"}, "Missing actual geometry boundary captures")
    require(cases["point"]["accepted"]["x"] < facts["width"] == cases["point"]["rejected"]["x"], "Point boundary not exercised")
    require(cases["box"]["accepted"]["width"] == facts["width"] < cases["box"]["rejected"]["width"], "Box boundary not exercised")
    require(cases["time_span"]["accepted"]["endMs"] == facts["durationMs"] < cases["time_span"]["rejected"]["endMs"], "Time boundary not exercised")
    require(cases["mask"]["accepted"]["maskMediaId"] != cases["mask"]["rejected"]["maskMediaId"] and
            all(x["foreignMediaRejected"] is True for x in cases.values()), "Mask/foreign identity negative missing")
    require(capture["restart"] == {"newHandleDistinct": True, "oldHandleRejected": True,
            "releasedHandleRejected": True, "timestampsPreserved": True}, "Media restart ownership negative missing")
    return {"fixtureSha256": sha(fixture), "frames": len(pts), "differentPtsDeltas": True}


def check_three_packages(c):
    binary_identities(c)
    packages = c.raw("packageIdentities")
    require(isinstance(packages, list) and len(packages) == 3, "Three actual verified packages required")
    ids = set()
    for row in packages:
        manifest = c.select(row["manifest"]); wasm = c.file(row["artifact"])
        require(manifest["language"] == "rust" and manifest["artifactSha256"] == sha(wasm), "Package artifact not actual WASM")
        require(wasm.open("rb").read(4) == b"\x00asm", "Artifact is not WASM")
        ids.add((manifest["pluginId"], manifest["version"]))
    require(len(ids) == 3, "Package identities are not distinct")
    caps = c.raw("capabilities")
    require(len(caps) == 3, "Capabilities for every actual package required")
    shared = []
    for selector in caps:
        _, value = c.public(selector)
        contracts = value["capabilityContracts"]
        video = next(x for x in contracts if x["capabilityKey"] == "video.generate")
        op = video["operations"][0]
        schema = next(s["schemaJson"] for s in video["schemas"] if s["id"] == op["inputSchema"]["id"])
        shared.append(hashlib.sha256(schema.encode()).hexdigest())
    require(len(set(shared)) == 1 and shared[0] == "832ea7f6ccf75f5af0d5cdd0b431ab96c6e7c969645deb962a739d372508bf6f",
            "Shared video input schema bytes differ")
    workflow = c.public(caps[2])[1]["capabilityContracts"]
    require({"video.generate", "video.speech", "video.edit"} <= {x["capabilityKey"] for x in workflow},
            "Workflow multi-capability declaration missing")
    require(len({op["id"] for x in workflow for op in x["operations"]}) >= 3, "Workflow operations not distinct")
    responses = c.response_entries()
    require(any(data and data.get("kind") == "completed" for _, _, data in responses), "No executed completed output")
    facts = media_facts(c); require(any(kind == "video" for _, kind in facts), "Actual video output missing")
    services = c.raw("serviceState")
    require(len(services) == 3 and all(s["ready"]["host"] == "127.0.0.1" and s["ready"]["port"] > 0 for s in services),
            "Independent actual service readiness missing")
    return {"packages": sorted(ids), "sharedVideoInputSchemaDigest": shared[0], "mediaCount": len(facts)}


def check_upload(c):
    original, execution = c.raw("originalInvocation"), c.raw("executionInvocation")
    left, right = json.loads(original["payloadJson"]), json.loads(execution["payloadJson"])
    require(set(original) == set(execution) and all(original[k] == execution[k] for k in original if k != "payloadJson"),
            "Upload changed original invocation identity")
    # Host lease replacement is the one permitted execution normalization.
    before = json.loads(json.dumps(left)); after = json.loads(json.dumps(right))
    for value in (before, after):
        for media in value["media"]: media["handle"] = "<host-rehydrated>"
    require(before == after, "Upload changed user fields beyond host handle")
    proof = c.raw("uploadProof"); facts = media_facts(c)
    require(facts and proof["sourceSha256"] in {h for h, _ in facts}, "Upload source digest differs from real media")
    results = c.response_entries()
    require(any(v and v.get("kind") == "accepted" for _, _, v in results), "Upload-backed submit not accepted")
    # Expiry belongs to a live host media handle used as an HTTP upload body.
    # Durable uploadRef provenance has no TTL contract. These actual native
    # broker errors must not be relabelled guest/CLI StandardResponses.
    boundary = c.select(proof["boundaryResult"])
    require(boundary["kind"] == "HOST_MEDIA_HTTP_BODY_BOUNDARY_RESULT" and boundary["standardResponse"] is False and
            boundary["uploadRefTtl"] == "NOT_DEFINED_NOT_TESTED", "Incorrect upload lease boundary scope")
    index = proof["boundaryExecution"]
    require(type(index) is int and 0 <= index < len(c.checked_records), "Missing actual native boundary execution")
    record = c.checked_records[index]; identity = c.select(proof["boundaryIdentity"])
    require(record["exitCode"] == 0 and record["command"][0] == identity["binary"]["path"],
            "Native boundary result lacks actual helper execution")
    c.legacy(identity["binary"]["path"], identity["binary"]["sha256"])
    require(read_json(c.stream(record, "stdout")) == boundary,
            "Native boundary facts differ from actual process stdout")
    negatives = boundary["negatives"]
    require(len(negatives) == 3 and {(r["case"], r["dimension"]) for r in negatives} ==
            {("foreign", "request"), ("foreign", "binding"), ("expired", "ttl")},
            "Missing actual lease expiry/ownership dimensions")
    lifecycle = boundary["lifecycle"]
    for row in negatives:
        require(row["kind"] == "HOST_MEDIA_HTTP_BODY_BOUNDARY" and row["error"]["code"] == "PERMISSION_DENIED" and
                row["httpRequestCountBefore"] == row["httpRequestCountAfter"] == 0 and
                row["executeRequest"]["method"] == "POST" and
                row["executeRequest"]["url"] == boundary["loopback"]["origin"] + "/upload",
                "Invalid lease did not fail before the actual upload request")
        expected = lifecycle["originalHandle"] if row["case"] == "foreign" else lifecycle["renewedHandle"]
        require(row["executeRequest"]["body"] == {"kind": "media", "handle": expected}, "Wrong actual upload body")
        if row["case"] == "foreign":
            field = "requestId" if row["dimension"] == "request" else "bindingId"
            require(set(row["scope"]) == set(row["owner"]) and row["scope"][field] != row["owner"][field] and
                    all(row["scope"][k] == row["owner"][k] for k in row["scope"] if k != field),
                    "Ownership negative did not isolate the declared scope dimension")
        else:
            require(row["scope"] == row["owner"] and type(row["ttlMs"]) is int and row["ttlMs"] > 0 and
                    type(row["elapsedMs"]) is int and row["elapsedMs"] >= row["ttlMs"], "Lease was not actually expired")
    require(boundary["loopback"]["acceptedConnections"] == 0 and boundary["loopback"]["threadJoined"] is True and
            lifecycle["oldHandleAfterReopenError"]["code"] == "PERMISSION_DENIED" and
            lifecycle["originalHandle"] != lifecycle["renewedHandle"], "Lease lifecycle/network ownership evidence missing")
    media = c.file(proof["boundaryMediaFile"])
    require(sha(media) == lifecycle["identity"]["digest"] == lifecycle["stat"]["sha256"] and
            media.stat().st_size == int(lifecycle["stat"]["byteLength"]), "Rehydrated actual bytes changed")
    return {"sourceSha256": proof["sourceSha256"], "normalizedOnlyHostHandles": True,
            "nativeUploadBodyLeaseNegatives": 3, "uploadRefTtl": "NOT_DEFINED_NOT_TESTED"}


def check_multi_output(c):
    values = c.response_entries(); facts = media_facts(c)
    require({"image", "audio"} <= {kind for _, kind in facts} and len({h for h, _ in facts}) >= 2,
            "Distinct delivered image/audio byte evidence missing")
    require(any(data and data.get("kind") == "completed" and len(data.get("outputs", [])) >= 2 for _, _, data in values),
            "Completed multi-output result missing")
    require(any("error" in response for _, response, _ in values), "Partial output terminal failure missing")
    state = c.raw("serviceState"); require(state["requestCount"] >= 2, "Actual multi-output service calls missing")
    normal, partial = c.select(state["normal"]), c.select(state["partial"])
    for scenario, row in (("normal", normal), ("partial_output", partial)):
        require(row["scenario"] == scenario and row["requestAfter"] == row["requestBefore"] + 1 and
                row["vendorRequest"]["method"] == "POST" and
                row["vendorRequest"]["requestJson"]["input"]["outputKinds"] == ["image", "audio"],
                "Multi-output request is not one actual dual-kind service call")
    require(normal["checkpoint"]["result"] == normal["actual"]["data"] and
            normal["checkpoint"]["state"] == "terminal" and
            {x["id"] for x in normal["checkpoint"]["mediaIdentities"]} ==
            {x["media"]["id"] for x in normal["actual"]["data"]["outputs"]},
            "Completed multi-output checkpoint differs from actual outputs")
    _, failed = c.public(state["asyncPartialResponse"])
    require(failed and failed["status"] == "failed" and isinstance(failed["terminalFailure"], dict) and
            bool(failed["outputs"]), "Failed async task did not retain output and terminal failure")
    flow = c.select(state["asyncPartialFlow"])
    require(flow["scenario"] == "partial_output" and flow["actual"]["data"] == failed,
            "Async partial output is not bound to its actual query flow")
    outputs = {x["media"]["id"]: x["media"] for x in failed["outputs"] if x["kind"] == "media"}
    retained = state["asyncPartialFiles"]
    require(retained and len(retained) == len(outputs) and {x["publicMediaId"] for x in retained} == set(outputs),
            "Retained partial-output files differ from public media identities")
    for item in retained:
        path = c.file(item["file"]); public = outputs[item["publicMediaId"]]
        require(path.stat().st_size == int(public["byteLength"]) and path.name == public["fileName"],
                "Retained partial bytes differ from actual public output")
    return {"media": facts, "completedCheckpointMatchesOutputs": True,
            "failedAsyncQueryRetainedOutputFiles": len(retained)}


def check_stream(c):
    observations = c.raw("observationTimeline")
    timing = observations["serviceTiming"]; first = timing["harnessFirstEventObservation"]
    times = [timing["requestReceivedMonotonicNs"], timing["firstFrameFlushedMonotonicNs"],
             first["harnessExplicitFsyncMonotonicNs"], timing["gateSetMonotonicNs"], timing["gateObservedMonotonicNs"],
             timing["lastFrameFlushedMonotonicNs"], timing["actualSocketWriteShutdownMonotonicNs"]]
    require(all(type(x) is int and x > 0 for x in times) and times == sorted(times) and times[2] < times[3] < times[-1]
            and first["cliAliveAfterFsync"] is True, "No actual fsynced event before gated socket EOF")
    event_file = c.blob("events")
    require(c.legacy(observations["eventFile"], observations["eventFileSha256"]) == event_file == c.legacy(first["path"]),
            "Event observation is not the retained CLI event file")
    need(event_file.stat().st_size <= 32 * 1024 * 1024, "Event evidence exceeds bounded JSONL size")
    lines = event_file.read_bytes().splitlines(keepends=True)
    require(bool(lines) and all(line.endswith(b"\n") for line in lines), "No actual complete JSONL events")
    events = [json.loads(line, object_pairs_hook=pairs) for line in lines]
    require(first["firstLineSha256"] == hashlib.sha256(lines[0]).hexdigest() and first["firstEvent"] == events[0]
            and len(lines[0]) <= first["fileSizeAtObservation"] <= event_file.stat().st_size,
            "Observed/fsynced first event bytes differ")
    require((first["fileDevice"], first["fileInode"]) == (event_file.stat().st_dev, event_file.stat().st_ino),
            "Observed CLI event file identity changed")
    sequences = [event["sequence"] for event in events]
    require(all(type(x) is int and x > 0 for x in sequences) and sequences == sorted(set(sequences)), "Event sequence duplicated/out of order")
    tape_path = c.legacy(observations["transcript"], observations["transcriptSha256"])
    tape = read_json(tape_path)
    require(tape["sha256"] == object_hash(tape["transcript"]), "Stream transcript digest changed")
    matches = [x for x in tape["transcript"]["invocations"] if x["invocation"]["requestId"] == observations["requestId"]]
    require(len(matches) == 1, "Stream tape original request missing/duplicated")
    entry = matches[0]; ports = entry["ports"]
    emits = [(i, p) for i, p in enumerate(ports) if p["call"]["method"] == "host-stream.emit-event"]
    reads = [i for i, p in enumerate(ports) if p["call"]["method"] == "host-http-stream.read"]
    require(bool(emits) and [p["call"]["args"]["event"] for _, p in emits] == events and
            all(p["result"] == {"ok": None} for _, p in emits), "Completed host emit/ACK results do not match persisted events")
    require(observations["firstEmitPortIndex"] == emits[0][0] and observations["successfulEmitResult"] == emits[0][1]["result"]
            and observations["readPortIndices"] == reads and min(reads) < emits[0][0] < max(reads),
            "First successful emit is not causally between actual reads")
    require(observations["allPortMethods"] == [p["call"]["method"] for p in ports] and
            any(p["result"].get("ok", {}).get("eof") is True for p in ports if p["call"]["method"] == "host-http-stream.read"),
            "Actual ordered stream EOF read missing")
    responses = c.response_entries()
    completed = [(inv, response, data) for inv, response, data in responses if inv == entry["invocation"] and response == entry["response"]]
    receipts = [row["response"] for row in c.raw("responses") if c.select(row["invocation"]) == entry["invocation"]
                and c.select(row["response"]) == entry["response"]]
    require(len(completed) == len(receipts) == 1 and completed[0][2]["kind"] == "completed" and
            observations["finalResponseSha256"] == sha(c.file(receipts[0]["file"])),
            "Final standard Response does not bind original record/tape")
    usage = [e["payload"] for e in events if e["type"] == "usage"]
    require(len(usage) >= 2 and usage == observations["duplicateUsageSnapshots"] and all(x == usage[0] for x in usage) and
            completed[0][2]["usage"] == usage[-1], "Reported duplicate usage was summed or estimated")
    require(timing["requestCount"] == 1, "Gated normal stream reconnected/retried")
    state = closed(c.raw("serviceState"), ("faults",), "stream fault index")
    scenarios = set()
    for selector in state["faults"]:
        fault = c.select(selector); scenarios.add(fault["scenario"])
        actual = fault["actual"]
        receipt = read_json(c.legacy(actual["response"]["path"], actual["response"]["sha256"]))
        invocation = read_json(c.legacy(actual["invocation"]["path"], actual["invocation"]["sha256"]))
        require(any(inv == invocation and response == receipt for inv, response, _ in responses) and "error" in receipt,
                "Stream fault has no actual command-bound terminal error")
        before, after = fault["stateBefore"]["proxy"], fault["stateAfter"]["proxy"]
        require(after["requestCount"] - before["requestCount"] == fault["observations"]["explicitInvocations"] == 1,
                "Fault stream retried/reconnected")
        raw_events = c.legacy(actual["eventsFile"]["path"], actual["eventsFile"]["sha256"])
        require([json.loads(line, object_pairs_hook=pairs) for line in raw_events.read_bytes().splitlines()] == actual["events"]
                and any(x["type"] == "text_delta" for x in actual["events"]), "Fault lost retained partial events")
        new = after["requests"][before["requestCount"]:]
        require(len(new) == 1 and new[0]["requestJson"]["scenario"] == fault["scenario"], "Fault remote request differs")
        c.legacy(new[0]["responseWire"]["path"], new[0]["responseWire"]["sha256"])
    require(scenarios == {"disconnect", "partial_output"}, "Missing actual disconnect/partial stream faults")
    return {"eventCount": len(events), "incrementalBeforeEof": True, "ackEvidence": "Completed ordered emit port results; no fabricated ACK timestamp."}


def check_recover(c):
    cp = c.raw("acceptedCheckpoint"); barrier = c.raw("killBarrier")
    require(cp["state"] == "accepted" and isinstance(cp["vendorTaskId"], str) and cp["vendorTaskId"] and
            cp.get("resumeToken") is not None, "Acceptance not durably resumable")
    require(barrier["durableCheckpoint"] == cp, "Kill barrier is not the committed checkpoint")
    require(barrier["kind"] == "sd28-durable-checkpoint-barrier" and
            barrier["point"] == "after-public-checkpoint-commit-before-port-reply-to-guest" and
            barrier["invocation"]["requestId"] == cp["requestId"] and
            barrier["successfulPortResult"]["revision"] == cp["revision"], "Barrier is not a successful durable public checkpoint commit")
    kills = [r for r in c.checked_records if r.get("pid") == barrier["helperPid"] and r.get("intentionalTermination") is True]
    require(len(kills) == 1 and kills[0]["exitCode"] != 0 and "barrier-submit" in kills[0]["command"] and
            any(x["pid"] == barrier["helperPid"] for x in kills[0]["reapedJobMembers"]) and
            any(Path(x["image"]).stem == "bgx-adapter-v2-worker" for x in kills[0]["reapedJobMembers"]),
            "Owned barrier helper/worker kill/reap not captured")
    before, after = c.raw("serviceStateBefore"), c.raw("serviceStateAfter")
    vendor = cp["vendorTaskId"]
    require(before["submitCount"] == after["submitCount"] == 1 and set(before["jobs"]) == set(after["jobs"]) == {vendor},
            "Recovery resubmitted or changed original vendor ID")
    response, data = c.public(c.artifacts["queryResponse"])
    require(response["operation"] == "query" and data["status"] == "succeeded",
            "Fresh query did not recover original accepted task")
    require(any(json.loads(invocation["payloadJson"])["vendorTaskId"] == vendor and
                record.get("pid") != barrier["helperPid"] and "--request-id" in record["command"] and
                record["command"][record["command"].index("--request-id") + 1] == cp["requestId"]
                for record, invocation in c.input_bindings(c.artifacts["queryResponse"])),
            "Fresh query did not use original taskRef vendor ID/request checkpoint")
    require(before["jobs"][vendor]["clientRequestId"] == cp["requestId"] == after["jobs"][vendor]["clientRequestId"],
            "Recovery request identity drift")
    binary_identities(c)
    return {"vendorTaskId": vendor, "submitCount": 1, "status": data["status"]}


def check_unknown(c):
    cp = c.raw("acceptedUnknownCheckpoint")
    require(cp["state"] == "acceptance_unknown" and cp["vendorTaskId"] is None and cp["resumeToken"] is None,
            "Unknown acceptance invented a remote ID/resume")
    before, after = c.raw("serviceStateBefore"), c.raw("serviceStateAfter")
    require(before["submitCount"] == after["submitCount"] == 1 and set(before["jobs"]) == set(after["jobs"]) and len(after["jobs"]) == 1,
            "Unknown acceptance retried submit")
    job = next(iter(after["jobs"].values()))
    require(job["queryCount"] == 0 and job["clientRequestId"] == cp["requestId"], "Out-of-band state leaked into recovery")
    results = c.response_entries()
    require(len(results) >= 2 and all(r.get("error", {}).get("code") == "ACCEPTANCE_UNKNOWN" for _, r, _ in results),
            "Unknown acceptance was falsely resolved")
    return {"submitCount": 1, "queryCount": 0, "recoveredVendorId": None}


def check_cancel(c):
    a, b = c.raw("jobA"), c.raw("jobB")
    require(a["vendorTaskId"] != b["vendorTaskId"] and a["requestId"] != b["requestId"], "Two original jobs are not distinct")
    anchors = closed(c.raw("serviceStateBefore"), ("kind", "acceptedA", "acceptedB", "bAfterCancelA"), "before-cancel checkpoint anchors")
    require(anchors["kind"] == "ACCEPTED_CHECKPOINTS_BEFORE_CANCEL", "Before-cancel evidence must remain explicitly checkpoint anchors")
    require(c.select(anchors["acceptedA"]) == a and c.select(anchors["acceptedB"]) == b == c.select(anchors["bAfterCancelA"])
            and a["state"] == b["state"] == "accepted", "Cancel A changed accepted sibling checkpoint B")
    after = c.raw("serviceStateAfter")
    aid, bid = a["vendorTaskId"], b["vendorTaskId"]
    require(after["submitCount"] == 2 and set(after["jobs"]) == {aid, bid},
            "Cancel/resume submitted another job")
    require(after["jobs"][aid]["cancelCount"] == 1 and after["jobs"][bid]["cancelCount"] == 0,
            "Monotone service cancel counters show wrong or sibling cancellation")
    require(after["jobs"][aid]["clientRequestId"] == a["requestId"] and after["jobs"][bid]["clientRequestId"] == b["requestId"],
            "Service jobs do not match the two original accepted checkpoints")
    _, cancel = c.public(c.artifacts["cancelResponse"])
    _, query = c.public(c.artifacts["queryBResponse"])
    require(cancel["outcome"] == "remote_confirmed" and after["jobs"][aid]["status"] == "cancelled" and
            query["status"] == "succeeded" and after["jobs"][bid]["status"] == "succeeded",
            "Cancellation confirmation or sibling completion not proven")
    for selector, vendor, original in ((c.artifacts["cancelResponse"], aid, a["requestId"]),
                                       (c.artifacts["queryBResponse"], bid, b["requestId"])):
        require(any(json.loads(invocation["payloadJson"])["vendorTaskId"] == vendor and
                    "--request-id" in record["command"] and
                    record["command"][record["command"].index("--request-id") + 1] == original
                    for record, invocation in c.input_bindings(selector)), "Cancel/query crossed original taskRef identity")
    return {"cancelledOriginal": aid, "unaffectedOriginal": bid, "cancelCountB": 0}


def check_replay(c):
    tape = c.raw("transcript"); report = c.report_output(c.artifacts["replayReport"], "replay"); counters = c.raw("networkCounters")
    require(tape["transcript"]["scope"] == "MOCK" and tape["sha256"] == object_hash(tape["transcript"]), "Transcript identity mismatch")
    require(report["scope"] == "MOCK" and report["transcriptSha256"] == tape["sha256"], "Actual replay transcript differs")
    closed(counters, ("capture", "mutations"), "replay counter index")
    captured = c.select(counters["capture"])
    require(captured["networkRequestCountAfterRecord"] == captured["networkRequestCountAfterReplayAndMutations"] and
            captured["replayReportSha256"] == sha(c.file(c.artifacts["replayReport"]["file"])), "Replay performed real network calls or refers to another report")
    require(len(report["invocations"]) == len(tape["transcript"]["invocations"]), "Replay invocation count differs")
    response_schema = read_json(c.sdk / "schemas/response.schema.json")
    def response_hash(value):
        # Replay hashes serde's generated Response struct bytes, whereas tape
        # hashes sort logical JSON maps. Generator field order comes from this
        # same public schema; do not invent a second wire field-order table.
        def ordered(node, item):
            if "oneOf" in node:
                matches = [x for x in node["oneOf"] if isinstance(item, dict) and set(x.get("properties", {})) == set(item)]
                require(len(matches) == 1, "Unknown recorded Response branch")
                return ordered(matches[0], item)
            if isinstance(item, dict) and "properties" in node:
                require(set(item) == set(node["properties"]), "Unknown recorded Response fields")
                return {key: ordered(child, item[key]) for key, child in node["properties"].items()}
            return item
        return hashlib.sha256(json.dumps(ordered(response_schema, value), separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    for expected, actual in zip(tape["transcript"]["invocations"], report["invocations"]):
        require(actual["responseSha256"] == response_hash(expected["response"]) and actual["portCalls"] == len(expected["ports"]) and
                actual["requestId"] == expected["invocation"]["requestId"] and actual["workerPid"] > 0,
                "Replay not exact ordered worker execution")
    mutations = counters["mutations"]
    names = {"unknown-field", "bad-hash", "other-package", "missing-port", "additional-port", "changed-call", "changed-response"}
    require(set(mutations) == names, "Missing actual replay mutation negatives")
    require(len({v["execution"] for v in mutations.values()}) == len(names), "Mutation failures reused one execution")
    for name, entry in mutations.items():
        closed(entry, ("execution", "transcript"), "replay mutation")
        require(type(entry["execution"]) is int and 0 <= entry["execution"] < len(c.checked_records), "Invalid mutation execution")
        actual = c.checked_records[entry["execution"]]; command = actual["command"]
        require(actual["exitCode"] != 0 and Path(command[0]).stem == "bgx-adapter-v2" and command[1] == "replay"
                and command.count("--transcript") == 1, "Replay mutation did not actually fail")
        path = c.legacy(command[command.index("--transcript") + 1])
        require(path == c.file(entry["transcript"]["file"]) and entry["transcript"]["pointer"] == "", "Mutation tape differs from actual command")
        altered = c.select(entry["transcript"])
        original_rows = [x for x in captured["mutations"] if x["name"] == name]
        require(len(original_rows) == 1 and original_rows[0]["sha256"] == sha(path) and
                original_rows[0]["exitCode"] == actual["exitCode"] and
                original_rows[0]["networkRequestCount"] == captured["networkRequestCountAfterRecord"], "Mutation counter/exit/tape does not match original capture")
        if name == "unknown-field":
            require(set(altered) > set(tape) and all(altered[k] == v for k, v in tape.items()), "Unknown-field mutation not isolated")
        elif name == "bad-hash":
            require(set(altered) == set(tape) and altered["transcript"] == tape["transcript"] and altered["sha256"] != tape["sha256"],
                    "Bad-hash mutation not isolated")
        else:
            closed(altered, ("sha256", "transcript"), "mutated tape")
            require(altered["sha256"] == object_hash(altered["transcript"]), "Logical mutation failed only because hash was stale")
            before, after = tape["transcript"], altered["transcript"]
            require(set(before) == set(after), "Unknown mutation transcript field")
            changed = [key for key in before if before[key] != after[key]]
            if name == "other-package":
                require(changed == ["package"] and set(before["package"]) == set(after["package"]), "Other-package mutation not isolated")
                continue
            require(changed == ["invocations"] and len(before["invocations"]) == len(after["invocations"]), "Runtime mutation not isolated")
            changed_rows = [(a, b) for a, b in zip(before["invocations"], after["invocations"]) if a != b]
            require(len(changed_rows) == 1, "Mutation changed several invocations")
            a, b = changed_rows[0]
            field = "response" if name == "changed-response" else "ports"
            require(set(a) == set(b) and a[field] != b[field] and all(a[k] == b[k] for k in a if k != field), "Mutation changed unrelated invocation fields")
            if name == "missing-port":
                require(len(a["ports"]) == len(b["ports"]) + 1 and any(a["ports"][:i] + a["ports"][i + 1:] == b["ports"]
                        for i in range(len(a["ports"]))), "Missing-port mutation not one missing ordered port")
            elif name == "additional-port":
                require(len(b["ports"]) == len(a["ports"]) + 1 and any(b["ports"][:i] + b["ports"][i + 1:] == a["ports"]
                        for i in range(len(b["ports"]))), "Additional-port mutation not one added ordered port")
            elif name == "changed-call":
                require(len(a["ports"]) == len(b["ports"]), "Changed-call mutation changed port count")
                changed_ports = [(x, y) for x, y in zip(a["ports"], b["ports"]) if x != y]
                require(len(changed_ports) == 1 and changed_ports[0][0]["result"] == changed_ports[0][1]["result"]
                        and changed_ports[0][0]["call"] != changed_ports[0][1]["call"], "Changed-call mutation altered result or several calls")
    return {"source": "ACTUALLY_EXECUTED", "invocations": len(report["invocations"]), "networkDelta": 0}


def check_profile(c):
    profile, code = c.raw("profileResponse"), c.raw("codeResponse")
    require(profile["kind"] == code["kind"] == "PROFILE_NATIVE_PUBLIC_SOURCE_PARITY" and
            profile["profileRuntime"] == "profile" and code["codeRuntime"] == "NATIVE_PUBLIC_RUST_SOURCE",
            "Profile/native/WASM runtime identity must remain explicit")
    require(profile["rustSourceSha256"] == code["rustSourceSha256"] == sha(c.sdk / "templates/rust/src/lib.rs"),
            "Native author source differs from actual captured code")
    require(len(profile["observations"]) == len(code["observations"]) == 3, "Actual capabilities/probe/submit comparison missing")
    for left, right in zip(profile["observations"], code["observations"]):
        require(left["invocation"] == right["invocation"] and left["profileResponse"] == right["codeResponse"],
                "Profile/code received different input or returned different result")
        require(left["profileResponse"]["operation"] == left["invocation"]["operation"], "Parity operation identity differs")
    require({x["invocation"]["operation"] for x in profile["observations"]} == {"capabilities", "probe", "submit"}, "Parity operations missing")
    require(profile["codeHostCalls"] == [] and len(profile["profileHostCalls"]) == 1, "Native/profile effect scope misreported")
    http = c.raw("serviceState")
    require(set(http) == {"bearer", "custom-header"}, "Both actual credential modes must be captured")
    for mode, selector in http.items():
        capture = c.select(selector)
        require(capture["kind"] == "PROFILE_ACTUAL_HTTP_BROKER" and capture["runtime"] == "profile", "Actual profile HTTP scope missing")
        payload = json.loads(capture["invocation"]["payloadJson"])
        result = json.loads(capture["response"]["dataJson"])
        require(capture["service"]["body"] == payload["input"] == result["data"] and
                capture["service"]["requestCount"] == 1 and capture["service"]["authMode"] == mode,
                "Separate actual profile HTTP body/result/counter differs")
        calls = [x for x in capture["hostCalls"] if x["method"] == "host-http.execute"]
        require(len(calls) == 1 and calls[0]["args"]["credential"] == payload["configuration"]["credentialRef"] and
                calls[0]["args"]["plan"]["generatedHeaders"] == [{"kind": mode, "name": capture["service"]["headerName"],
                    "targetField": capture["service"]["headerName"]}], "Credential lowering bypassed existing broker")
        require(capture["service"]["authorizationVerified"] is True, "Actual server did not verify vault authorization")
    return {"profileRuntime": "profile", "codeRuntime": code["codeRuntime"], "comparisons": len(profile["observations"]),
            "loopbackScope": "Separate actual profile HTTP/vault execution; no claim of native Rust HTTP or WASM execution."}


def check_form(c):
    path = c.blob("exportedInvocation"); original = read_json(path)
    request_hash = sha(path)
    identity = c.raw("inputIdentity"); execution = c.raw("executionIdentity")
    closed(identity, ("requestSha256", "payloadSha256"), "CLI original input identity")
    closed(execution, ("payloadSha256",), "CLI normalized execution identity")
    payload_hash = hashlib.sha256(original["payloadJson"].encode("utf-8")).hexdigest()
    require(identity["requestSha256"] == request_hash and identity["payloadSha256"] == payload_hash,
            "Browser/CLI original request or decoded payload identity differs")
    require(execution["payloadSha256"] == identity["payloadSha256"], "No-media browser payload changed before execution")
    response, data = c.public(c.artifacts["response"])
    require(response["operation"] == original["operation"] and data is not None, "Browser invocation not actually executed")
    evidence = closed(c.raw("browserEvidence"), ("export", "presentation", "screenshot", "joined"), "browser evidence index")
    browser = c.select(evidence["export"])
    require(isinstance(browser["exports"], list) and any(row["requestSha256"] == request_hash and
            row["payloadSha256"] == payload_hash for row in browser["exports"]), "Original actual browser download not captured")
    require(browser["errors"] == [] and len(set(browser["distinctCapabilities"])) >= 2, "Browser runtime or capability projection failed")
    require(all(row["method"] == "GET" for row in browser["requests"]), "Form submitted network requests instead of exporting")
    presentation = c.select(evidence["presentation"])
    require(presentation["scope"] == "ACTUAL_BROWSER_STANDARD_RESPONSE_PRESENTATION" and
            presentation["receiptSHA256"] == sha(c.file(c.artifacts["response"]["file"])) and
            presentation["errors"] == [] and presentation["privateWireFieldsAdded"] is False,
            "Actual standard response presentation did not bind the CLI receipt")
    browser_execution = False
    for record in c.checked_records:
        if Path(record["command"][0]).stem == "node" and record["exitCode"] == 0 and "log" in record:
            if read_json(c.legacy(record["log"], record["sha256"])) == presentation:
                browser_execution = True
    need(browser_execution, "Browser receipt presentation has no original actual Node/browser execution")
    screenshot = c.file(evidence["screenshot"])
    with screenshot.open("rb") as stream:
        require(stream.read(8) == b"\x89PNG\r\n\x1a\n", "Browser screenshot artifact missing/invalid")
    joined = c.select(evidence["joined"])
    need(isinstance(joined.get("sourceAndBuildHashes"), dict) and bool(joined["sourceAndBuildHashes"]),
         "Browser source/built asset identities were not captured")
    prefix = "packages/vendor-adapter-contract/v2/"
    for name, expected in joined["sourceAndBuildHashes"].items():
        require(name.startswith(prefix), "Browser captured asset outside public SDK")
        path = no_links(c.sdk / safe_relative(name[len(prefix):]))
        require(sha(path) == expected, "Captured browser source/build asset changed")
    return {"requestSha256": request_hash, "payloadSha256": payload_hash, "browserExportMatchesCliInput": True}


def check_matrix(c):
    scenarios = c.raw("scenarioResults"); expected = read_json(c.sdk / "fixtures/services/scenarios.json")
    names = expected["scenarios"] if isinstance(expected["scenarios"], list) else list(expected["scenarios"])
    names = [row["id"] if isinstance(row, dict) else row for row in names]
    require(isinstance(scenarios, list) and len(scenarios) == len({x["scenario"] for x in scenarios}), "Duplicate fault scenario")
    require(set(names) <= {x["scenario"] for x in scenarios}, "Required fault scenario missing")
    for row in scenarios:
        require(type(row["execution"]) is int and 0 <= row["execution"] < len(c.checked_records), "Fault scenario not actually executed")
        response, data = c.public(row["response"])
        record = c.checked_records[row["execution"]]; command = record["command"]
        require(c.runtime_command(record, response) and command.count("--output") == 1 and
                read_json(c.legacy(command[command.index("--output") + 1])) == response, "Scenario response does not bind selected actual execution")
        require(row["expectedError"] == response.get("error", {}).get("code"), "Fault did not produce declared structured outcome")
        if row["scenario"] == "partial_output" and response["operation"] == "query":
            require(data and data["status"] == "failed" and data["terminalFailure"] is not None,
                    "Async partial output did not retain standard terminal failure")
        elif row["scenario"] in ("disconnect", "partial_output", "acceptance_unknown"):
            require("error" in response, "Transport EOF/fault became success")
    require(c.raw("sourceClosure") == source_snapshot(c.sdk), "Fault matrix source closure differs")
    negatives = c.raw("missingCaseRejection")
    required = {"missing", "duplicate", "unknown", "nonzero", "hash_mismatch", "not_run", "replay_not_executed"}
    require(set(negatives) == required, "Missing fail-closed conform negatives")
    for negative_name, row in negatives.items():
        require(type(row["execution"]) is int and 0 <= row["execution"] < len(c.checked_records), "Invalid conform negative execution")
        record = c.checked_records[row["execution"]]; command = record["command"]
        require(record["exitCode"] != 0 and Path(command[0]).stem == "bgx-adapter-v2" and
                command[1] == "conform" and command.count("--output") == 1,
                "Invalid matrix declaration did not actually fail")
        if "standardError" in row:
            closed(row, ("execution", "standardError"), "conform structural rejection")
            require(negative_name in {"unknown", "duplicate"}, "Unexpected conform structural rejection")
            error = c.select(row["standardError"])
            closed(error, ("code", "path", "message", "hint"), "actual CLI structural error")
            require(error["code"] == "INVALID_INPUT" and
                    c.file(row["standardError"]["file"]) == c.stream(record, "stderr"),
                    "Structural error does not bind actual conform stderr")
            require(not ordinary_path(command[command.index("--output") + 1]).exists(),
                    "Structural conform error unexpectedly has an output report")
        else:
            closed(row, ("execution", "report"), "conform report rejection")
            report = c.select(row["report"])
            require(read_json(c.legacy(command[command.index("--output") + 1])) == report and report["conformant"] is False,
                    "Invalid matrix declaration did not produce a failed actual report")
    return {"scenarios": sorted(names), "conformNegativeCases": sorted(required)}


def trusted_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value)
    return value


def check_candidate(c):
    inputs = c.raw("candidateInventory")
    closed(inputs, ("candidateA", "candidateB", "evidenceRoot", "evidence"), "candidate inputs")
    for name in ("candidateA", "candidateB", "evidenceRoot"):
        require(inputs[name] in c.roots, "Candidate directory must be explicitly authorized as artifact root")
    release_evidence_path = c.file(inputs["evidence"])
    release_evidence = read_json(release_evidence_path)
    # The release verifier also supports standalone absolute-path evidence. When
    # imported into this runner, those embedded paths do not acquire permission:
    # constrain every build root/receipt/input/output/executable first.
    for build in release_evidence["builds"]:
        source = ordinary_path(build["sourceRoot"])
        require(source.is_absolute() and any(source.is_relative_to(base) for base in c.roots.values()),
                "Build source root not explicitly authorized")
        no_links(source)
        for binary in build["binaries"].values(): c.legacy(binary)
        c.file({"root": inputs["evidenceRoot"], **build["toolchain"]})
        for ref in build["executions"]:
            record = read_json(c.file({"root": inputs["evidenceRoot"], **ref}))
            c.legacy(record["command"][0], record["executableSha256"])
            cwd = ordinary_path(record["cwd"])
            require(cwd.is_absolute() and any(cwd.is_relative_to(base) for base in c.roots.values()), "Build cwd not authorized")
            no_links(cwd)
            for rows in (record["inputsBefore"], record["inputsAfter"], record["outputs"]):
                for row in rows: c.legacy(row["path"], row["sha256"])
            for name in ("stdout", "stderr"):
                c.file({"root": inputs["evidenceRoot"], **record[name]})
    verifier = trusted_module("sdk_conformance_release", Path(__file__).with_name("verify_release.py"))
    try:
        measured = verifier.assess(c.roots[inputs["candidateA"]], c.roots[inputs["candidateB"]],
                                   c.roots[inputs["evidenceRoot"]], release_evidence_path, True)["reproduction-proof.json"]
    except verifier.exporter.ExportError as error:
        raise InvalidProof("Candidate reproduction proof rejected: " + str(error)) from error
    saved = c.raw("twoBuildIdentities")
    require(saved == measured, "Saved reproduction proof differs from current measured two-build identities")
    isolation = c.raw("isolationEvidence")
    require(isolation["violations"] == [] and isolation["protectedBefore"] == isolation["protectedAfter"] and
            isinstance(isolation["protectedBefore"], list) and bool(isolation["protectedBefore"]), "Protected source/data isolation not proven")
    return {"candidateInventories": measured["candidateInventories"], "byteIdentical": measured["byteIdentical"],
            "phase": "Measured reproduction proof only; no G06/SDK_READY bootstrap."}


def check_external_author(c):
    need("independentAuthorEvidence" in c.artifacts, "Missing required artifact: independentAuthorEvidence")
    index = c.author_index(c.artifacts["independentAuthorEvidence"])
    require(c.executions == index["executions"], "AUTHOR_EXECUTION_INDEX_MISMATCH")
    require(c.artifacts.get("newOperationInvocation") == index["newOperationInvocation"], "AUTHOR_INVOCATION_INDEX_MISMATCH")
    need("binaryIdentities" in c.artifacts, "Missing required artifact: binaryIdentities")
    rooted_author_selector(c.artifacts["binaryIdentities"])
    identities = c.raw("binaryIdentities")
    closed(identities, ("before", "after"), "binary identities")
    for phase in ("before", "after"):
        require(isinstance(identities[phase], dict), "CLI/worker identities not captured unchanged")
        for ref in identities[phase].values(): rooted_author_reference(ref)
    nested, evidence = c.nested(c.artifacts["independentAuthorEvidence"])
    need(Path(__file__).with_name("check_author_evidence.py").is_file(), "Independent author verifier not implemented yet")
    verifier = trusted_module("sdk_conformance_author", Path(__file__).with_name("check_author_evidence.py"))
    need(callable(getattr(verifier, "verify_evidence", None)), "Independent author checker verify_evidence API missing")
    try:
        # The author's original implicit evidence references belong to the root
        # explicitly selected for this imported proof. Passing the outer
        # reserved evidence alias would override the checker's root argument.
        result = verifier.verify_evidence(nested.root, evidence,
                                         {name: root for name, root in nested.roots.items() if name != "evidence"})
    except verifier.exporter.ExportError as error:
        raise InvalidProof("Independent author proof rejected: " + str(error)) from error
    require(isinstance(result, dict) and result.get("caseId") == "C36" and result.get("status") == "PASS", "Independent author evidence rejected")
    binary_identities(c)
    invocation = c.raw("newOperationInvocation")
    require(invocation["operation"] == "submit" and "/" in json.loads(invocation["payloadJson"])["operationId"],
            "Independent author's real namespaced operation invocation missing")
    return {"source": "INDEPENDENT_AUTHOR", "verifierResult": result}


VALIDATORS = dict(zip(IDS, (check_real_probe, check_three_packages, check_upload, check_multi_output, check_stream,
                          check_recover, check_unknown, check_cancel, check_replay, check_profile, check_form,
                          check_matrix, check_candidate, check_external_author)))


def verify(root, reference, chain, sdk, artifact_roots=None):
    c = Context(root, sdk, artifact_roots)
    proof = read_json(c.file(reference))
    closed(proof, ("schemaVersion", "kind", "chainId", "mode", "provenance", "sourceClosure", "executions", "artifacts"), "joined proof")
    require(proof["schemaVersion"] == 1 and proof["kind"] == "JOINED_CHAIN_PROOF_V1" and
            proof["chainId"] == chain["id"] and proof["mode"] == "MOCK", "Wrong joined proof identity")
    require(proof["provenance"] in ("CAPTURED_EXECUTION", "REVIEWED_EXISTING_EXECUTION"), "Unknown execution provenance")
    closure = closed(proof["sourceClosure"], ("current", "captured", "review"), "source closure")
    now = source_snapshot(c.sdk)
    require(closure["current"] == now, "Joined proof source closure changed")
    if proof["provenance"] == "CAPTURED_EXECUTION":
        require(closure["captured"] == now and closure["review"] is None, "Captured source closure missing or changed")
    else:
        require(isinstance(closure["captured"], list) and bool(closure["captured"]), "No historical captured source hashes")
        require(all(isinstance(x, dict) and set(x) == {"path", "sha256"} for x in closure["captured"]) and
                len({x["path"] for x in closure["captured"]}) == len(closure["captured"]), "Duplicate/invalid selected source row")
        current = {x["path"]: x["sha256"] for x in now}
        require(all(current.get(x["path"]) == x["sha256"] for x in closure["captured"]), "Recorded historical sources differ")
        review = c.select(closure["review"])
        closed(review, ("kind", "currentDigest", "capturedDigest", "capturedSource", "unmeasuredAtExecution", "analysis"), "source review")
        require(review["kind"] == "SOURCE_REVIEW_V1" and review["currentDigest"] == object_hash(now) and
                review["capturedDigest"] == object_hash(closure["captured"]), "Reviewed source identities differ")
        require(isinstance(review["analysis"], str) and bool(review["analysis"].strip()) and
                isinstance(review["unmeasuredAtExecution"], list) and bool(review["unmeasuredAtExecution"]) and
                all(isinstance(x, str) and bool(x.strip()) for x in review["unmeasuredAtExecution"]), "Historical limitations must be explicit")
        original = c.select(review["capturedSource"])
        if isinstance(original, list):
            require(all(isinstance(x, dict) and set(x) == {"path", "sha256"} for x in original), "Invalid original source rows")
            require(len({x["path"] for x in original}) == len(original), "Duplicate original source path")
            original = {x["path"]: x["sha256"] for x in original}
        require(isinstance(original, dict) and bool(original), "Original captured source map missing")
        captured = {}
        prefix = "packages/vendor-adapter-contract/v2/"
        for name, digest in original.items():
            require(isinstance(name, str) and isinstance(digest, str) and re.fullmatch("[0-9a-f]{64}", digest), "Invalid original source identity")
            absolute = ordinary_path(name)
            if absolute.is_absolute():
                if not absolute.is_relative_to(c.sdk): continue
                name = absolute.relative_to(c.sdk).as_posix()
            else:
                name = name.replace("\\", "/")
                if name.startswith(prefix): name = name[len(prefix):]
            name = safe_relative(name)
            require(name not in captured or captured[name] == digest, "Conflicting original source identity")
            captured[name] = digest
        require(all(captured.get(x["path"]) == x["sha256"] for x in closure["captured"]),
                "Reviewed source subset was not measured in original capture")
    require(isinstance(proof["artifacts"], dict) and set(proof["artifacts"]) <= set(chain["requiredArtifacts"]),
            "Unknown typed joined artifacts")
    need(set(proof["artifacts"]) == set(chain["requiredArtifacts"]), "Required typed joined artifacts not captured")
    c.artifacts, c.executions = proof["artifacts"], proof["executions"]
    c.checked_records = c.records()
    require(chain["id"] in VALIDATORS, "Unknown fixed joined chain")
    facts = VALIDATORS[chain["id"]](c)
    return {"chainId": chain["id"], "source": proof["provenance"], "proof": reference,
            "sourceDigest": object_hash(now), "executions": len(c.checked_records), "facts": facts}


def composition_self_test(root, sdk):
    """Local context fixtures only, with no claim of author/runtime execution."""
    base = root / "composition-fixtures"; base.mkdir(exist_ok=False)
    outer, original, ungranted = [base / name for name in ("outer", "original", "ungranted")]
    for directory in (outer, original, ungranted): directory.mkdir()
    (outer / "stdout.bin").write_bytes(b"outer-decoy\n")
    (outer / "stderr.bin").write_bytes(b"outer-error-decoy\n")
    (original / "stdout.bin").write_bytes(b"original-local-output\n")
    (original / "stderr.bin").write_bytes(b"original-local-error\n")
    (ungranted / "private.bin").write_bytes(b"not-authorized\n")
    def reference(path, name="evidence", folder=original):
        return {"root": name, "path": path.relative_to(folder).as_posix(), "sha256": sha(path)}
    def save(path, value):
        path.write_text(json.dumps(value), encoding="utf-8")
        return reference(path, "originalRoot")
    output = reference(original / "stdout.bin")
    # These are infrastructure command-shaped fixtures, not invented actual
    # author executions. The global runner self-test keeps every case NOT_RUN.
    command = {"command": ["fixture-only-no-process", "probe"], "exitCode": 0,
               "stdout": output, "stderr": reference(original / "stderr.bin"), "timedOut": False}
    command_path = original / "command.json"
    command_ref = save(command_path, command)
    proof_path = original / "proof.json"
    proof_ref = save(proof_path, {"stream": output})
    originals = {p: p.read_bytes() for p in (command_path, proof_path, original / "stdout.bin", original / "stderr.bin")}
    c = Context(outer, sdk, {"originalRoot": original, "author": original})
    selector = {"file": proof_ref, "pointer": ""}
    local, proof = c.nested(selector)
    require(local.root == original and local.file(proof["stream"]) == original / "stdout.bin"
            and local.roots["author"] == original, "Nested original proof lost its authorized local root")
    checks = ["composition_untouched_nested_proof_uses_selector_authorized_root"]
    c.executions = [{"record": {"file": command_ref, "pointer": ""}, "expectedExitCode": 0}]
    rows = c.records()
    require(c.stream(rows[0], "stdout").read_bytes() == b"original-local-output\n"
            and c.stream(rows[0], "stderr").read_bytes() == b"original-local-error\n",
            "Outer reserved evidence overrode original command stream context")
    checks.append("composition_original_typed_command_streams_ignore_outer_collision")
    def reject(label, action, missing=False):
        try: action()
        except MissingProof:
            require(missing, "Invalid composition misclassified missing: " + label)
        except (InvalidProof, ValueError, KeyError):
            require(not missing, "Missing composition misclassified invalid: " + label)
        else: raise InvalidProof("Composition negative falsely accepted: " + label)
        checks.append(label)
    reject("composition_ungranted_selector_root_rejected",
           lambda: c.nested({"file": {**proof_ref, "root": "ungranted"}, "pointer": ""}))
    claims_path = original / "claimed-roots.json"
    claims_ref = save(claims_path, {"artifactRoots": {"ungranted": str(ungranted)},
                                  "stream": {"root": "ungranted", "path": "private.bin",
                                             "sha256": sha(ungranted / "private.bin")}})
    claimed, selected = c.nested({"file": claims_ref, "pointer": ""})
    reject("composition_nested_json_cannot_grant_directory",
           lambda: claimed.file(selected["stream"]))
    (outer / "proof.json").write_bytes(proof_path.read_bytes())
    wrong, unchanged = c.nested({"file": {**proof_ref, "root": "evidence"}, "pointer": ""})
    reject("composition_outer_collision_cannot_rebase_to_original",
           lambda: wrong.file(unchanged["stream"]))
    subdir = original / "subdir"; subdir.mkdir()
    (subdir / "only-child.bin").write_bytes(b"file-parent-is-not-an-authorization\n")
    child_proof = subdir / "proof.json"
    save(child_proof, {"stream": reference(subdir / "only-child.bin", folder=subdir)})
    child, selected = c.nested({"file": reference(child_proof, "originalRoot"), "pointer": ""})
    reject("composition_proof_file_parent_never_inferred",
           lambda: child.file(selected["stream"]), True)
    projected = outer / "projected.json"
    projected.write_text(json.dumps({"stream": {**output, "root": "originalRoot"}}), encoding="utf-8")
    same, selected = c.nested({"file": reference(projected, folder=outer), "pointer": ""})
    require(same.root == outer and same.file(selected["stream"]) == original / "stdout.bin",
            "Existing same-root explicitly authorized projection lost its meaning")
    checks.append("composition_existing_same_root_explicit_projection_preserved")
    explicit_path = original / "explicit-granted-stream.json"
    explicit_ref = save(explicit_path, {**command, "stdout": {**output, "root": "author"}})
    c.executions = [{"record": {"file": explicit_ref, "pointer": ""}, "expectedExitCode": 0}]
    rows = c.records()
    require(c.stream(rows[0], "stdout") == original / "stdout.bin", "Explicit granted stream root lost")
    checks.append("composition_explicit_granted_stream_alias_preserved")
    legacy_path = original / "legacy-stream.json"
    legacy_ref = save(legacy_path, {**command,
        "stdout": {"path": str(original / "stdout.bin"), "sha256": sha(original / "stdout.bin")},
        "stderr": {"path": str(original / "stderr.bin"), "sha256": sha(original / "stderr.bin")}})
    c.executions = [{"record": {"file": legacy_ref, "pointer": ""}, "expectedExitCode": 0}]
    rows = c.records()
    require(c.stream(rows[0], "stdout") == original / "stdout.bin", "Legacy stream authorization changed")
    checks.append("composition_two_field_legacy_stream_rule_preserved")
    bad_path = original / "unknown-stream-field.json"
    save(bad_path, {**command, "stdout": {**output, "directory": str(ungranted)}})
    c.executions = [{"record": {"file": reference(bad_path, "originalRoot"), "pointer": ""}, "expectedExitCode": 0}]
    reject("composition_unknown_typed_stream_field_rejected", c.records)
    bad_path = original / "ungranted-stream.json"
    save(bad_path, {**command, "stdout": {"root": "ungranted", "path": "private.bin", "sha256": sha(ungranted / "private.bin")}})
    c.executions = [{"record": {"file": reference(bad_path, "originalRoot"), "pointer": ""}, "expectedExitCode": 0}]
    reject("composition_ungranted_typed_stream_root_rejected", c.records)
    require(all(p.read_bytes() == raw for p, raw in originals.items()), "Original composition bytes changed")
    checks.append("composition_original_proof_commands_raw_bytes_unchanged")
    return checks


def self_test(root, sdk):
    """Infrastructure fixtures only; never assert that these are SDK executions."""
    root = root / "joined-fixtures"; root.mkdir(exist_ok=False)
    value = root / "value.json"
    value.write_text('{"a/b":{"~key":["世界",7]}}', encoding="utf-8")
    context = Context(root, sdk)
    ref = {"path": "value.json", "sha256": sha(value)}
    require(context.select({"file": ref, "pointer": "/a~1b/~0key/0"}) == "世界", "Positive RFC6901 selector failed")
    checks = ["joined_exact_raw_bytes_and_rfc6901_selector"]
    def reject(label, action, missing=False):
        try: action()
        except MissingProof:
            require(missing, "Invalid proof misclassified as missing: " + label)
            checks.append(label)
        except (InvalidProof, ValueError, KeyError, OSError):
            require(not missing, "Missing proof misclassified as invalid: " + label)
            checks.append(label)
        else: raise InvalidProof("Joined negative falsely accepted: " + label)
    reject("joined_bad_hash_rejected", lambda: context.file({**ref, "sha256": "0" * 64}))
    reject("joined_path_escape_rejected", lambda: context.file({"path": "../private.json", "sha256": "0" * 64}))
    reject("joined_json_cannot_authorize_external_root", lambda: context.file({**ref, "root": "ungranted"}))
    reject("joined_bad_pointer_escape_rejected", lambda: context.select({"file": ref, "pointer": "/a~2b"}))
    reject("joined_absent_member_remains_not_run", lambda: context.select({"file": ref, "pointer": "/absent"}), True)
    duplicate = root / "duplicate.json"; duplicate.write_text('{"x":1,"x":2}', encoding="utf-8")
    reject("joined_duplicate_json_key_rejected", lambda: read_json(duplicate))
    snapshot = source_snapshot(sdk)
    names = {row["path"] for row in snapshot}
    require({"examples/capabilities.json", "examples/payloads.json", "templates/rust/src/lib.rs"} <= names,
            "Actual flat/example/template compile inputs omitted")
    require("schemas/adoption-evidence.schema.json" not in names and
            all(not (set(PurePosixPath(name).parts) & SKIP) for name in names), "Caches/non-wire governance in compiled closure")
    checks.append("joined_flat_fixture_template_inputs_included_caches_governance_excluded")
    proof = {"schemaVersion": 1, "kind": "JOINED_CHAIN_PROOF_V1", "chainId": "profile-parity", "mode": "MOCK",
             "provenance": "CAPTURED_EXECUTION", "sourceClosure": {"current": snapshot, "captured": snapshot, "review": None},
             "executions": [], "artifacts": {"profileResponse": {"file": ref, "pointer": ""}}}
    def save(data):
        p = root / "proof.json"; p.write_text(json.dumps(data), encoding="utf-8")
        return {"path": "proof.json", "sha256": sha(p)}
    chain = {"id": "profile-parity", "requiredArtifacts": ["profileResponse"]}
    reject("joined_no_actual_execution_rejected", lambda: verify(root, save(proof), chain, sdk))
    reject("joined_pass_label_never_proof", lambda: verify(root, save({**proof, "status": "PASS"}), chain, sdk))
    reject("joined_missing_typed_artifact_not_run", lambda: verify(root, save({**proof, "artifacts": {}}), chain, sdk), True)
    escaped_build = root / "escaped-build.json"
    escaped_build.write_text(json.dumps({"builds": [{"sourceRoot": str(Path(root.anchor) / "ungranted-sdk-release-source")}]}), encoding="utf-8")
    index = root / "candidate-index.json"
    index.write_text(json.dumps({"candidateA": "sdk", "candidateB": "evidence", "evidenceRoot": "evidence",
        "evidence": {"path": escaped_build.name, "sha256": sha(escaped_build)}}), encoding="utf-8")
    context.artifacts = {"candidateInventory": {"file": {"path": index.name, "sha256": sha(index)}, "pointer": ""}}
    reject("joined_candidate_json_cannot_expand_absolute_scope", lambda: check_candidate(context))
    checks.extend(composition_self_test(root, sdk))
    return checks
