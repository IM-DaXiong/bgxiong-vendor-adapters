# 普通语音作者起步模板（S0）

使用 CLI 2.0.2：`bgx-adapter-v2 init --sdk <绝对SDK目录> --dir <不存在的绝对项目目录> --template speech`。
然后沿既有 `build --sdk ... --project ... --offline`、`check --sdk ... --worker ... --project ...`、`pack --sdk ... --project ... --out ...` 工作流。

模板可独立构建和读取 capabilities；只声明 `std/speech.synthesize`、同步 complete、一个音频结果，UI 控件为空。没有声明 batch、流式、克隆或声音目录。`src/capabilities.json` 的公共 schema 字节来自 SDK 生成器；作者修改身份、模型列表及实际执行声明，不改公共 schema 的内容或摘要。

**尚未接入模型。** submit 明确返回 `RESOURCE_UNAVAILABLE`，不会用静音 WAV 冒充合成成功。作者在 `src/lib.rs` 的 submit 中完成包内转换，通过现有 `host::call` / HTTP 媒体端口调用自己的 companion，产出真实音频，返回 `completed`、一个 `role=audio` 输出；没有附加信息时 `data=null` 即可。请求正文在宿主 HTTP 出站前自动持久化。

普通输入只有 `{"schemaVersion":1,"text":"你好。"}`。省略 mode 为 plain，省略 execution.delivery 为 complete。选填风格、情绪、时长或附件不支持时，保留原始输入并给未应用提示；不能以缺能力拒绝普通合成。显式选择的模式、声音身份、runtimeProfile 或 stream 不能静默替换。输出附件独立解析，坏附件不能丢弃有效音频。详细合同见 SDK 的 `profiles/speech-v1/README.md`。

S0 不提供 companion、模型权重或主程序安装功能。source/manifest 中 `local.sdk.speech` 是开发身份，发布自己的包时需同时修改这两处身份。生成源 `capabilities.json` 中的 pluginVersion 要与 Cargo.toml、manifest.json 一致。
