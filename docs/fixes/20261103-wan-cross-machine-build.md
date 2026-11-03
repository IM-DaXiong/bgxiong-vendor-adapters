# Wan跨机器构建

基线4396ebb，公开仓0.2.2，当前主仓验收发现：build-package最先因CRLF检出误报schema stale；JSON解析后完全相同且LF摘要与声明一致。规范LF检出，不改schema语义。随后真实CLI build日志拒绝comfy.rs内L:/绝对include_str；lib.rs/target_keyframes.rs测试也有同类路径。统一引用本仓contract/v2；不改公共SDK或业务映射。内容修正按规则升0.2.3，companion identity同步。原始日志在配套宿主logs/20261103-target-interaction-wan-*；真实GPU与应用回写验收未完成。
