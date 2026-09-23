# BGXiong Vendor Adapters / 比格熊第三方适配器

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
