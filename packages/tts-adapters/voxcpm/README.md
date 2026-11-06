# local.tts.voxcpm A-pack (S3 T17)

第二引擎适配包：与 Breeze 共用 speech-v1 / adapter-v2 宿主，**不**改 H1。

- pluginId: `local.tts.voxcpm`
- version: `1.0.0-a`
- model: `openbmb/VoxCPM2`
- companion: 自 `runtimes/tts/voxcpm` 拷贝；启动依赖本机 checkpoints + wheelhouse（见 manifest remoteArtifacts）。

## 构建

本仓库没有独立的 VoxCPM 打包脚本。当前公开内容是材料包：`tts-bundle.json`、`resources.json` 和 `companion/start-sidecar.ps1`。生成目录不入库。缺权重时 plain 合成仍是未验，见下一节。

## 真实合成前提
- 已准备 VoxCPM2 checkpoints（`config.json` + `model.safetensors`）与 pip 环境
- 缺资源时：包可构建/导入，plain 合成记为 **未验（缺资源）**，不阻断 Breeze A/B
