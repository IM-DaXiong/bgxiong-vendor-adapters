# Breeze TTS 2 A-pack (S1)

独立于主程序的普通合成 A 包：固定 SDK → Rust adapter → 受管 companion → 真实 Breeze 推理 → WAV。

## 范围

- 已实现并作为本包交付目标：`mode=plain`、`delivery=complete`。
- Companion 另含 `voice_design` / `reference_clone` / stream / batch 能力代码；S1 不把它们当作 A 包通过前提，是否实测见 PLAN S1 记录。
- 不改主程序、不安装发行版、不上传 CDN。

## 最短运行（本机已准备权重与 venv 时）

```powershell
# 1) 启 companion（复用已验证 staging 运行根，或指向本包 companion 并自备 store）
$env:BGXIONG_MANAGED_PYTHON = 'E:\BreezeTTS2-assets\s0-staging\python\python.exe'
$env:BGX_TTS_STORE_ROOT = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\_store'
$env:BGX_EXTERNAL_PROVISIONED = '1'
$env:BGX_BREEZE_RUNTIME_ROOT = 'E:\BreezeTTS2-assets\s1-staging\data\runtimes\tts\breeze\1.0.0'
# 也可用 packages/tts-adapters/breeze/scripts/prove-s1.ps1 一键跑通

pwsh -NoProfile -File packages/tts-adapters/breeze/scripts/prove-s1.ps1
```

## 目录

- `adapter/`：speech adapter 源码（WASM）
- `companion/`：从 `58612cd3f` 提取的 sidecar + upstream（不含权重/.venv）
- `tts-bundle.json` / `resources.json`：包描述与资源索引
- `scripts/prove-s1.ps1`：SDK 真实合成证明入口
