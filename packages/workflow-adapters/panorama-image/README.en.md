# Local ComfyUI single-image ERP panorama adapter (1.0.2)

The package version is 1.0.2 in `manifest.json`. Changing this heading does not record a new live Comfy run. Live runs stay **NOT_RUN** on the authoring PC.

Package ID: `local.thirdparty.comfy-panorama-image`  
Model: `panorama-image-single`  
Operation: `image-to-panorama`  
Capability: `image.generate` with `requiredFeatures: ["image-projection-v1"]`  
Output: `equirectangular360` at 2048×1024

This package is an independent Cargo workspace initialized with the public SDK template (`vendor-sdk` / `template-lock.json`, author SDK **2.0.0**). Live Comfy runs are **NOT_RUN** on the authoring PC; accept on another machine.

On Windows, provenance lives at package-root [SOURCE.json](SOURCE.json). The workflow file is [workflow/source.json](workflow/source.json). Do not add a second provenance file whose name differs only by case.

See `README.zh-CN.md` for model paths, headless sticker_state, and build commands.
