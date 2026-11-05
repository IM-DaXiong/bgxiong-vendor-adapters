"""Package-owned public-profile tracking receipt, derived from actual tracker frames."""
import json
import math


def tracking_receipt(transform, fps, analysis, selection):
    if not isinstance(fps, (int, float)) or not math.isfinite(fps) or fps <= 0:
        raise ValueError("Tracking frame rate must be positive")
    if (selection["analysisId"], selection["analysisRevision"], selection["analysisDigest"]) != (analysis["analysisId"], analysis["revision"], analysis["digest"]):
        raise ValueError("Tracking selection does not belong to this analysis")
    frames = transform["source_frames"]
    sources, detected, absent = transform["source"], transform["detected"], transform["absent"]
    if not isinstance(frames, int) or frames <= 0 or not (len(sources) == len(detected) == len(absent)):
        raise ValueError("Tracker frame facts are incomplete")
    facts = {}
    for frame, hit, missing in zip(sources, detected, absent):
        if not isinstance(frame, int) or not 0 <= frame < frames or frame in facts or not isinstance(hit, bool) or not isinstance(missing, bool):
            raise ValueError("Tracker frame facts are invalid")
        facts[frame] = (hit, missing)
    choices = {s["shotId"]: s["choice"] for s in selection["shots"]}
    shots = analysis["shots"]
    if len(choices) != len(selection["shots"]) or set(choices) != {s["shotId"] for s in shots}:
        raise ValueError("Tracking selection must cover each analyzed shot exactly once")
    lost, unresolved = [], []
    for shot in shots:
        choice = choices[shot["shotId"]]
        if choice["kind"] == "absent":
            continue  # Explicit user absence is not a lost face.
        if choice["kind"] != "target" or choice["candidateId"] not in {c["candidateId"] for c in shot["candidates"]}:
            raise ValueError("Tracking target is not an analyzed candidate")
        first = max(0, round(shot["range"]["startMs"] * fps / 1000))
        last = min(frames, round(shot["range"]["endMs"] * fps / 1000))
        for frame in range(first, last):
            fact = facts.get(frame)
            destination = unresolved if fact is None else lost if not fact[0] or fact[1] else None
            if destination is None:
                continue
            reason = "missing_tracking_frame" if fact is None else "target_not_detected"
            start, end = round(frame * 1000 / fps), min(round((frame + 1) * 1000 / fps), analysis["coveredRange"]["endMs"])
            if destination and destination[-1]["shotId"] == shot["shotId"] and destination[-1]["reason"] == reason and destination[-1]["endMs"] == start:
                destination[-1]["endMs"] = end
            else:
                destination.append({"shotId":shot["shotId"],"startMs":start,"endMs":end,"reason":reason})
    covered = analysis["coveredRange"]
    if abs(frames * 1000 / fps - (covered["endMs"] - covered["startMs"])) > 1000 / fps + 1:
        raise ValueError("Tracking frame coverage differs from analysis")
    reasons = []
    if lost: reasons.append("部分帧未检测到所选人物，请检查预览并重新选择")
    if unresolved: reasons.append("部分帧缺少跟踪数据，请重新分析")
    return {"selectionDigest":selection["selectionDigest"],"coveredRange":covered,"lostRanges":lost,"unresolvedRanges":unresolved,"ready":{"ok":not reasons,"reasons":reasons}}


class H3FaceTrackingReceipt:
    @classmethod
    def INPUT_TYPES(cls):
        return {"required":{"transform":("H3FACEXFORM",),"fps":("FLOAT",{"min":0.001}),"analysis_json":("STRING",{"multiline":False}),"selection_json":("STRING",{"multiline":False})}}
    RETURN_TYPES = ()
    OUTPUT_NODE = True
    FUNCTION = "report"
    CATEGORY = "H3/Face Refine"

    def report(self, transform, fps, analysis_json, selection_json):
        receipt = tracking_receipt(transform, fps, json.loads(analysis_json), json.loads(selection_json))
        return {"ui":{"trackingReceipt":[receipt]},"result":()}


NODE_CLASS_MAPPINGS = {"H3FaceTrackingReceipt":H3FaceTrackingReceipt}
NODE_DISPLAY_NAME_MAPPINGS = {"H3FaceTrackingReceipt":"人脸跟踪验收"}
