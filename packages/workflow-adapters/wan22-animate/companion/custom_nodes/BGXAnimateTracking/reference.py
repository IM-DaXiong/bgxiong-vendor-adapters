"""Adapter-owned reference preparation. CPU only; never changes workflow nodes."""

import asyncio
import logging
import os
from pathlib import Path, PurePosixPath
import tempfile
import warnings

from PIL import Image, ImageOps, UnidentifiedImageError

POLICY = "contain-gray-v1"
BASE_PATH = "/bgx/wan22-animate/reference"
BACKGROUND = (127, 127, 127)


def validate_request(payload):
    if not isinstance(payload, dict) or set(payload) != {"image", "width", "height"}:
        raise ValueError("Expected image, width and height")
    image = payload["image"]
    if (not isinstance(image, str) or not image or "\\" in image or ":" in image
            or any(ord(c) < 32 for c in image) or image.startswith("/")
            or any(p in ("", ".", "..") for p in image.split("/"))):
        raise ValueError("Reference must name a file inside the Comfy input directory")
    for key in ("width", "height"):
        if type(payload[key]) is not int or payload[key] < 1:
            raise ValueError("Target dimensions must be positive integers")
    return image, payload["width"], payload["height"]


def prepare_reference(input_root, payload):
    image, width, height = validate_request(payload)
    root = Path(input_root).resolve(strict=True)
    source = (root / PurePosixPath(image)).resolve(strict=True)
    if not source.is_relative_to(root) or not source.is_file():
        raise ValueError("Reference must be a regular file inside the Comfy input directory")
    # Enforce Pillow's decoded-image safety limit for both source and output.
    with warnings.catch_warnings():
        warnings.simplefilter("error", Image.DecompressionBombWarning)
        Image._decompression_bomb_check((width, height))
        with Image.open(source) as original:
            oriented = ImageOps.exif_transpose(original)
            oriented.load()
            source_width, source_height = oriented.size
            # Integer contain geometry: one target axis is exact, the other never exceeds it.
            if source_width * height > source_height * width:
                content_width = width
                content_height = max(1, source_height * width // source_width)
            else:
                content_height = height
                content_width = max(1, source_width * height // source_height)
            rgba = oriented.convert("RGBA")
            rgba = rgba.resize((content_width, content_height), Image.Resampling.LANCZOS)
            canvas = Image.new("RGB", (width, height), BACKGROUND)
            left, top = (width - content_width) // 2, (height - content_height) // 2
            canvas.paste(rgba, (left, top), rgba.getchannel("A"))

    output_dir = root / "bgx-wan22-reference"
    output_dir.mkdir(exist_ok=True)
    if output_dir.is_symlink() or output_dir.resolve() != output_dir:
        raise ValueError("Prepared reference directory must remain inside the Comfy input directory")
    temporary = None
    try:
        fd, temporary = tempfile.mkstemp(prefix="reference-", suffix=".tmp", dir=output_dir)
        with os.fdopen(fd, "wb") as output:
            canvas.save(output, format="PNG")
        destination = Path(temporary).with_suffix(".png")
        os.replace(temporary, destination)
        temporary = None
    finally:
        if temporary is not None:
            Path(temporary).unlink(missing_ok=True)
    logging.info("[BGXAnimateReference] policy=%s source=%sx%s target=%sx%s content=%sx%s",
                 POLICY, source_width, source_height, width, height, content_width, content_height)
    return {"policy": POLICY, "image": destination.relative_to(root).as_posix(),
            "width": width, "height": height,
            "sourceWidth": source_width, "sourceHeight": source_height,
            "content": {"x": left, "y": top, "width": content_width, "height": content_height}}


def register_routes():
    """Called only by the installed Comfy companion, not by pure image tests."""
    import folder_paths
    from aiohttp import web
    from server import PromptServer

    semaphore = asyncio.Semaphore(1)

    @PromptServer.instance.routes.get(BASE_PATH)
    async def reference_info(request):
        return web.json_response({"policy": POLICY})

    @PromptServer.instance.routes.post(BASE_PATH)
    async def reference_prepare(request):
        try:
            payload = await request.json()
            validate_request(payload)
            async with semaphore:
                result = await asyncio.to_thread(prepare_reference, folder_paths.get_input_directory(), payload)
            return web.json_response(result)
        except (ValueError, FileNotFoundError, UnidentifiedImageError,
                Image.DecompressionBombError, Image.DecompressionBombWarning):
            return web.json_response({"error": {"code": "INVALID_REFERENCE",
                                      "message": "Reference image or dimensions are invalid; preparation did not complete"}}, status=400)
        except Exception:
            logging.exception("[BGXAnimateReference] preparation failed")
            return web.json_response({"error": {"code": "REFERENCE_PREPARATION_FAILED",
                                      "message": "Reference preparation failed; generation was not submitted"}}, status=500)
