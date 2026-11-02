#!/usr/bin/env python3
"""Read-only per-module adoption evidence check; never imports into an app.

Requires the pinned packages in requirements-design.txt. --self-test validates
only governance fixtures. A module remains NOT_ADOPTED until its separate plan
provides actual G07 execution receipts. SDK readiness alone is insufficient.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import sys
import tempfile

sys.dont_write_bytecode = True
from jsonschema import Draft202012Validator

SDK = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("adoption_export", Path(__file__).with_name("export_sdk.py"))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)
require = exporter.require
CATEGORIES = {"six-operations", "host-ports", "catalog-parity", "ipc-contract", "restart", "cancellation", "isolation", "module-acceptance"}

def validator():
    schema = exporter.read_json(SDK / "schemas/adoption-evidence.schema.json")
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)

def verify(value, evidence_root=None, release_status=None, release_lock=None):
    validator().validate(value)
    if value["moduleStatus"] == "NOT_ADOPTED":
        return {"moduleId":value["moduleId"], "moduleStatus":"NOT_ADOPTED", "G07":"NOT_RUN", "productionChanges":False}
    require(evidence_root is not None and release_status is not None and release_lock is not None, "EXPLICIT_MODULE_EVIDENCE_AND_RELEASE_REQUIRED")
    root = exporter.absolute(evidence_root)
    status = exporter.read_json(exporter.absolute(release_status, True))
    lock_path = exporter.absolute(release_lock, True)
    lock = exporter.read_json(lock_path)
    require(status.get("status") == "SDK_READY" and status.get("sdkReady") is True and lock.get("sealed") is True, "SDK_NOT_READY")
    require(status.get("sdkRelease") == value["sdkRelease"] == lock.get("sdkRelease") and exporter.digest(lock_path) == value["sdkReleaseLockSha256"] == status.get("releaseLockDigest"), "RELEASE_IDENTITY_MISMATCH")
    operations = exporter.read_json(SDK / "operation-contracts.json")["operations"]
    ports = exporter.read_json(SDK / "schemas/host-ports.schema.json")["x-wit-functions"]
    require(set(value["operations"]) == set(operations), "INCOMPLETE_COMMON_OPERATIONS")
    require(set(value["hostPorts"]) == {p["interface"] + "." + p["name"] for p in ports}, "INCOMPLETE_COMMON_HOST_PORTS")
    categories = [e["category"] for e in value["evidence"]]
    require(set(categories) == CATEGORIES and len(categories) == len(CATEGORIES), "INCOMPLETE_OR_DUPLICATE_MODULE_EVIDENCE")
    for item in value["evidence"]:
        path = exporter.no_links(root / exporter.relative(item["path"]), True)
        require(exporter.digest(path) == item["sha256"], "MODULE_EVIDENCE_DRIFT")
        receipt = exporter.read_json(path)
        require(receipt.get("kind") == "ACTUAL_MODULE_EXECUTION" and receipt.get("moduleId") == value["moduleId"] and receipt.get("category") == item["category"], "MODULE_EXECUTION_SCOPE_MISMATCH")
        require(receipt.get("sdkReleaseLockSha256") == value["sdkReleaseLockSha256"], "MODULE_USED_DIFFERENT_SDK")
        require(isinstance(receipt.get("command"), list) and receipt["command"] and receipt.get("exitCode") == 0 and receipt.get("timedOut") is False and receipt.get("durationSeconds", 0) > 0, "MODULE_NOT_ACTUALLY_EXECUTED")
        for name in ["stdout", "stderr"]:
            ref = receipt[name]
            artifact = exporter.no_links(root / exporter.relative(ref["path"]), True)
            require(exporter.digest(artifact) == ref["sha256"], "MODULE_LOG_DRIFT")
        require(isinstance(receipt.get("observations"), list) and receipt["observations"], "STRUCTURED_MODULE_OBSERVATIONS_REQUIRED")
    return {"moduleId":value["moduleId"], "moduleStatus":value["moduleStatus"], "G07":value["G07"], "evidenceScope":"EXPLICIT_UNKEYED_MEASUREMENTS_NOT_PRODUCTION_EXECUTION", "productionChanges":False}

def self_test():
    base = {"schemaVersion":1,"moduleId":"fixture-unadopted","sdkRelease":"2.0.0","sdkReleaseLockSha256":None,
        "moduleStatus":"NOT_ADOPTED","G07":"NOT_RUN","operations":[],"hostPorts":[],
        "parity":{"sharedCatalog":False,"mcpPublished":False,"f1SharedKernel":False},"evidence":[]}
    require(verify(base)["moduleStatus"] == "NOT_ADOPTED", "NOT_ADOPTED_FIXTURE")
    checks = 1
    for changed in [dict(base, moduleStatus="ADOPTED"), dict(base, G07="PASS"), dict(base, unknown=True), dict(base, operations=["submit"]), dict(base, parity={"sharedCatalog":True,"mcpPublished":False,"f1SharedKernel":False})]:
        require(not validator().is_valid(changed), "INVALID_ADOPTION_ACCEPTED"); checks += 1
    advanced = dict(base, moduleStatus="ADOPTION_CANDIDATE", sdkReleaseLockSha256="0"*64,
        operations=list(exporter.read_json(SDK/"operation-contracts.json")["operations"]),
        hostPorts=[p["interface"] + "." + p["name"] for p in exporter.read_json(SDK/"schemas/host-ports.schema.json")["x-wit-functions"]],
        parity={"sharedCatalog":True,"mcpPublished":True,"f1SharedKernel":True},
        evidence=[{"category":c,"path":c+".json","sha256":"0"*64} for c in sorted(CATEGORIES)])
    validator().validate(advanced)
    try: verify(advanced)
    except exporter.ExportError: checks += 1
    else: raise AssertionError("candidate accepted without actual evidence")
    print(json.dumps({"scope":"GOVERNANCE_FIXTURES_ONLY","checks":checks,"G07":"NOT_RUN","moduleStatus":"NOT_ADOPTED","sdkReady":False}))

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--self-test",action="store_true"); p.add_argument("--manifest",type=Path)
    p.add_argument("--evidence-root",type=Path);p.add_argument("--release-status",type=Path);p.add_argument("--release-lock",type=Path)
    args=p.parse_args()
    if args.self_test: self_test(); return
    require(args.manifest is not None,"EXPLICIT_MODULE_MANIFEST_REQUIRED")
    result=verify(exporter.read_json(exporter.absolute(args.manifest,True)),args.evidence_root,args.release_status,args.release_lock)
    print(json.dumps(result,ensure_ascii=False))

if __name__ == "__main__":
    try: main()
    except Exception as e:
        print(json.dumps({"status":"NOT_VERIFIED","error":str(e),"productionChanges":False}),file=sys.stderr);raise SystemExit(1)
