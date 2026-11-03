"""P02 real SAM2 multi-keyframe injection probe (upstream API; not production tracking.py).

Loads kijai ComfyUI-segment-anything-2 video predictor, injects two conditioning
frames into one inference_state with the same obj_id, verifies clear_old_points
is per-frame, then propagates towardEnd / towardStart separately.

Environment (override with env vars):
  BGX_COMFY_ROOT   default: L:\\AI\\ComfyUI-Installs\\ComfyUI (1)\\ComfyUI
  BGX_SAM2_WEIGHTS default: <COMFY>/models/sam2/sam2_hiera_base_plus.safetensors
  BGX_SAM2_CFG     default: <node>/sam2_configs/sam2_hiera_b+.yaml

Exit codes:
  0 = probe assertions passed
  2 = environment/deps/model missing (document and stop per stage rules)
  1 = probe ran but contract assertions failed
"""

from __future__ import annotations

import json
import os
import sys
import traceback
import types
import importlib.util
from pathlib import Path


def _die(code: int, msg: str) -> None:
    print(f"PROBE_FAIL[{code}]: {msg}")
    raise SystemExit(code)


def _ensure_pkg(name: str, path: str) -> None:
    if name in sys.modules:
        return
    m = types.ModuleType(name)
    m.__path__ = [path]
    m.__file__ = os.path.join(path, "__init__.py")
    m.__package__ = name
    sys.modules[name] = m


def _load_mod(fullname: str, filepath: str):
    spec = importlib.util.spec_from_file_location(fullname, filepath)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {fullname} from {filepath}")
    mod = importlib.util.module_from_spec(spec)
    mod.__package__ = fullname.rpartition(".")[0]
    sys.modules[fullname] = mod
    spec.loader.exec_module(mod)
    return mod


def discover_env() -> dict:
    comfy = Path(os.environ.get("BGX_COMFY_ROOT", r"L:\AI\ComfyUI-Installs\ComfyUI (1)\ComfyUI"))
    node = comfy / "custom_nodes" / "ComfyUI-segment-anything-2"
    weights = Path(
        os.environ.get(
            "BGX_SAM2_WEIGHTS",
            str(comfy / "models" / "sam2" / "sam2_hiera_base_plus.safetensors"),
        )
    )
    cfg = Path(
        os.environ.get("BGX_SAM2_CFG", str(node / "sam2_configs" / "sam2_hiera_b+.yaml"))
    )
    venv_py = comfy / ".venv" / "Scripts" / "python.exe"
    return {
        "comfy_root": comfy,
        "node_dir": node,
        "weights": weights,
        "cfg": cfg,
        "venv_py": venv_py,
    }


def load_sam2_video(env: dict):
    comfy = env["comfy_root"]
    node = env["node_dir"]
    if not comfy.is_dir():
        _die(2, f"Comfy root missing: {comfy}")
    if not node.is_dir():
        _die(2, f"ComfyUI-segment-anything-2 missing: {node}")
    if not env["weights"].is_file():
        _die(2, f"SAM2 weights missing: {env['weights']}")
    if not env["cfg"].is_file():
        _die(2, f"SAM2 cfg missing: {env['cfg']}")

    # Comfy must be importable for load_torch_file.
    sys.path.insert(0, str(comfy))
    try:
        import torch
        import comfy  # noqa: F401
        from comfy.utils import load_torch_file  # noqa: F401
    except Exception as exc:
        _die(2, f"torch/comfy import failed: {type(exc).__name__}: {exc}")

    root = "kijai_sam2"
    _ensure_pkg(root, str(node))
    _ensure_pkg(root + ".sam2", str(node / "sam2"))
    _ensure_pkg(root + ".sam2.modeling", str(node / "sam2" / "modeling"))
    _ensure_pkg(root + ".sam2.modeling.backbones", str(node / "sam2" / "modeling" / "backbones"))
    _ensure_pkg(root + ".sam2.modeling.sam", str(node / "sam2" / "modeling" / "sam"))
    _ensure_pkg(root + ".sam2.utils", str(node / "sam2" / "utils"))
    try:
        lm = _load_mod(root + ".load_model", str(node / "load_model.py"))
    except Exception as exc:
        _die(2, f"load_model import failed: {type(exc).__name__}: {exc}")

    import torch

    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    dtype = torch.float16 if device.type == "cuda" else torch.float32
    if device.type != "cuda":
        print("WARN: CUDA unavailable; attempting CPU fp32 (may be slow / OOM)")
    try:
        model = lm.load_model(str(env["weights"]), str(env["cfg"]), "video", dtype, device)
    except Exception as exc:
        _die(2, f"SAM2 load failed: {type(exc).__name__}: {exc}\n{traceback.format_exc()}")
    return model, device, dtype, torch


