# Seedream 5.0 Pro 独立图片 SDK 预览

当前版本1.0.4，精确模型 `doubao-seedream-5-0-pro-260628`，操作 `local.bgx.seedream-image/generate`。只依赖随包公开SDK镜像及锁定registry，不引用应用、native provider或文本包。SDK、CLI、worker保持冻结。

可用运行包：`E:/CodexWork/seedvr2-image-v2-20261025/seedream-image-p1-20261025/package-v104-candidate`。标准build/check/pack和预编译目录check通过；SDK CLI本地HTTP已形成可用预览。软件提交/历史/保存、安装版、真实厂商均未验，不代表主仓全部云端API完整迁移。

## 输入与HTTP

- SDK submit外层modelId唯一映射HTTP model，不在input重复模型字段。prompt原样提交；size省略沿native为2K，显式支持2048x2048/2720x1530/1530x2720；watermark省略false；responseFormat省略url，另支持b64_json。appliedParams只报告冻结input已有键，默认不伪装成用户输入。
- referenceImages为0至10个有序mediaId，须精确匹配公共media lease。单张→image字符串，多张→数组；原件魔数决定MIME，JPEG/WebP不伪装PNG。负面提示词当前native未发出，本预览不声明该输入，过期提交明确拒绝；软件完整迁移前须解决目录与实际消费差异。原生其他任意合法尺寸字符串尚未证明覆盖，支持边界以本包闭合schema为准。
- endpointBaseUrl明确为API根，追加/images/generations；credentialRef仅经broker注入Bearer。JSON请求经受管file/HTTP media body，API JSON响应与CDN原件经media sink，分块读写，大b64不进入单个回执帧。API/CDN分别授权，CDN不带API凭证，不放开任意下载地址。

## 结果、计量与失败保全

遍历完整data，支持原生URL/b64别名，每份有效成果保留，不任意去重。图片经media返回，完整原始API JSON作为provider-response文件输出；原始usage JSON同时保存。input/output/total tokens用公共metric，image_count/generated_images用本包命名空间metric与images单位，零值保留，不估算。

包要求现有 `stream-events-v1`。每份原件完成后经 `host-stream.emit-event` 发media_chunk并等待消费确认，reported usage提前发标准usage快照；最终completed重复引用同一媒体ID。部分结果下载失败、逐项error或空项明确返回真实error，API request_id保留，已确认原件及计量事件仍可定位。没有把部分成功伪装completed，没有伪造远端任务，也没有改SDK。

**消费者必须在租约有效期内消费并持久化事件，按媒体ID合并最终结果，不能把media_chunk视为任务已完成，也不能将usage事件和最终快照相加。** CLI事件日志/原件通过不代表应用host已完成该消费。当前应用仍须另做P2绑定、事件、历史和保存闭环。

提交未收到完整HTTP响应时报ACCEPTANCE_UNKNOWN，不自动重提；未授权CDN、原件MIME不实、下载断开均显式失败。单对象读取32MiB是预览边界。同步操作无query/远端恢复/取消。单响应多个成果与UI批量编排不同，本包不实现批量调度。原件不裁剪、不转码，保存实际扩展名。

## 已执行与复跑

1.0.0保留18项行为结果及部分失败回执边界复现；1.0.1仅用既有事件修正部分成果保全，并补image_count；1.0.2另升版保留前版原件，补request ID、逐项失败与提交不确定性。1.0.2定向14次HTTP覆盖13个受影响行为，包含1772543字节真实可解码PNG的b64输出与参考输入、5份含重复成果、usage有/无、401、未授权CDN、第二图404、逐项error/空项、提交断连、伪MIME原件拒绝和下载断开。未重跑依赖未变的全矩阵；详细原始命令/输入/回执/SHA在迁移计划§8及交付目录。真实厂商调用0次。

验证脚本verify_http.py要求cli/sdk/worker/package/output均为绝对路径，新output须不存在。--minimal先验大URL/大b64；--extended-only复用有效最小结果；--cases逗号列出定向用例。Pillow只生成可解码fixture，凭证是明确的非秘密测试字符串。开发host导入源须在标记workspace内，声明fileName为broker的SHA加真实扩展名。

作者SDK根用于init/build/pack；运行时SDK根及其内部worker用于check/caps/submit。vendor-sdk和template-lock.json禁止手改，build只用作者.bgx-sdk缓存/target。早期静态合同、appliedParams与fixture编排错误均留在计划，不冒称首轮通过。API有效性、出图效果及软件/安装版留人工/后续验证。

1.0.4补齐成果保全顺序：原始响应先发标准事件；usage解析错误仍收取合法图片，但不伪造计量；某个结果/URL失败不跳过后续独立结果，成功原件立即保全，最终仍报真实失败。1.0.3的10项定向HTTP及1.0.4的4项分支复验有原始回执；另保留1.0.3业务错误码被覆盖的失败及修正，不宣称首轮通过。固定SDK、已验旧候选均未改，真实云端仍0次。具体范围及错误记录见迁移计划§9.8–9.9。
