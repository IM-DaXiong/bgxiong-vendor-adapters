# 比格熊第三方适配器统一规范

- **三仓平级唯一治理规范（用户2026-11-07）**：先定位同级主仓 `../bgxiong-ai-story`；主程序、`../bgxiong-content-skills`、`../bgxiong-third-party-adapters` 各为唯一源码 owner，禁止另建真源或在主仓消费副本维护 Skill。全部产品 Skill（含 H3）只在内容仓治理，提交推送后按锁同步主仓。任务开始必读[唯一规范正文](../bgxiong-ai-story/docs/dev/guides/20261107-GUIDE-三仓平级与创作Skill唯一治理目录.md)；缺主仓时先恢复同级定位，不另抄一份规范。

用户2026-11-05明确：唯一工程名、开发目录、Git仓名及用户可见分发总目录名均为bgxiong-third-party-adapters；中文名比格熊第三方适配器。

- 先发现主仓H，使用同级H/../bgxiong-third-party-adapters。本机L:/00Dev/bgxiong-third-party-adapters。公开远程https://github.com/IM-DaXiong/bgxiong-third-party-adapters，禁止再建平行开发真源。
- 本根已承接原bgxiong-vendor-adapters的Git历史、合同、源码和未提交修改，并合入原sources/main/packages独有包。packages是合并后主线源码；projects是独立项目。旧sources/main不是活动owner，不得再次在其中开发。
- artifacts存标准安装包，records存历史保全；禁止按最大版本覆盖源码，禁止同版本覆盖不同包内容。每包唯一owner，分支独有修改先保全合并。
- 发行逐包核源码版本、已验证产物、安装信封、companion、SDK、license和SHA256，并解包实际EXE核对。WIP/教学材料不冒充已发行可用包；缺项明确列阻断，不静默打旧包。
- 用户安装不依赖我方开发路径。当前引用、历史任务、冻结合同及卸载由统一安装/恢复owner管理，禁止直接删运行包或UPDATE用户库绕过校验。
- 厂商逻辑归独立包，SDK/宿主通用规则归公共owner。公开同步不带密钥、模型、用户媒体、运行库和本机记录；合并目录不构成把未审查项目全量公开的理由。
- 禁强推、reset --hard、覆盖未提交修改及改写已推送历史；本地提交后按用户/主仓授权同步并核对远程。
- 全部智能体开工先读本文件。目录迁移的本机保全和主仓计划不在本公开仓库，本次未能读取产品仓原文；不要把它们写成这里的文件。公开文档入口见仓库 README 的「文档入口」。

- **用户2026-11-06，更新必须同步公开下载**：适配器源码或包更新后正常提交推送本公开仓并核对远程包含提交。通过实际验收的新版本同时发布可下载的完整安装信封、包、companion、LICENSE及SHA256，核下载字节并更新发行锁/下载入口。源码已推送、候选/WIP和草稿Release分别记实，不得称用户已可下载正式最新版。主仓同步记录安装版本和真实验收结果，禁止同版本覆盖。
