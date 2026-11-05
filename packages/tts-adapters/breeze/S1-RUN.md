# S1 最短运行说明（本机）

## 前置
- 权重与 wheelhouse：见 resources.json / E:/BreezeTTS2-assets
- 已验证 companion 运行根（含 .venv）：E:/BreezeTTS2-assets/s1-staging/data/runtimes/tts/breeze/1.0.0
- 固定 SDK CLI/worker：packages/vendor-adapter-contract/v2/target/debug/

## 一键
`powershell
pwsh -NoProfile -File packages/tts-adapters/breeze/scripts/prove-s1.ps1
`

## 样音
E:/BreezeTTS2-assets/s1-sdk-prove/out/s1_sdk_plain.wav
输入：{"schemaVersion":1,"text":"你好。"}
