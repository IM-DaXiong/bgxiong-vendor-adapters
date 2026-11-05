# RunningHub H3 Spectrum 动态多参独立适配器

当前权威源码：**1.0.1**；公开 SDK：**V2 2.0.0**。本目录与 `packages/cloud-adapters/` 内其他厂商包同级，具有自己的 Cargo workspace，不加入宿主或 SDK workspace，不参与主程序编译。厂商工作流、HTTP 组装及响应解释只在本目录实现。

本包是 Turbo 动态包 `runninghub-h3-dynamic` 的 **Spectrum 质量优先** 孪生：同一套 `build_graph`（删 LoadImage/LoadAudio、注入 1000+/2000+、接线 136、SaveVideo 92→9000），底图与 workflowId 换成 Spectrum。

| 项 | Spectrum（本包） | Turbo（冻结，勿改） |
|---|---|---|
| 路径 | `packages/cloud-adapters/runninghub-h3-spectrum-dynamic` | `packages/cloud-adapters/runninghub-h3-dynamic` |
| pluginId | `local.thirdparty.runninghub-h3-spectrum-dynamic` | `local.thirdparty.runninghub-h3-dynamic` |
| modelId | `runninghub-h3-r2v-spectrum-dynamic` | `runninghub-h3-r2v-turbo-dynamic` |
| workflowId | `2116576147646533634` | `2100758868111486978` |
| 加速节点 | `SpectrumApplyMiniMaxH3`@141 + BasicScheduler steps≈20 | TurboLoRA/TurboSampler + steps≈6 |

## 边界

- 图 **1–3**、音 **0–3**；`inputMode` 固定 `multi_image_to_video`；参考音 = 音色条件；成片音轨 = 模型生成（VAEDecodeAudio → CreateVideo）。
- 提交：`POST /openapi/v2/run/workflow/{WORKFLOW}`，body 为整图 JSON 字符串 + 空 `nodeInfoList`。
- **禁止**把本包 workflowId 写回 Turbo 包；**禁止**在 Spectrum 底图混入 MiniMaxH3TurboLoRA / MiniMaxH3TurboSampler。

## 本地验证

```powershell
cargo test --manifest-path packages/cloud-adapters/runninghub-h3-spectrum-dynamic/Cargo.toml --locked
```

完整 wasm 打包见同目录 `Build.ps1`（需 SDK 工具链）；本阶段以原生 `cargo test` 为准。

## 文档

- 计划：`docs/dev/plans/20261101-PLAN-RunningHub-H3-spectrum-动态多参工作流.md`
- 指南：`docs/dev/guides/20261101-GUIDE-RunningHub-H3-spectrum-动态多参发布与适配.md`

本次修复：schemaJson与摘要生成前统一LF；新版本不得覆盖已安装旧版本。生成的SDK镜像由当前公开CLI init恢复，不手改镜像或锁；包检查通过仍须安装与工作台提交验收。