def make_moving_square_video(torch, num_frames: int = 8, h: int = 64, w: int = 64):
    """Isolated synthetic media: white square moves right across black frames."""
    frames = []
    centers = []
    for i in range(num_frames):
        img = torch.zeros((3, h, w), dtype=torch.float32)
        cx = 8 + i * 6
        cy = h // 2
        x0, x1 = max(0, cx - 4), min(w, cx + 4)
        y0, y1 = max(0, cy - 4), min(h, cy + 4)
        img[:, y0:y1, x0:x1] = 1.0
        frames.append(img)
        centers.append((float(cx), float(cy)))
    # SAM2 kijai init_state expects (N,H,W,C) after permute path in production;
    # predictor.init_state wants images as (N,3,image_size,image_size) typically.
    batch = torch.stack(frames, dim=0)  # N,3,H,W
    return batch, centers, h, w


def mask_area(mask_t) -> int:
    return int((mask_t > 0).sum().item())


def main() -> int:
    env = discover_env()
    print("=== P02 SAM2 multi-keyframe probe ===")
    print(json.dumps({k: str(v) for k, v in env.items()}, indent=2))

    model, device, dtype, torch = load_sam2_video(env)
    print(f"device={device} dtype={dtype}")
    print(f"model.class={type(model).__name__} image_size={getattr(model, 'image_size', None)}")

    # Versions / deps evidence
    import platform

    versions = {
        "python": sys.version.split()[0],
        "platform": platform.platform(),
        "torch": torch.__version__,
        "cuda_available": bool(torch.cuda.is_available()),
        "cuda_device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else None,
        "sam2_node": "ComfyUI-segment-anything-2",
        "sam2_weights": env["weights"].name,
        "sam2_cfg": env["cfg"].name,
        "predictor": type(model).__name__,
    }
    try:
        import yaml

        versions["pyyaml"] = getattr(yaml, "__version__", "present")
    except Exception:
        versions["pyyaml"] = None
    print("VERSIONS", json.dumps(versions, indent=2))

    num_frames = 8
    video_nchw, centers, H, W = make_moving_square_video(torch, num_frames=num_frames)
    # Resize to model.image_size like production backend (bilinear).
    image_size = int(model.image_size)
    resized = torch.nn.functional.interpolate(
        video_nchw, size=(image_size, image_size), mode="bilinear", align_corners=False
    )
    # Scale click coords into original HxW space used by normalize_coords in add_new_points.
    kf_a, kf_b = 2, 5
    pt_a = centers[kf_a]
    pt_b = centers[kf_b]

    autocast = (
        torch.autocast(device_type="cuda", dtype=dtype)
        if device.type == "cuda" and dtype != torch.float32
        else torch.nullcontext()
    )

    try:
        with torch.inference_mode(), autocast:
            state = model.init_state(
                resized.contiguous(),
                H,
                W,
                device=str(device),
            )
            # Inject first keyframe
            model.add_new_points_or_box(
                inference_state=state,
                frame_idx=kf_a,
                obj_id=1,
                points=[[pt_a[0], pt_a[1]]],
                labels=[1],
                clear_old_points=True,
                box=None,
            )
            # Capture frame A points before clearing frame B
            obj_idx = 0
            pts_before_b = {
                int(k): v
                for k, v in state["point_inputs_per_obj"][obj_idx].items()
            }
            if kf_a not in pts_before_b:
                _die(1, f"after inject A, frame {kf_a} points missing")

            # Inject second keyframe (same obj_id); clear_old_points must not wipe A
            model.add_new_points_or_box(
                inference_state=state,
                frame_idx=kf_b,
                obj_id=1,
                points=[[pt_b[0], pt_b[1]]],
                labels=[1],
                clear_old_points=True,
                box=None,
            )
            pts_after_b = state["point_inputs_per_obj"][obj_idx]
            if kf_a not in pts_after_b:
                _die(1, "clear_old_points on frame B wiped frame A points (SPEC §8.2 fail)")
            if kf_b not in pts_after_b:
                _die(1, f"frame {kf_b} points missing after inject")
            print(
                f"OK multi-inject obj_id=1 frames={sorted(int(k) for k in pts_after_b.keys())}"
            )

            # towardEnd: forward from earliest conditioning
            toward_end = {}
            for out_frame_idx, out_obj_ids, out_mask_logits in model.propagate_in_video(
                state, start_frame_idx=kf_a, reverse=False
            ):
                toward_end[int(out_frame_idx)] = out_mask_logits.detach().float().cpu()
            print(f"OK towardEnd frames={sorted(toward_end.keys())}")

            # Reset non-cond tracking metadata by re-init + re-inject for a clean reverse pass
            # (production P12 may keep one state; probe validates reverse coverage separately).
            state2 = model.init_state(resized.contiguous(), H, W, device=str(device))
            for fi, pt in ((kf_a, pt_a), (kf_b, pt_b)):
                model.add_new_points_or_box(
                    inference_state=state2,
                    frame_idx=fi,
                    obj_id=1,
                    points=[[pt[0], pt[1]]],
                    labels=[1],
                    clear_old_points=True,
                    box=None,
                )
            toward_start = {}
            for out_frame_idx, out_obj_ids, out_mask_logits in model.propagate_in_video(
                state2, start_frame_idx=kf_b, reverse=True
            ):
                toward_start[int(out_frame_idx)] = out_mask_logits.detach().float().cpu()
            print(f"OK towardStart frames={sorted(toward_start.keys())}")

            # Coverage expectations
            if any(i not in toward_end for i in range(kf_a, num_frames)):
                _die(1, f"towardEnd incomplete vs {kf_a}..{num_frames-1}: {sorted(toward_end)}")
            if any(i not in toward_start for i in range(0, kf_b + 1)):
                _die(1, f"towardStart incomplete vs 0..{kf_b}: {sorted(toward_start)}")

            # Correction frames should have positive mass on the moving square.
            for fi in (kf_a, kf_b):
                # logits shape (objects,1,H,W) or similar
                logits = toward_end.get(fi)
                if logits is None:
                    logits = toward_start.get(fi)
                area = mask_area(logits > 0)
                print(f"correction frame {fi} positive_logits_area={area}")
                if area <= 0:
                    _die(1, f"correction frame {fi} empty after multi-condition propagate")

            # Do NOT OR-merge opposite maps for evidence; report separate coverage.
            overlap = sorted(set(toward_end) & set(toward_start))
            print(f"OK directional maps kept separate; overlap_frame_count={len(overlap)}")
            print("PROBE_PASS")
            print(json.dumps({"versions": versions, "towardEnd": sorted(toward_end), "towardStart": sorted(toward_start)}, indent=2))
            return 0
    except SystemExit:
        raise
    except Exception as exc:
        _die(1, f"probe assertion/runtime error: {type(exc).__name__}: {exc}\n{traceback.format_exc()}")
    finally:
        try:
            if hasattr(model, "reset_state"):
                pass
        except Exception:
            pass
        if device.type == "cuda":
            torch.cuda.empty_cache()


if __name__ == "__main__":
    raise SystemExit(main())