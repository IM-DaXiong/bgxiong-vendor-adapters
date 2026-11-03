"""BGX Wan2.2 Animate tracking companion nodes."""

from .sequence import BGXAnimateFrameSequence
from .tracking import BGXAnimateBidirectionalMask, BGXAnimateMultiKeyframeMask
from .tracking_artifacts import (
    BGXAnimateMaskArtifactExport,
    BGXAnimateMaskArtifactLoad,
    register_routes as register_tracking_artifact_routes,
)
from .reference import register_routes as register_reference_routes

register_reference_routes()
register_tracking_artifact_routes()

NODE_CLASS_MAPPINGS = {
    "BGXAnimateFrameSequence": BGXAnimateFrameSequence,
    "BGXAnimateBidirectionalMask": BGXAnimateBidirectionalMask,
    "BGXAnimateMultiKeyframeMask": BGXAnimateMultiKeyframeMask,
    "BGXAnimateMaskArtifactExport": BGXAnimateMaskArtifactExport,
    "BGXAnimateMaskArtifactLoad": BGXAnimateMaskArtifactLoad,
}

NODE_DISPLAY_NAME_MAPPINGS = {
    "BGXAnimateFrameSequence": "BGX Animate Frame Sequence",
    "BGXAnimateBidirectionalMask": "BGX Animate Bidirectional Mask",
    "BGXAnimateMultiKeyframeMask": "BGX Animate Multi Keyframe Mask",
    "BGXAnimateMaskArtifactExport": "BGX Animate Mask Artifact Export",
    "BGXAnimateMaskArtifactLoad": "BGX Animate Mask Artifact Load",
}

__all__ = ["NODE_CLASS_MAPPINGS", "NODE_DISPLAY_NAME_MAPPINGS"]
