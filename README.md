# bgxiong-third-party-adapters / 比格熊第三方适配器

本仓库是比格熊第三方适配器的独立开源真源，与主程序仓 `bgxiong-ai-story` 分开维护。适配器的文档、SDK、合同、模板、案例和更新以本仓库为准；主程序只按契约导入并运行插件。不要把适配器开发写回主程序仓库。

现名 `bgxiong-third-party-adapters`；公开地址 https://github.com/IM-DaXiong/bgxiong-third-party-adapters 。旧名 `bgxiong-vendor-adapters` 已更名并指向本仓库，不要再当成另一份产品。

许可见 [LICENSE](./LICENSE)（不是 MIT）：个人使用和在自有项目/产品中商用都可以；禁止将本仓库内容原样或改造后作为独立商业产品、付费包或付费适配器库出售。

2026-11-05统一：本目录为开发和Git唯一根，公开仓库 https://github.com/IM-DaXiong/bgxiong-third-party-adapters 。

- packages/：已合并主线包源码；Wan22以较新0.2.12及原未提交修改为准，其他包从原sources/main/packages合入。
- projects/：独立工程及其唯一 owner（清单见 `projects/README.md`）。
- artifacts/：发行产物；records/：保全与核查证据；这两类本机资料不自动发布。
- contract/、examples/：公开合同与教学材料。
- 用户分发目录统一名称bgxiong-third-party-adapters；实际新发行落盘和安装验收另记，目录合并不等于全包构建通过。

## 公开同步台账（2026-11-06）

- 开源远程：`https://github.com/IM-DaXiong/bgxiong-third-party-adapters`（原 `bgxiong-vendor-adapters` 已更名指向同一仓库）。
- 主线包 SDK 依赖改为本仓 `contract/v2`，去掉对本机 `bgxiong-ai-story` / `L:/00Dev/...` 的硬编码路径。
- 纳入 RunningHub H3 作者材料、seed 接线冒烟工程，以及 AutoDL 作者占位规范；本机安装验收目录与 AutoDL 参考 API 副本仍不入库。
- 发行产物与核查证据仍在本机 `artifacts/`、`records/`，不随公开同步。

## 原公开仓说明（名称已统一）

# 开发与使用说明

开源的 **第三方算力平台适配器** 开发文档、SDK、模板与脱敏案例。  
服务 [**比格熊数字导演工作站 / BGXiong**](https://www.bgxiong.com)：把本仓库交给智能体，再提供目标平台的 API 说明（与密钥），智能体即可按契约开发可导入的适配器插件。

> 与产品仓 `bgxiong-ai-story` **完全独立**：不包含 Host / UI / Rust 运行时；**禁止回写**产品私有实现。产品内契约路径为 `packages/vendor-adapter-contract`（SSOT 仍在产品仓）；本仓为对外开发者发行版。

---

## 智能体怎么用（核心场景）

1. 打开本仓库，先读 `AGENT.md` 与 `contract/v1/HOST_PAYLOAD.zh-CN.md`（或 `.en.md`）。
2. 用户补充：**第三方平台 API 文档**（鉴权、创建任务、查询、下载结果）+ 测试用密钥。
3. 智能体按 `contract/v1/templates/rust` 与 `contract/v1/wit/vendor-adapter.wit` 实现 guest 适配器。
4. 填写 `manifest.json`（对照 `contract/v1/schemas/manifest.schema.json`）。
5. 对照 `examples/` 与 `contract/v1/fixtures/` 自检；用 `scripts/validate-example.py` 做基础校验。
6. 在比格熊客户端中导入生成的适配器包并联调。

---

## 人类开发者怎么用

1. 官网安装 BGXiong：[https://www.bgxiong.com](https://www.bgxiong.com)
2. 阅读 `docs/USAGE.zh-CN.md` 与 `docs/DEVELOPING.md`
3. 从 `contract/v1/templates/rust` 复制模板，或参考 `examples/`
4. 构建 wasm32-wasip2 适配器后，在客户端「第三方适配器」入口导入（以实际 UI 为准）

---

## 目录

```text
AGENT.md                 智能体开发手册（优先读）
contract/                契约发行树（源自 vendor-adapter-contract）
  README.md
  v1/
    HOST_PAYLOAD.*.md    Host ↔ Adapter 字段词典
    wit/                 WIT 接口
    schemas/             JSON Schema
    sdk/                 rust / python / typescript / tinygo
    templates/rust/      可复制模板
    fixtures/            能力样例 JSON（脱敏）
    examples/            随包案例源树
examples/                顶层案例快捷入口（与 contract/v1/examples 同步）
docs/                    USAGE / DEVELOPING
scripts/validate-example.py
community/
```

---

## 案例

| 案例 | 说明 |
|------|------|
| `examples/comfyui-lan-minimax-h3` | 局域网 ComfyUI + MiniMax H3 标本 |
| `examples/comfyui-lan-z-image-turbo` | 局域网 ComfyUI + Z-Image Turbo 标本 |
| `examples/lan-openai-compat-text` | 局域网 OpenAI 兼容文本标本 |
| `examples/runninghub-minimax-h3-r2v` | RunningHub + MiniMax H3 R2V 视频标本 |
| `examples/runninghub-workflow` | RunningHub 工作流类适配标本 |
| `examples/video-reference-media-probe` | 视频参考媒体探活标本 |

标本仅供对照，**不是**标准本身；标准以 `HOST_PAYLOAD` + `wit` + `schemas` 为准。

---

## License

**自定义许可**（不是 MIT）：个人与商用均可使用；**禁止**将本仓库内容原样或改造后作为独立商品出售。全文见 [LICENSE](./LICENSE)。

---

## About BGXiong / 关于比格熊

**比格熊数字导演工作站（BGXiong）**是装在你电脑上的 AI 导演工具：从故事到分镜、生图/生视频与导出，尽量在同一桌面客户端完成——数据在本地，模型与算力通道由你选择。

- 官网 / 下载：[https://www.bgxiong.com](https://www.bgxiong.com)
- 版本说明：[https://www.bgxiong.com/client/version.html](https://www.bgxiong.com/client/version.html)

---

## 联系我们 / Contact

公众号搜索 **天途影像**，可在后台私信联系。

WeChat Official Account: search **天途影像** and message us in the backend.

## 与主仓同级克隆（跨机器）

主程序仓为 `bgxiong-ai-story`。验收/开发适配器时，默认把本开源仓克隆到其同级目录 `../bgxiong-third-party-adapters`（不要写死盘符）。

```powershell
git clone https://github.com/IM-DaXiong/bgxiong-third-party-adapters.git <实际开发根>/bgxiong-third-party-adapters
```

本仓库为第三方适配器 **Git 真源（公开）**。本机若另有产物/缓存根目录名，不得替代本仓库作为 git 远程。
