#!/usr/bin/env python3
"""Create a new author reference index; never rewrites or executes original evidence.

See EVIDENCE-REFERENCES.zh-CN.md. This is reference preparation, not C36 or READY.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import sys

sys.dont_write_bytecode = True
SDK = Path(__file__).absolute().parents[1]
_spec = importlib.util.spec_from_file_location("author_reference_contract", SDK / "tools/conformance_proofs.py")
joined = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(joined)


def compose(context, author_selector):
    return context.author_index(author_selector)


def build_parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--evidence-root", type=Path, required=True)
    p.add_argument("--author-proof-root", required=True)
    p.add_argument("--author-proof-path", required=True)
    p.add_argument("--author-proof-sha256", required=True)
    p.add_argument("--artifact-root", action="append", default=[])
    p.add_argument("--out", type=Path, required=True)
    return p


def write_index(context, out, index):
    joined.require(out.is_absolute() and ".." not in out.parts and out.is_relative_to(context.root),
                   "AUTHOR_REFERENCE_OUTPUT_OUTSIDE_EVIDENCE")
    author_root = context.roots[index["independentAuthorEvidence"]["file"]["root"]]
    joined.require(not out.is_relative_to(author_root)
                   and not any(out.is_relative_to(root) for name, root in context.roots.items() if name != "evidence"),
                   "AUTHOR_REFERENCE_OUTPUT_IN_INPUT_ROOT")
    joined.no_links(out.parent)
    joined.require(not os.path.lexists(out), "AUTHOR_REFERENCE_INDEX_EXISTS")
    with out.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(index, stream, ensure_ascii=False, indent=2)
        stream.write("\n"); stream.flush(); os.fsync(stream.fileno())


def main():
    a = build_parser().parse_args()
    try:
        joined.require(a.evidence_root.is_absolute() and ".." not in a.evidence_root.parts,
                       "EXPLICIT_AUTHOR_REFERENCE_EVIDENCE_ROOT_REQUIRED")
        roots = {}
        for raw in a.artifact_root:
            name, sep, folder = raw.partition("=")
            joined.require(sep and name not in roots and name not in {"evidence", "sdk"}, "INVALID_AUTHOR_REFERENCE_ROOT_GRANT")
            path = Path(folder)
            joined.require(path.is_absolute() and ".." not in path.parts, "ABSOLUTE_AUTHOR_REFERENCE_ROOT_REQUIRED")
            roots[name] = path
        context = joined.Context(a.evidence_root, SDK, roots)
        selected = {"file": {"root": a.author_proof_root, "path": a.author_proof_path,
                              "sha256": a.author_proof_sha256}, "pointer": ""}
        index = compose(context, selected)
        write_index(context, a.out, index)
        print(json.dumps({"status": "REFERENCE_INDEX_PREPARED", "path": str(a.out), "sha256": joined.sha(a.out),
                          "authorCommandsReexecuted": False, "sdkReady": False}))
        return 0
    except (ValueError, OSError) as error:
        print(json.dumps({"status": "FAILED", "error": str(error), "sdkReady": False}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
