#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Basic pack validation for BGXiong vendor adapters."""
from __future__ import annotations
import json
import sys
from pathlib import Path

REQUIRED_ANY = [
    ("manifest.json",),
]
OPTIONAL_HINTS = ["README.md", "Cargo.toml", "wit"]


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: validate-example.py <pack-dir>")
        return 2
    pack = Path(argv[1])
    if not pack.is_dir():
        print("FAIL not a directory:", pack)
        return 1
    man = pack / "manifest.json"
    if not man.is_file():
        # allow nested
        cands = list(pack.glob("**/manifest.json"))
        if not cands:
            print("FAIL missing manifest.json")
            return 1
        man = cands[0]
        pack = man.parent
    try:
        data = json.loads(man.read_text(encoding="utf-8"))
    except Exception as e:
        print("FAIL manifest json:", e)
        return 1
    if not isinstance(data, dict):
        print("FAIL manifest not object")
        return 1
    # soft required keys commonly used
    for k in ("id",):
        if k not in data:
            print("WARN missing key:", k)
    print("OK", pack)
    print("manifest_id=", data.get("id"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
