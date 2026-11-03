"""Offline, read-only Comfy tracking evidence mapping. Never contacts /prompt or mutates a task.
The result.outputs array is deliberately empty: the maintenance SDK must import every
listed file with its specified media ID and populate real broker output handles first.
"""
from __future__ import annotations
import argparse, hashlib, json, shutil, uuid
from pathlib import Path
from PIL import Image


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def export(preserved: Path, out: Path):
    if out.exists():
        raise ValueError("Output already exists; preserve prior evidence and choose a new directory")
    preserved = preserved.resolve()
    for proof in load(preserved / "preserved-inventory.json"):
        path = Path(proof["path"]).resolve()
        if not path.is_relative_to(preserved) or digest(path) != proof["sha256"]:
            raise ValueError("Preserved original evidence changed")
    checkpoint = load(preserved / "checkpoint.original.json")
    frozen = load(preserved / "frozen.original.json")
    request = frozen["request"]
    task = checkpoint["requestId"]
    uuid.UUID(task)
    if checkpoint["state"] != "acceptance_unknown":
        raise ValueError("Only the original unknown accepted request may be mapped")
    invocation = load(preserved / "sdk-submit/execution.invocation.json")
    if invocation["operation"] != "submit" or invocation["requestId"] != task:
        raise ValueError("Original invocation identity differs")
    payload = json.loads(invocation["payloadJson"])
    if payload["input"] != request["input"] or payload["configuration"] != request["configuration"]:
        raise ValueError("Original submit differs from frozen input/configuration")
    posts = [json.loads(line) for line in (preserved / "sdk-submit/before-send.jsonl").read_text(encoding="utf-8").splitlines()]
    posts = [p for p in posts if p["method"] == "POST" and p["url"].endswith("/prompt")]
    if len(posts) != 1 or posts[0]["requestId"] != task or posts[0]["body"]["client_id"] != task:
        raise ValueError("Need exactly one authentic original prompt POST")
    if posts[0]["url"] != request["configuration"]["endpointBaseUrl"].rstrip("/") + "/prompt":
        raise ValueError("Original POST endpoint differs from frozen endpoint")
    history = load(preserved / "history.http.json")
    if len(history) != 1:
        raise ValueError("Need one complete original history response")
    prompt_id, history = next(iter(history.items()))
    if history["prompt"][1] != prompt_id or history["prompt"][3]["client_id"] != task:
        raise ValueError("History prompt/client identity differs")
    graph = posts[0]["body"]["prompt"]
    if graph != history["prompt"][2]:
        raise ValueError("Remote executed graph differs from original POST")
    status = history["status"]
    if status["status_str"] != "success" or status["completed"] is not True:
        raise ValueError("Original remote prompt has not completed successfully")
    if not any(m[0] == "execution_success" and m[1].get("prompt_id") == prompt_id for m in status["messages"]):
        raise ValueError("Missing original execution success evidence")
    manifest = load(preserved / "manifest.http.json")
    if manifest != load(preserved / "remote-artifact/manifest.json"):
        raise ValueError("HTTP and on-disk sealed manifests differ")
    inp = request["input"]
    expected = {"requestId": task, "selectionDigest": inp["selection"]["selectionDigest"], "sourceDigest": inp["sourceIdentity"]["contentDigest"], "range": inp["range"], "direction": inp["direction"]}
    for k, value in expected.items():
        if manifest[k] != value:
            raise ValueError("Manifest binding differs: " + k)
    exp = graph["track.export"]["inputs"]
    for key, expected_value in {"request_id": task, "selection_digest": manifest["selectionDigest"], "source_digest": manifest["sourceDigest"], "direction": manifest["direction"], "input_schema_digest": request["inputSchemaDigest"], "operation_id": request["operationId"], "range_start_ms": inp["range"]["startMs"], "range_end_ms": inp["range"]["endMs"]}.items():
        if exp[key] != expected_value:
            raise ValueError("Submitted export binding differs: " + key)
    if manifest["operationRef"]["inputSchemaDigest"] != request["inputSchemaDigest"] or manifest["operationRef"]["operationId"] != request["operationId"]:
        raise ValueError("Manifest operation differs from original frozen request")
    if manifest["operationRef"]["operationVersion"] != exp["operation_version"]:
        raise ValueError("Manifest operation version differs from submitted graph")
    if manifest["resolvedKeyframes"] != json.loads(exp["resolved_keyframes_json"]):
        raise ValueError("Resolved keyframes differ from original submitted graph")
    if manifest["ready"] is not True or manifest["uncomputedRanges"]:
        raise ValueError("Original artifact is not ready")
    source_sha = digest(preserved / "source.submitted.mp4")
    if manifest["sourceDigest"] != "sha256:" + source_sha:
        raise ValueError("Actual submitted source bytes differ")
    identities = checkpoint["mediaIdentities"]
    if not any(i["id"] == "source" and i["digest"].removeprefix("sha256:") == source_sha for i in identities):
        raise ValueError("Source bytes differ from original checkpoint")
    raw_frames = manifest["frames"]
    if len(raw_frames) != manifest["frameCount"] or not raw_frames:
        raise ValueError("Incomplete manifest frame list")
    times = json.loads(graph["sam.multi"]["inputs"]["frame_times_json"])
    if [f["timeMs"] for f in raw_frames] != times:
        raise ValueError("Frame PTS differ from submitted sampled sequence")
    # Validate every actual PNG, including non-covered frames: original artifact digest
    # seals indexed raw L pixels, not PNG encoding bytes.
    pixel_hasher = hashlib.sha256()
    files = []
    for i, f in enumerate(raw_frames):
        if f["frameIndex"] != i or f["state"] not in ("covered", "lost", "uncomputed"):
            raise ValueError("Invalid original frame index/state")
        rel = Path(f["relPath"])
        if rel.parts != ("masks", f"frame_{i:05d}.png"):
            raise ValueError("Unexpected original mask path")
        path = preserved / "remote-artifact" / rel
        with Image.open(path) as image:
            if image.size != (manifest["outputGeometry"]["width"], manifest["outputGeometry"]["height"]):
                raise ValueError("Mask geometry differs")
            pixels = image.convert("L").tobytes()
        pixel_hasher.update((str(i) + ":").encode("ascii")); pixel_hasher.update(pixels)
        files.append(path)
    if manifest["digest"] != "sha256:" + pixel_hasher.hexdigest():
        raise ValueError("Actual mask pixels differ from sealed artifact digest")
    out.mkdir(parents=True)
    media_dir = out / "media"; media_dir.mkdir()
    media = []; frames = []
    def item(media_id, role, kind, mime, path):
        return {"id": media_id, "role": role, "kind": kind, "mime": mime, "path": str(path.resolve()), "sha256": digest(path), "byteLength": str(path.stat().st_size), "fileName": path.name}
    for f, path in zip(raw_frames, files):
        ref = None
        if f["state"] == "covered":
            target = media_dir / path.name; shutil.copyfile(path, target)
            media_id = task + ":mask:" + str(f["frameIndex"])
            record = item(media_id, "maskFrame", "image", "image/png", target); media.append(record)
            ref = {"artifactId": media_id, "digest": "sha256:" + record["sha256"]}
        frames.append({"frameIndex": f["frameIndex"], "timeMs": f["timeMs"], "state": f["state"], "maskPageRef": ref})
    span = inp["range"]["endMs"] - inp["range"]["startMs"]
    pages = []; chunks = [frames[i:i + 60] for i in range(0, len(frames), 60)]
    def json_file(media_id, role, name, value):
        path = media_dir / name
        path.write_text(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")), encoding="utf-8")
        media.append(item(media_id, role, "file", "application/octet-stream", path))
    for i, chunk in enumerate(chunks):
        cursor = "p" + str(i); next_cursor = "p" + str(i + 1) if i + 1 < len(chunks) else None
        media_id = task + ":page:" + str(i)
        value = {"requestId": task, "selectionDigest": manifest["selectionDigest"], "page": {"cursor": cursor, "nextCursor": next_cursor, "complete": next_cursor is None}, "frames": chunk}
        json_file(media_id, "framePage", cursor + ".json", value)
        pages.append({"cursor": cursor, "nextCursor": next_cursor, "startMs": chunk[0]["timeMs"], "endMs": chunks[i + 1][0]["timeMs"] if next_cursor else span, "mediaId": media_id, "available": True})
    index_id = task + ":index"
    json_file(index_id, "frameIndex", "index.json", {"requestId": task, "selectionDigest": manifest["selectionDigest"], "complete": True, "pages": pages})
    keys = ["requestId", "sourceDigest", "range", "direction", "selectionDigest", "operationRef", "outputGeometry", "outputFps", "resolvedKeyframes", "coveredRanges", "lostRanges", "uncomputedRanges", "skipRanges", "ready", "readyReason"]
    data = {k: manifest.get(k) for k in keys}
    data.update({"previewMediaRef": None, "maskArtifact": {"artifactId": task, "digest": manifest["digest"], "pageIndexRef": index_id}, "framePageIndexRef": index_id})
    result = {"kind": "completed", "vendorTaskId": prompt_id, "appliedParams": {"direction": inp["direction"]}, "outputs": [], "data": data, "usage": None}
    completion = out / (task + ".completion.json")
    completion.write_text(json.dumps({"result": result, "media": media}, ensure_ascii=False, indent=2), encoding="utf-8")
    evidence = [{"reference": str(p.resolve()), "sha256": digest(p)} for p in sorted(preserved.rglob("*")) if p.is_file()]
    evidence += [{"reference": str(completion.resolve()), "sha256": digest(completion)}]
    evidence_path = out / "evidence-files.json"
    evidence_path.write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(json.dumps({"completion": str(completion.resolve()), "evidence": str(evidence_path.resolve()), "mediaCount": len(media), "maskFrames": len(files), "vendorTaskId": prompt_id, "validation": "Offline binding/pixel digest verified; SDK must import all media then validate completed result against original frozen contract."}))
    return completion


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preserved", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    export(args.preserved, args.out)
