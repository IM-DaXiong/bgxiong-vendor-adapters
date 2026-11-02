# Vendor Adapter 1.3.0 升级

1.3.0沿用已有capabilities JSON与WIT世界，新增可选模型级 `promptProjection` 描述；SDK仍平台无关。

协议唯一源 `protocol/vendor-adapter-protocol.json` 新增 `inputModeIds`、`promptProjectionSlots`、`promptProjectionIdMaxChars`。描述schema唯一源为 `schemas/prompt-projection-capability.schema.json`；Rust SDK提供 `PromptProjection::new`、`to_json` 与字段校验。

`profileId` / `version` / `defaultMethodName` / `inputModes` 均必填。profileId为小写稳定标识，version为正整数；名称不得包含目录分隔或控制字符。输入模式无重复且非空，同时声明implementedModeIds时须取其子集。只有video/keyframe槽可声明，后处理槽禁止。

已有插件不声明专属格式且实际能力已就绪时，提示词格式查询明确返回中立generic@1；能力缺失、探测失败或待确认则显式不可用。请求模式须同时符合实际模型implementedModeIds与格式inputModes。未知profile/version不回退Generic；默认标准包缺失不换包。插件应声明已证实支持的实际模式；新增模式须同批落实提交和投影证据。

升级插件时，使用同一份 `protocol/`、`schemas/` 和 Rust SDK，重新编译插件，再用实际目标服务验证提交与查询。离线合同检查不代表厂商任务或成片已经通过。
