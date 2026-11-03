"""Export the package tracking input schema from the public video-target-keyframes-v1 profile.

The package never hand-writes this schema. The public tracking input and the selection document it
references are bundled into one resource (selection becomes `$defs/selection`, its `$defs` are
lifted), written with LF, and its byte digest is copied into `x-targetTracking.roles.trackingPreview`.
`--check` fails when the committed files differ from a fresh export.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

PROFILE = "profiles/video-target-keyframes-v1"
SELECTION_FILE = "selection.schema.json"
TRACKING_FILE = "tracking.input.schema.json"
PROGRESS_FILE = "progress.schema.json"
RECEIPT_FILE = "tracking-receipt.schema.json"
OUTPUT_FILE = "tracking.output.schema.json"
RESOURCE_HINT = {
    "blocking": False,
    "basis": "mask-only tracking; no diffusion",
    "vramPeakBytes": None,
    "message": "跟踪预览只跑遮罩，不触发扩散生成。",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def rewrite_refs(node, inside_selection):
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key == "$ref":
                out[key] = map_ref(value, inside_selection)
            else:
                out[key] = rewrite_refs(value, inside_selection)
        return out
    if isinstance(node, list):
        return [rewrite_refs(item, inside_selection) for item in node]
    return node


def map_ref(ref, inside_selection):
    if inside_selection and ref.startswith("#/$defs/"):
        return ref
    if ref == SELECTION_FILE or ref == SELECTION_FILE + "#":
        return "#/$defs/selection"
    prefix = SELECTION_FILE + "#/$defs/"
    if ref.startswith(prefix):
        return "#/$defs/" + ref[len(prefix):]
    raise SystemExit(f"unsupported $ref in public profile: {ref}")


def encode(document):
    return (json.dumps(document, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def selection_defs(profile):
    raw = (profile / SELECTION_FILE).read_bytes()
    selection = json.loads(raw)
    defs = {name: rewrite_refs(body, True) for name, body in selection.get("$defs", {}).items()}
    for reserved in ("selection", "progress", "receipt"):
        if reserved in defs:
            raise SystemExit(f"public selection $defs already names '{reserved}'")
    defs["selection"] = rewrite_refs(
        {k: v for k, v in selection.items() if k not in ("$schema", "$id", "$defs")}, True)
    return defs, sha(raw)


def body_of(document):
    return {k: rewrite_refs(v, False) for k, v in document.items() if k not in ("$schema", "$id")}


def export(sdk_root):
    profile = Path(sdk_root) / PROFILE
    defs, selection_sha = selection_defs(profile)
    tracking_bytes = (profile / TRACKING_FILE).read_bytes()
    tracking = json.loads(tracking_bytes)
    bundled = {"$schema": tracking["$schema"]}
    bundled["$comment"] = (
        f"Generated from {PROFILE}/{TRACKING_FILE} sha256:{sha(tracking_bytes)} and "
        f"{SELECTION_FILE} sha256:{selection_sha}. Do not edit; run export-profile-schemas.py.")
    bundled.update(body_of(tracking))
    bundled["$defs"] = defs
    bundled["x-resourceHint"] = RESOURCE_HINT
    return encode(bundled)


def export_output(sdk_root):
    """Query data is either live progress (running) or the sealed receipt (succeeded)."""
    profile = Path(sdk_root) / PROFILE
    defs, selection_sha = selection_defs(profile)
    progress_bytes = (profile / PROGRESS_FILE).read_bytes()
    receipt_bytes = (profile / RECEIPT_FILE).read_bytes()
    defs["progress"] = body_of(json.loads(progress_bytes))
    defs["receipt"] = body_of(json.loads(receipt_bytes))
    return encode({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$comment": (
            f"Generated from {PROFILE}/{PROGRESS_FILE} sha256:{sha(progress_bytes)}, "
            f"{RECEIPT_FILE} sha256:{sha(receipt_bytes)} and {SELECTION_FILE} sha256:{selection_sha}. "
            "Do not edit; run export-profile-schemas.py."),
        "oneOf": [{"$ref": "#/$defs/progress"}, {"$ref": "#/$defs/receipt"}],
        "$defs": defs,
    })


DIGEST_PATTERN = re.compile(
    r'("trackingPreview"\s*:\s*\{[^{}]*?"inputSchemaDigest"\s*:\s*")([0-9a-f]{64})(")', re.S)


def with_tracking_digest(input_schema_text, digest):
    matches = DIGEST_PATTERN.findall(input_schema_text)
    if len(matches) != 1:
        raise SystemExit("input.schema.json must declare exactly one trackingPreview.inputSchemaDigest")
    return DIGEST_PATTERN.sub(lambda m: m.group(1) + digest + m.group(3), input_schema_text)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    tracking_path = root / "schemas" / TRACKING_FILE
    input_path = root / "schemas" / "input.schema.json"
    output_path = root / "schemas" / OUTPUT_FILE
    tracking = export(args.sdk.resolve())
    output = export_output(args.sdk.resolve())
    digest = sha(tracking)
    current_input = input_path.read_bytes().decode("utf-8")
    next_input = with_tracking_digest(current_input, digest)
    stale = []
    if tracking_path.read_bytes() != tracking:
        stale.append(str(tracking_path))
    if output_path.read_bytes() != output:
        stale.append(str(output_path))
    if next_input != current_input:
        stale.append(str(input_path))
    if args.check:
        if stale:
            print(json.dumps({"stale": stale}, ensure_ascii=False))
            sys.exit(1)
    else:
        tracking_path.write_bytes(tracking)
        output_path.write_bytes(output)
        input_path.write_bytes(next_input.encode("utf-8"))
    print(json.dumps({"trackingInputSchemaDigest": digest, "updated": [] if args.check else stale}))


if __name__ == "__main__":
    main()
