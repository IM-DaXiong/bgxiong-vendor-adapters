# 用法

本仓库是比格熊第三方适配器的独立开源真源：
https://github.com/IM-DaXiong/bgxiong-third-party-adapters

旧名 `bgxiong-vendor-adapters` 已更名，指向同一仓库。本仓库不含主程序；主程序按契约导入你交付的插件。许可见根目录 [LICENSE](../LICENSE)（不是 MIT）。

## 导入到比格熊

1. 安装 [BGXiong / 比格熊数字导演工作站](https://www.bgxiong.com)
2. 按 [开发指南](DEVELOPING.md) 构建适配器（或使用已构建包）。契约从 [SDK V1](../contract/README.md) 进入；[SDK V2](../contract/v2/README.md) 仍是开发者预览（`sdkReady` 为 false）
3. 在客户端「设置 → 第三方适配器」导入或安装适配包（菜单以实际版本为准）
4. 配置平台 API 密钥后做一次探测/试跑；安装成功不等于厂商侧已经出片

## 给智能体

把本仓库交给智能体，并附上第三方官方 API 文档。密钥在客户端导入时填写，不要写入源码。智能体应先读根目录 [AGENT.md](../AGENT.md) 与 [LICENSE](../LICENSE)，再按合同制作可导入插件。包清单见 [packages/README.zh-CN.md](../packages/README.zh-CN.md)。
