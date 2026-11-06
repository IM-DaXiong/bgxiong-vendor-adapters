# 文档链接检查

2026-11-06：公开仓文档入口和失效相对链接的交接说明。

## 命令

在仓库根目录执行：

```bash
python scripts/check-doc-links.py
```

可选 `--show-examples` 列出代码围栏里的示例路径。那些路径不导致失败。脚本不访问外部 URL。

## 失败与跳过

失败：

- 围栏外的 Markdown 链接、图片，以及 HTML `src` / `href`，目标不在本仓库
- 这些链接上的锚点对不上目标 Markdown 的标题
- 围栏外、用反引号写出的仓库路径不存在

不失败：

- 围栏里的命令和示例路径
- `http` / `https` / `mailto`（不下载、不判断对方是否在线）
- 含空格、通配、省略号或占位符的反引号
- 未入库的生成物路径（`dist-pack`、`target/`、zip）

## 本次怎么处理宿主记录

产品仓 bgxiong-ai-story 本次不能读取（GitHub API 404）。宿主计划、修复和验收记录没有迁进本仓库，也没有编造正文。包说明里保留迁入时已经写明的公开结论，并注明宿主原文未核验。

SDK V2 仍以 `contract/v2/release-lock.json` 的 `sdkReady: false` 为预览，不写成已封存发行。
