# S1 最短运行说明（本机）

## 前置
- 权重与 wheelhouse：见 resources.json / E:/BreezeTTS2-assets
- 已验证 companion 运行根（含 .venv）：E:/BreezeTTS2-assets/s1-staging/data/runtimes/tts/breeze/1.0.0
- 固定 SDK CLI/worker：使用本机已构建的公开 SDK 可执行文件。本仓库不提交构建输出；SDK 源码见 [contract/v2/README.md](../../../contract/v2/README.md)。

## 一键
`powershell
pwsh -NoProfile -File packages/tts-adapters/breeze/scripts/prove-s1.ps1
`

## 样音
E:/BreezeTTS2-assets/s1-sdk-prove/out/s1_sdk_plain.wav
输入：{"schemaVersion":1,"text":"你好。"}
