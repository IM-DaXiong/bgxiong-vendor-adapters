# Breeze / TTS H1 整合接手说明

> **公开读者：** 下面是 2026-10 迁入时的宿主交接笔记，不是当前发行说明。 指向宿主 docs/dev、主程序版本脚本和未入库产物的记录不在本仓库，本次未能读取产品仓原文，因此去掉了这些死链接，没有另写替代稿。 当前公开身份以 [README.md](README.md) 与 `tts-bundle.json` 的 `1.0.0-d` 为准。SDK 模板见 [contract/v2/templates/tts-adapter/](../../../contract/v2/templates/tts-adapter/)。


> **当前包装交接（2026-10-28）**：runtime/安装资源 **1.0.2**，dependencies **1.0.1**，A **1.0.0-a**。新候选在 `src-tauri/target/release/bundle/nsis/BreezeTTS-CDN-1.0.2-candidate-20261028/`，扫描其 `offline-drop/`。小包一致性及 CDN 门禁通过；全 runtime source-sync 仍有 Singing 两文件漂移，未称全仓通过。详见文末本次包装交接及宿主 FIX 记录（不在本仓库，未核验原文）。

> **本次最终技术状态（2026-10-28）**：离线投放与 B01–B06 源码已完成，本任务未构建新 EXE/NSIS。31 个隔离用例、5 个前端用例、PS5.1 解析及隐藏子进程门禁通过；SDK 专项/完整编译被共享 Singing `contract.generated.ts` 过期阻断，不能写全绿。总验收按用户要求统一交总验收聊天；版本、依赖、原日志与未验项详见宿主计划（不在本仓库，未核验原文）。

> **当前交付（2026-10-27 用户最新指令）**：SDK 语音离线安装代码已接入主线工作区；标准 `data/offline-drop/BreezeTTS/` 已含独立 `adapter-pack/`、侧车与中文说明。软件按「设置 → 本地模型 → 离线包安装」选 `data/offline-drop/` 安装。必须使用编入本次改动的新 EXE。人工安装、合成、试听由用户执行；下文“仅计划/未实施”与旧 E 盘手工导入步骤属于历史记录，当前以宿主计划（不在本仓库，未核验原文）及 install/README.md 为准。

> **接手先读（2026-10-27）**：宿主计划（不在本仓库，未核验原文）是用户可用剩余工作的唯一执行队列。软件内安装/模型自动登记/正式安装版出声尚未验证；按 B01–B11 补齐，不重跑旧平台计划或高级能力矩阵。本轮只交计划，不自动实施。

> **2026-10-27 本机资源准备更新**：当前发行准备以 宿主 CDN 记录（不在本仓库，未核验原文） 为准。资源发行 `1.0.1`，A WASM 不变；使用 `scripts/publish-breeze-h1-kit.ps1 -AssetsRoot <资产目录>`，不再重写旧 `1.0.0-a` kit。下文是远程切片原交接，原路径与命令只用于追溯。

日期：2026-10-26（Asia/Shanghai）  
分支：**`main`**（本轮成果直接落在主线，与 S0–S5 一致；未另开 feature 分支）  
远程：`https://github.com/IM-DaXiong/bgxiong-ai-story-dev.git`  
当前 tip（写本文时）：`1016fed7a` — `feat(tts): T22 H1 version 0.6.1026 + offline kit CDN staging`  
（若你拉到的 tip 更新，以 `git log --oneline --grep=tts` / 本文件后续修订为准。）

相关文档：[HANDOFF.md](HANDOFF.md)（分项交接）、[T22-RUN.md](T22-RUN.md)（版本/CDN 暂存细节）、[S5-RUN.md](S5-RUN.md)。2026-10-26 的宿主 TTS 方案不在本公开仓库，本次未能读取产品仓原文。

---

## 0. 接手前先读（边界）

- **本机不必**再下模型、大权重、CDN 实传。模型在**别的仓库 / 资产机**已有；CDN 在**另一台机器**打包上传。
- **不要**借机重做整架构、不要把 C-pack stream/batch 塞进 H1 基础包、不要声称 H2 已冻结。
- 冻结 A 包 wasm：**`40570a1ead1b92bc233d1650d9feeeff518eede5b6c75b20fbcc8bcd60074a3f`**（勿改字节冒充升级）。
- 应用版本 SSOT：**`0.6.1026`**（scripts/bump-version.ps1 发版日 MMDD）。完整 NSIS EXE **本切片未重打**——需要安装包时再在有打包环境的机器上按既有 `tauri-build-release` / 一键打包 BAT 构建。

