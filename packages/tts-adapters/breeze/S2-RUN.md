# S2：主程序接入 A 包（local.tts.breeze@1.0.0-a）

## 前置
- S1 已通过，人工确认可听 `E:/BreezeTTS2-assets/s1-sdk-prove/out/s1_sdk_plain.wav`
- 本机 companion 运行根：`E:/BreezeTTS2-assets/s1-staging/data/runtimes/tts/breeze/1.0.0`
- 不扩展 `TtsEngineId`；经 `speech_vendor_adapter` + `sdk-speech-v2.*` 模型走 v2

## 安装
```powershell
powershell -NoProfile -File packages/tts-adapters/breeze/scripts/install-s2-into-app.ps1
```
1. 打开应用 → 设置 → SDK v2 作者开发包
2. 「安装适配包」选择 `packages/tts-adapters/breeze/dist-pack`（或 S1 prove pack）
3. 安装配置粘贴 `packages/tts-adapters/breeze/s2-host-config.speechManaged.json` 全文
4. 安装成功后点「加入语音模型」

## 应用内验证
1. 打开 TTS 工作室 / 对白语音
2. 选择 provider `speech_vendor_adapter`，模型为 `adapter:local.tts.breeze:sdk-speech-v2.…`
3. plain 输入「你好。」→ 合成 → 播放
4. 首次合成会拉起 managed companion（可能较慢，等 health modelLoaded）
5. Creator/MCP：同一模型绑定后走同核

## 不做
- 不进 S3（B 包/第二引擎）
- 不改 speech-v1 / local-runtime-v1 合同
