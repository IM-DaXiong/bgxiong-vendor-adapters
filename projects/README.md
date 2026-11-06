# projects/

独立工程及其唯一 owner。与 `packages/` 主线包并列；不在此目录按最大版本覆盖主线。

| 工程 | 说明 | 公开范围 |
|------|------|----------|
| `bgxiong-h3-face-refine-adapter` | H3 人脸精修独立适配器（当前基线见包内 SOURCE/manifest） | 源码与合同材料 |
| `runninghub-h3-author-20261027` | RunningHub H3 作者材料（官方节点摘录与工作流源） | `materials/`；本机安装验收目录不入库 |
| `runninghub-seed-wire-check-20261029` | RunningHub seed/参数接线冒烟（`video-controls-wire-smoke`） | 源码 |
| `autodl-h3-author-20261027` | AutoDL H3 作者占位与规范 | 仅 AGENTS；参考 API 副本不入库 |
| `runninghub-legacy-upgrades-20261028` | 历史升级保全 | 默认不入库（见根 `.gitignore`） |

构建与发行产物放 `artifacts/`；核查证据放 `records/`（均不作为隐式公开输入）。
