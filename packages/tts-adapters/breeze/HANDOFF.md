# TTS 外挂宿主分项交接（S5 T24）

> **公开读者：** 下面是 2026-10 迁入时的宿主交接笔记，不是当前发行说明。 指向宿主 docs/dev、主程序版本脚本和未入库产物的记录不在本仓库，本次未能读取产品仓原文，因此去掉了这些死链接，没有另写替代稿。 当前公开身份以 [README.md](README.md) 与 `tts-bundle.json` 的 `1.0.0-d` 为准。SDK 模板见 [contract/v2/templates/tts-adapter/](../../../contract/v2/templates/tts-adapter/)。


> **当前包装交接（2026-10-28）**：runtime/安装资源 **1.0.2**，dependencies **1.0.1**，A **1.0.0-a**。新候选在 `src-tauri/target/release/bundle/nsis/BreezeTTS-CDN-1.0.2-candidate-20261028/`，扫描其 `offline-drop/`。小包一致性及 CDN 门禁通过；全 runtime source-sync 仍有 Singing 两文件漂移，未称全仓通过。详见文末本次包装交接及宿主 FIX 记录（不在本仓库，未核验原文）。

> **本次最终技术状态（2026-10-28）**：离线投放与 B01–B06 源码已完成，本任务未构建新 EXE/NSIS。31 个隔离用例、5 个前端用例、PS5.1 解析及隐藏子进程门禁通过；SDK 专项/完整编译被共享 Singing `contract.generated.ts` 过期阻断，不能写全绿。总验收按用户要求统一交总验收聊天；版本、依赖、原日志与未验项详见宿主计划（不在本仓库，未核验原文）。

> **当前交付（2026-10-27 用户最新指令）**：SDK 语音离线安装代码已接入主线工作区；标准 `data/offline-drop/BreezeTTS/` 已含独立 `adapter-pack/`、侧车与中文说明。软件按「设置 → 本地模型 → 离线包安装」选 `data/offline-drop/` 安装。必须使用编入本次改动的新 EXE。人工安装、合成、试听由用户执行；下文“仅计划/未实施”与旧 E 盘手工导入步骤属于历史记录，当前以宿主计划（不在本仓库，未核验原文）及 install/README.md 为准。

> **当前唯一开发入口（2026-10-27）**：宿主离线安装计划（不在本仓库，未核验原文）。用户要求简化到实际可用，旧 S0–S5/第二引擎/H2 不重新作为前置；本轮仅计划，B 项未实施。

> **2026-10-27 更新**：模型/依赖/Python/SoX 的本机 CDN 资源集和安装配置已准备，隔离离线安装通过。当前入口与剩余宿主接线见 宿主 CDN 记录（不在本仓库，未核验原文）；下文为远程原交接，不代表本轮已编译发版。

日期：2026-10-26（Asia/Shanghai）

> **整合接手请先读：`INTEGRATION.md`**（分支 main / tip 见该文；模型与 CDN 实传不在本机范围）。

## 1. 基础可用交付（H1）
- **状态：可用**
- A 包 `local.tts.breeze@1.0.0-a`（`dist-pack/`，wasm `40570a1ead…`）
- 主程序 speechManaged 安装路径见 `S2-RUN.md` / `scripts/install-s2-into-app.ps1`
- 离线基础套件：scripts/build-breeze-h1-offline-kit.ps1 → `dist/offline/tts/breeze-h1/`；跟踪副本 `dist-offline-h1/`（zip sha256 `6dcfb05e6773…`）；发布暂存见 `publish-breeze-h1-kit.ps1` / `INTEGRATION.md`
- 用户端音质：需人工试听确认；机器 WAV 结构校验不代替听感

## 2. 换包 / 第二引擎证明（S3）
- **A→B**：B 包 `1.0.0-b`（`dist-pack-b/`，wasm `12aea70e2d…`）seed 转发；升级文档 `S3-RUN.md`
- **第二引擎 VoxCPM**：脚手架已交；真实合成缺本机 checkpoints（约 4GB）时不冒称多引擎 plain 已验
- H1 宿主摘要在 A→B 路径按计划不重编验证（见 PLAN A07）

