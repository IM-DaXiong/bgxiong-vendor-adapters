# 开源第三方适配器仓（智能体必读）

- **公开远程**：https://github.com/IM-DaXiong/bgxiong-vendor-adapters （THE third-party adapters git remote）
- **与主仓同级**：发现主仓根 `H`（`bgxiong-ai-story`）后，默认路径为 `H/../bgxiong-vendor-adapters`；`<开发根>/bgxiong-ai-story` 与 `<开发根>/bgxiong-vendor-adapters` 同级。禁止把某个盘符或 `00Dev` 写成唯一规则。
- **本地开发必须用本开源仓做适配器 Git 工作**（branch/commit/push）。不要新建私有 `bgxiong-third-party-adapters` 远程替代本仓。
- **产物/缓存**：个别机器可另有本地产物根（例如历史目录名 `bgxiong-third-party-adapters`）存放构建缓存与安装包；那只是存储，不是 Git 真源。缺省验收机应 `git clone` 本仓到同级目录。
- **合同**：`contract/v1` 与 `contract/v2`；产品级包在 `packages/`；教学标本在 `examples/`。
- 禁止 force-push / reset --hard / 改写已推送历史；禁止提交密钥、token、大型模型权重、`target/`、`.bgx-sdk/`。
