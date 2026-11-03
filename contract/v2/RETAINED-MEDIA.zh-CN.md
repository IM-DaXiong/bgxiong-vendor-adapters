# 受管媒体只读工具候选 2.0.1

这是 native tools/reference-host 的候选增量。协议、WIT、author SDK 与 worker 仍为 2.0.0；原已验执行工具单独保留。本候选没有新增生成操作或厂商分支。

## 调用

`bgx-adapter-v2-retained retained info` 返回工具版本、协议版本、root-submission 范围和公开分块预算，不打开账户或工作区。

`retained inspect` 的必需参数为 `--package ABS --workspace ABS --config ABS --request-id ROOT --output ABS`。配置是原 CLI **运行配置**，不是应用安装配置。工具只解析非秘密绑定和信任信息，不读取凭据文件、不构造网络 broker 或 worker。

`retained export` 使用相同参数，另加 `--selection ABS --destination NEW_ABS`。selection 是 inspect 返回的媒体 ID 的唯一子集 JSON 数组。destination 必须是已有父目录下本次排他新建的目录；不能进入源 package、objects、grants、cli 或 staging。工具收据必须是工作区 receipts 下的新 JSON 文件。已有收据或目标不覆盖。

默认只读打开已存在的 `checkpoint-fixtures.sqlite`，缺失或 schema 不符即拒绝，绝不初始化库。产品可加 `--host-checkpoints`，复用 SDK stdio checkpoint port：工具只请求 `{hostCheckpoint:{op:"get",requestId:ROOT}}`，宿主提供原 opaque row；工具不获得 insert/swap 能力。

## 输出与消费

收据包含 `toolVersion/protocolVersion/toolStatus/sourceIdentity/eventSequence/pending/pendingTail/response/checkpointState/checkpointRevision/checkpointResult/usage/media/issues/destination`。

- `toolStatus=complete`、exit 0 只代表读取/导出动作完整，生成仍可能失败。原 Response 和最新标准 checkpoint result 都保留，不伪装互换；消费者不能把工具成功当作生成成功。
- media 的字段为 `id/role/kind/mime/byteLength/declaredOutput/identity/relativePath/sha256/error`。原历史 handle 不作为新授权返回；原 Response/checkpointResult 内的 handle 也不可复用。
- 同 ID 同事实按 `(id,role)` 合并；同 ID 事实变化拒绝；同字节不同 ID 保留。未知输出槽保留导出、标 `declaredOutput=false` 并给 issues，不猜成主结果。
- 一项原件损坏/导出失败时，其他合法原件和收据仍保留；该项 error、toolStatus=partial、exit 1。清理问题进入 issues，不清除已经完成的合法导出。宿主须先消费收据中的实际成果，再呈现明确问题，不能只按退出码丢弃全批。
- reported usage 是累计快照替换，不求和、不估算。缺终态标 pending；未写完末行仅保留完整已确认前缀，终态残行/完整坏行明确拒绝。

标准事件必须通过原 request/递增 sequence/schema 校验。Session 复核原 package/binding/configuration/frozen digest；原件身份重建只发生在 native broker 内，并经 rehydrate/info/read 验真。应用不得解析 Grant 或猜对象文件名。

## 重启和边界

工具合并原 root submit 的已确认事件/Response，以及同一公开 root checkpoint 中已持久的 result/媒体身份；不会执行 submit/query/cancel。查询子调用失败前仅有子 journal、尚未进入 root checkpoint 的部分成果仍未覆盖，不能宣称所有异步部分回收已完成。

导出是原字节的受控副本，按公开 chunk 预算流式写入并验 SHA，sync 后排他发布；源对象、原日志和 checkpoint 不改。音频/视频验真需要配置中显式的受管 probe。本轮实际验收以图片/文件为主，视频编辑 UI 和语音驱动另行分阶段实施。

SDK 工具只负责原件与租约；产品负责最终媒体命名、任务归属、事务幂等、历史和用户另存。不能用工具收据替代软件闭环验收。
