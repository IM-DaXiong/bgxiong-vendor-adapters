# local.tts.voxcpm A-pack (S3 T17)

第二引擎适配包：与 Breeze 共用 speech-v1 / adapter-v2 宿主，**不**改 H1。

- pluginId: `local.tts.voxcpm`
- version: `1.0.0-a`
- model: `openbmb/VoxCPM2`
- companion: 自 `runtimes/tts/voxcpm` 拷贝；启动依赖本机 checkpoints + wheelhouse（见 manifest remoteArtifacts）。

## 构建
见仓库 `scripts/build-tts-adapter-package.ps1`；产物 `dist-pack/`。

## 真实合成前提
- 已准备 VoxCPM2 checkpoints（`config.json` + `model.safetensors`）与 pip 环境
- 缺资源时：包可构建/导入，plain 合成记为 **未验（缺资源）**，不阻断 Breeze A/B