---

## 1. 已完成（可保留）

| 阶段 | 内容 | 关键 tip / 产物 |
|------|------|----------------|
| S0 | speech-v1 合同冻结 + SDK 工具链 | `9f986a5f2` 一带 |
| S1 | Breeze A 包真实 SDK 合成 `local.tts.breeze@1.0.0-a` | `3adb5827e`；`packages/tts-adapters/breeze/dist-pack/` |
| S2 | 主程序 speechManaged 接入 | `ce1d2ca54`；见 `S2-RUN.md` / `scripts/install-s2-into-app.ps1` |
| S3 | B 包 seed + VoxCPM 脚手架（非完整权重验收） | `2ac9682ff`；`dist-pack-b/` |
| S4 | C 包 stream/batch + 宿主事件桥（**非** H1 基础前提） | `efb78fefd`；`dist-pack-c/` |
| S5 | H1 离线 kit + 作者资料 + 交接 | `dedd606e1` … `19b68cdf6` |
| T22 余量 | 版本 bump + CDN layout/离线暂存接线 | **`1016fed7a`** |

### 离线 H1 kit（基础包，A only）

- 跟踪副本：packages/tts-adapters/breeze/dist-offline-h1/breeze-tts-h1-1.0.0-a.zip
- sha256：`6dcfb05e6773f2924c66e392f4304c68eeee2ea2172761a17d741f27e60ebd13`
- meta：同目录 `breeze-tts-h1-1.0.0-a.meta.json`（**`cdnUploaded=false`**，诚实）
- `includesCPack=false` / `includesH2=false`
- 构建脚本：scripts/build-breeze-h1-offline-kit.ps1；发布暂存：scripts/publish-breeze-h1-kit.ps1
- 委托入口：`scripts/build-tts-offline-bundle.ps1 -Runtime breeze`

### CDN 合同（未实传，勿当已可下）

- layout：runtimes/cdn-object-layout.json → stem `breeze-tts-h1` → `BreezeTTS`
- manifest：runtimes/manifests/breeze-tts-h1-1.0.0-a.json（亦拷到 `src-tauri/resources/runtime-manifests/`）
- URL 合同（上传后才 200）：
  - `https://r.bgxiong.com/client/ai-story/runtimes/BreezeTTS/breeze-tts-h1-1.0.0-a.zip`
  - `https://r.bgxiong.com/client/ai-story/runtimes/BreezeTTS/breeze-tts-h1-1.0.0-a.json`
- 本开发机已暂存（gitignore / 本机路径，**非**远程大文件）：
  - `data/offline-drop/BreezeTTS/`
  - `D:\00Dev\UPLOAD_TEMP\client\ai-story\runtimes\BreezeTTS\`（若接手机盘符不同，以 publish 脚本打印为准）

---

## 2. 缺口 / 别人整合时要注意

| 缺口 | 说明 | 谁做 |
|------|------|------|
| CDN 实传 + verify | 匿名 PUT 404；无本机上传凭证。上传后跑 `scripts/verify-runtime-cdn.ps1 -Runtime breeze-h1`，再改 meta `cdnUploaded=true` | **CDN 打包机 / ops** |
| 大模型 / runtimeRoot 权重 | 不在本仓 zip；Breeze 权重在资产机（例：历史路径 `E:\BreezeTTS2-assets\...`）或其它仓库 | **资产机 / 模型仓** |
| VoxCPM 真合成 | 脚手架有；缺本机 ~4GB checkpoints 时勿冒称多引擎 plain 已验 | 资产机 |
| NSIS `0.6.1026` 安装包 | 版本号已 bump；EXE 未在本切片重打 | 发行机按既有 BAT/脚本 |
| T21 H2 冻结 / D 包 | **未做**；C 仅为可选增强候选 | 另授权 |
| T14 GUI 全量人工听感 | 机器 WAV 结构校验 ≠ 听感签收 | 产品人工 |
| 旧 IndexTTS/Piper 退役 | **不**自动退役 | 另排期 |

**整合禁忌：**

1. 不要把空 CDN 链接写成「已可下载」。
2. 不要把 C-pack / stream / batch 打进 H1 基础离线 kit。
3. 不要 force-push / 改写已发布历史。
4. 不要为「完整感」在本机重下多 GB 模型进 git。
5. 合并主仓其它并行分支时：保留 A wasm 哈希与 `includesCPack/includesH2=false` 语义；版本冲突按 `bump-version.ps1` 规则（日历 MMDD）处理，勿无说明回退。

---

## 3. 建议接手步骤

```powershell
cd <repo>\00Dev\bgxiong-ai-story   # 自行定位盘符
git fetch origin
git checkout main
git pull --ff-only origin main     # 若不能 ff，再 merge；禁止 force
```

核对应在 tip 上看到 TTS 提交链（至少含 `1016fed7a` 或其后继）。

**装进本机已有 app（speechManaged）：**

1. 按 `S2-RUN.md` / `scripts/install-s2-into-app.ps1` 安装 A 包。
2. 或从离线 kit：解压 `dist-offline-h1/breeze-tts-h1-1.0.0-a.zip`，Settings → SDK v2 导入 `adapter-pack/`，按 kit 内 `INSTALL.md` 改 `host-config.speechManaged.example.json` 的 runtimeRoot/storeRoot/managedPython。
3. plain 合成 → 播放 → 另存；听感人工确认。

**仅刷新离线 kit / 再暂存 CDN 布局（无需本机上传）：**

```powershell
powershell -NoProfile -File scripts/publish-breeze-h1-kit.ps1 -Force
# CDN 机上传 UPLOAD_TEMP 或 curl -T 后：
powershell -NoProfile -File scripts/verify-runtime-cdn.ps1 -Runtime breeze-h1
```

**作者包（T23）：** 公开模板是 [contract/v2/templates/tts-adapter/](../../../contract/v2/templates/tts-adapter/)。作者不需要产品仓里的私有宿主源码。

---

## 4. 关键路径速查

```
packages/tts-adapters/breeze/
  dist-pack/          A 1.0.0-a
  dist-pack-b/        B 1.0.0-b
  dist-pack-c/        C 1.0.0-c（增强，非基础）
  dist-offline-h1/    H1 离线 zip + meta（跟踪）
  companion/          sidecar 源码
  HANDOFF.md / INTEGRATION.md / T22-RUN.md / S*-RUN.md
