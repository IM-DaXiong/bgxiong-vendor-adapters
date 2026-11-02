#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Check a BGXiong adapter directory against the public manifest contract.

Contract release 1.2.0, manifest schemaVersion 2. This does not compile WASM.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

LANGUAGES = {"profile", "python", "typescript", "rust", "tinygo"}
ENDPOINT_POLICIES = {"staticHosts", "userConfigurable", "allowPrivateNetwork"}
REQUIRED = (
    "schemaVersion",
    "pluginId",
    "displayName",
    "version",
    "language",
    "entry",
    "capabilitySlots",
    "requestedCapabilities",
)
PLACEHOLDER_KEYS = {
    "",
    "REPLACE_WITH_DEDICATED_LOW_BALANCE_KEY",
    "YOUR_API_KEY",
}
KEY_ASSIGN = re.compile(r'pub const API_KEY:\s*&str\s*=\s*"([^"]*)"')


def fail(message: str) -> int:
    print("FAIL", message)
    return 1


def find_manifest(pack: Path) -> tuple[Path, Path] | None:
    direct = pack / "manifest.json"
    if direct.is_file():
        return pack, direct
    found = sorted(p for p in pack.glob("**/manifest.json") if "target" not in p.parts)
    if not found:
        return None
    return found[0].parent, found[0]


def check_manifest(data: object) -> list[str]:
    errors: list[str] = []
    if not isinstance(data, dict):
        return ["manifest is not a JSON object"]
    for key in REQUIRED:
        if key not in data:
            errors.append(f"missing {key}")
    if data.get("schemaVersion") != 2:
        errors.append("schemaVersion must be 2")
    plugin_id = data.get("pluginId")
    if not isinstance(plugin_id, str) or not plugin_id.strip() or len(plugin_id) > 128:
        errors.append("pluginId must be a non-empty string up to 128 chars")
    if data.get("language") not in LANGUAGES:
        errors.append(f"language must be one of {sorted(LANGUAGES)}")
    entry = data.get("entry")
    if not isinstance(entry, str) or not entry.strip():
        errors.append("entry must be a non-empty string")
    slots = data.get("capabilitySlots")
    if not isinstance(slots, list) or not slots:
        errors.append("capabilitySlots must be a non-empty array")
    elif not all(isinstance(slot, dict) and isinstance(slot.get("slot"), str) and slot.get("slot") for slot in slots):
        errors.append("each capabilitySlots item needs a non-empty slot")
    caps = data.get("requestedCapabilities")
    if not isinstance(caps, dict):
        errors.append("requestedCapabilities must be an object")
    else:
        policies = caps.get("endpointPolicy")
        if not isinstance(policies, list) or not policies or any(item not in ENDPOINT_POLICIES for item in policies):
            errors.append("requestedCapabilities.endpointPolicy is invalid")
        if not isinstance(caps.get("auth"), list):
            errors.append("requestedCapabilities.auth must be an array")
    return errors


def scan_secrets(pack: Path) -> list[str]:
    errors: list[str] = []
    for path in pack.rglob("*"):
        if not path.is_file() or "target" in path.parts:
            continue
        if path.suffix not in {".rs", ".json", ".md", ".toml", ".py"}:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeError:
            continue
        if re.search(r"sk-[A-Za-z0-9]{16,}", text):
            errors.append(f"possible live key in {path.name}")
        if "BEGIN PRIVATE KEY" in text or "BEGIN RSA PRIVATE KEY" in text:
            errors.append(f"private key block in {path.name}")
        if path.name != "config.rs":
            continue
        for match in KEY_ASSIGN.finditer(text):
            value = match.group(1)
            if value in PLACEHOLDER_KEYS or "REPLACE_WITH" in value:
                continue
            errors.append(f"non-placeholder API_KEY in {path.relative_to(pack)}")
    return errors


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print("Usage: validate-example.py <pack-dir>")
        return 2
    pack = Path(argv[1])
    if not pack.is_dir():
        return fail(f"not a directory: {pack}")
    found = find_manifest(pack)
    if found is None:
        return fail("missing manifest.json")
    pack, manifest_path = found
    try:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return fail(f"manifest json: {exc}")
    errors = check_manifest(data)
    errors.extend(scan_secrets(pack))
    if errors:
        for item in errors:
            print("FAIL", item)
        return 1
    if not (pack / "README.md").is_file():
        print("WARN missing README.md")
    if data.get("language") == "rust":
        for hint in ("Cargo.toml", "wit", "vendor-sdk"):
            if not (pack / hint).exists():
                print("WARN missing", hint)
    print("OK", pack)
    print("pluginId=", data.get("pluginId"))
    print("schemaVersion=", data.get("schemaVersion"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
