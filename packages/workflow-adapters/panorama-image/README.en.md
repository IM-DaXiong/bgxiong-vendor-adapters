# Local ComfyUI single-image ERP panorama adapter (1.0.1)

Package ID: `local.thirdparty.comfy-panorama-image`  
Model: `panorama-image-single`  
Operation: `image-to-panorama`  
Capability: `image.generate` with `requiredFeatures: ["image-projection-v1"]`  
Output: `equirectangular360` at 2048×1024

This package is an independent Cargo workspace initialized with the public SDK template (`vendor-sdk` / `template-lock.json`, author SDK **2.0.0**). Live Comfy runs are **NOT_RUN** on the authoring PC; accept on another machine.

On Windows, provenance lives at package-root `SOURCE.json` because `workflow/SOURCE.json` collides with `workflow/source.json` on case-insensitive volumes.

See `README.zh-CN.md` for model paths, headless sticker_state, and build commands.
