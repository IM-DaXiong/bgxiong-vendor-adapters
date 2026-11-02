"""Capture read-only repository isolation evidence. Never open an application DB.

Only Git-indexed and nonignored source files are inspected. Test resources are
created under a fresh OS temporary directory, never under an application root.
"""
from pathlib import Path
import argparse
import hashlib
import json
import subprocess
import tempfile

SDK = Path(__file__).resolve().parents[1]
ROOT = SDK.parents[2]


def sha(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
                                     separators=(",", ":")).encode()).hexdigest()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def inventory(root):
    return sorted(set(x.decode("utf-8") for x in
                      git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0") if x))


def protected(paths, policy):
    return [p for p in paths if p != policy["managementFile"] and
            not any(p.startswith(prefix) for prefix in policy["allowedRoots"])]


def snapshot(root, paths):
    return {p: sha(root / p) if (root / p).is_file() else None for p in paths}


def verify(root, baseline, paths=None):
    errors = []
    if baseline.get("schemaVersion") != 1 or not baseline.get("protectedFiles"):
        return ["missing or incomplete protected baseline"]
    expected = baseline["protectedFiles"]
    if digest(expected) != baseline.get("protectedDigest"):
        errors.append("baseline protected digest mismatch")
    policy = baseline.get("policy", {})
    if not policy.get("allowedRoots") or not baseline.get("publicDesignFiles"):
        return errors + ["missing policy or public design baseline"]
    actual = snapshot(root, protected(inventory(root) if paths is None else paths, policy))
    for p in sorted(set(expected) | set(actual)):
        if expected.get(p) != actual.get(p) or (p in expected) != (p in actual):
            errors.append("protected source changed: " + p)
    return errors


def self_test():
    with tempfile.TemporaryDirectory(prefix="bgx-sdk-isolation-") as name:
        root = Path(name)
        (root / "protected.txt").write_text("original", encoding="utf-8")
        paths = ["protected.txt"]
        files = snapshot(root, paths)
        baseline = {"schemaVersion": 1, "protectedFiles": files,
                    "protectedDigest": digest(files),
                    "publicDesignFiles": {"design.json": "test-only"},
                    "policy": {"managementFile": "ledger.json", "allowedRoots": ["sdk/"]}}
        assert verify(root, baseline, paths) == []
        (root / "protected.txt").write_text("tampered", encoding="utf-8")
        assert verify(root, baseline, paths)
        (root / "protected.txt").write_text("original", encoding="utf-8")
        assert verify(root, baseline, paths) == []
        assert verify(root, {**baseline, "protectedFiles": {}}, paths)
        assert verify(root, {**baseline, "publicDesignFiles": {}}, paths)
        assert verify(root, baseline, [])
        (root / "added.txt").write_text("unauthorized", encoding="utf-8")
        assert verify(root, baseline, paths + ["added.txt"])
    return ["normal-copy", "changed-protected-copy-rejected", "missing-baseline-rejected",
            "missing-design-rejected", "deleted-file-rejected", "added-file-rejected"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.out.exists():
        parser.error("baseline already exists; refusing to replace audit history")
    policy = json.loads((SDK / "isolation-policy.json").read_text(encoding="utf-8"))
    ledger = json.loads((ROOT / policy["managementFile"]).read_text(encoding="utf-8"))
    design = ledger["publicDesignFiles"]
    for rel, expected in design.items():
        if not (ROOT / rel).is_file() or sha(ROOT / rel) != expected:
            parser.error("missing or changed design asset: " + rel)
        if args.source_root and sha(args.source_root / rel) != expected:
            parser.error("source/worktree design differs: " + rel)
    files = snapshot(ROOT, protected(inventory(ROOT), policy))
    data = {"schemaVersion": 1, "baseCommit": git(ROOT, "rev-parse", "HEAD").decode().strip(),
            "workspaceRoot": str(ROOT), "sourceRoot": str(args.source_root) if args.source_root else None,
            "policy": policy, "publicDesignFiles": design, "publicDesignDigest": digest(design),
            "protectedFiles": files, "protectedDigest": digest(files),
            "dirtyFiles": git(ROOT, "status", "--porcelain=v1", "-uall").decode("utf-8"),
            "selfTests": self_test() if args.self_test else []}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"protectedCount": len(files), "publicDesignDigest": data["publicDesignDigest"],
                      "selfTests": data["selfTests"], "isolationErrors": verify(ROOT, data)}))
    if verify(ROOT, data):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