scripts/
  build-breeze-h1-offline-kit.ps1
  publish-breeze-h1-kit.ps1
  build-tts-offline-bundle.ps1   (-Runtime breeze)
  bump-version.ps1
  verify-runtime-cdn.ps1         (-Runtime breeze-h1)
  install-s2-into-app.ps1
runtimes/cdn-object-layout.json
runtimes/manifests/breeze-tts-h1-1.0.0-a.json
```

---

## 5. 合并冲突时优先保留

- TTS / breeze 适配器、宿主 speechManaged 接线、离线 kit 与 `cdn-object-layout` 的 BreezeTTS 条目。
- A 包 wasm 与 manifest 版本 `1.0.0-a`。
- 文档中「cdnUploaded=false / 未 H2」等诚实表述，直到 CDN 机 verify 通过再改。

若远程主线版本号与 `0.6.1026` 冲突：以更新的发版日规则为准，并在 PLAN/本文件补一句说明，勿静默覆盖 bump 脚本语义。


## 2026-10-28 包装定向修复最终交接

当前小 runtime/安装资源 **1.0.2**，依赖归档 **1.0.1**，A 适配器 **1.0.0-a** 原字节不变。新候选 `src-tauri/target/release/bundle/nsis/BreezeTTS-CDN-1.0.2-candidate-20261028/`，总验收选择其 `offline-drop/`；新 EXE/NSIS 本任务未构建，需总构建消费仓库中的新 bundled manifest/provisioning。旧标准离线 1.0.1 与旧发行保留，本轮未提升/覆盖。

小 ZIP 148516 字节，SHA `de793dc80dd2ea4936970300e57f3d1d65da8f854171f6aee98f2e626b7eed47`；配置、源码、镜像完整路径/SHA 见候选 verification/delivery-hashes.json。52 文件 ZIP/源码/CRC、四镜像、LF 与 ZIP 可复现、旧包/A 保全、拒绝同版本覆写、固定库存大资源硬链接已实查；CDN manifest 门禁 PASS。全 runtime source-sync 已不报 Breeze，但仍报 Singing server/job_store.py、server/main.py 漂移，不能称全仓通过；需其 owner 补匹配包后总验收再查。

根因与原日志：宿主 FIX 记录（不在本仓库，未核验原文）。本轮未运行 Cargo、GPU、GUI、用户安装或出声，未上传 CDN。仅沿现有 stage 脚本生成候选，无新安装系统。

重放（下一次须换未占用版本与新候选目录，不能覆盖本候选）：`python scripts/stage-breeze-release.py --reuse-resources <旧冻结发行根> --out <新候选根>`。