## 3. 可选增强（S4，非基础前提）
- **状态：已 SDK 证明，未 H2 冻结发行**
- C 包 `1.0.0-c`（`dist-pack-c/`，wasm `0362d786ba…`）：stream + `std/speech.batch`
- 宿主事件桥：adapter_host/v2/process.rs 读 `events.jsonl`
- 证明：`proof-s4-c.ps1` → batch/stream PASS
- **T21**：仅 H2 候选；**未**签 H2，不交 D 包增强发行

## 4. 安装版 / 人工验收
- 离线 H1 kit 已刷新：`dist-offline-h1/breeze-tts-h1-1.0.0-a.zip` sha256 `6dcfb05e6773f2924c66e392f4304c68eeee2ea2172761a17d741f27e60ebd13`；A wasm 仍为 `40570a1ead…`；`includesCPack/includesH2=false`
- 应用版本 SSOT：`0.6.1102` → `0.6.1026`（`bump-version.ps1`，发版日 MMDD）。完整 NSIS EXE **本切片未重打**
- CDN：已按既有 publish 路径暂存 `UPLOAD_TEMP` + `data/offline-drop/BreezeTTS` + versioned manifest；匿名 PUT 404；**`cdnUploaded=false`**。详见 `T22-RUN.md`。人工下一步：带写权限上传后跑 `verify-runtime-cdn.ps1 -Runtime breeze-h1`
- 人工试听清单：导入 A → plain 合成 → 播放 → 另存 → 重开可读（最短脚本见 PLAN）

## 5. 后续迁移边界
- 旧 IndexTTS / Piper 等静态实现：**不**自动退役；逐引擎保全后另排期
- 历史任务/checkpoint 仍钉原包字节；卸载受活动守卫保护

## 缺项（诚实）
- T14 GUI 全量人工听感若未在本机签收，保持“待人工确认”
- T17 真实 VoxCPM 合成待 checkpoints
- T21 H2 冻结与 D 包未做
- CDN 实传与 verify 待 ops 凭证；0.6.1026 NSIS 安装包未本切片重打


## 2026-10-28 包装定向修复最终交接

当前小 runtime/安装资源 **1.0.2**，依赖归档 **1.0.1**，A 适配器 **1.0.0-a** 原字节不变。新候选 `src-tauri/target/release/bundle/nsis/BreezeTTS-CDN-1.0.2-candidate-20261028/`，总验收选择其 `offline-drop/`；新 EXE/NSIS 本任务未构建，需总构建消费仓库中的新 bundled manifest/provisioning。旧标准离线 1.0.1 与旧发行保留，本轮未提升/覆盖。

小 ZIP 148516 字节，SHA `de793dc80dd2ea4936970300e57f3d1d65da8f854171f6aee98f2e626b7eed47`；配置、源码、镜像完整路径/SHA 见候选 verification/delivery-hashes.json。52 文件 ZIP/源码/CRC、四镜像、LF 与 ZIP 可复现、旧包/A 保全、拒绝同版本覆写、固定库存大资源硬链接已实查；CDN manifest 门禁 PASS。全 runtime source-sync 已不报 Breeze，但仍报 Singing server/job_store.py、server/main.py 漂移，不能称全仓通过；需其 owner 补匹配包后总验收再查。

根因与原日志：宿主 FIX 记录（不在本仓库，未核验原文）。本轮未运行 Cargo、GPU、GUI、用户安装或出声，未上传 CDN。仅沿现有 stage 脚本生成候选，无新安装系统。

重放（下一次须换未占用版本与新候选目录，不能覆盖本候选）：`python scripts/stage-breeze-release.py --reuse-resources <旧冻结发行根> --out <新候选根>`。
