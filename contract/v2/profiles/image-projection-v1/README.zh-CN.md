# 图片投影 profile v1

协议仍为2；使用现有 `image.generate` 能力和包自己的 operationId。包必须声明 `requiredFeatures: ["image-projection-v1"]`；不支持的宿主在执行前拒绝。没有新增 host import、WIT 字段或执行信封。

该feature无requiredImports，留在已验证包与宿主的协商集合，不写入Invocation.enabledFeatures的旧WIT枚举。调用层使用validator的`package::invocation_features`，按公开feature catalog取需要host import的子集；宿主仍精确核对该子集，完整集合继续控制profile校验与恢复。不要把所有requiredFeatures直接复制到Invocation。

图片投影与文件格式独立。真实MIME、文件名、尺寸和摘要以媒体broker解码原件为准；JPEG/WebP不得改名PNG。HTTP、MediaStat、checkpoint回执先解码同版公开DTO；必填nullable字段不能省，静态图片durationMs可null。MediaStat没有handle，handle取自HTTP media payload；checkpoint.write核对真实递增revision。

输入根字段 `outputProjection` 必填，string，enum 是 `flat`、`equirectangular360` 的非空子集；default 必须属于 enum。固定类型也用单元素 enum。工作台以此作为唯一公共控件，扩展参数区不再重复生成；提交用户确认值，不由默认值覆盖。参考图数量、输入方式、尺寸和其余参数由原 input schema/mediaSlots 声明，与投影类型独立。未声明 feature 的旧包沿原合同运行，不注入新字段。

`flat` 是普通图；`equirectangular360` 是单目、水平360°/垂直180°、完整无裁边的等距柱状投影，源像素宽必须恰为高的两倍。SDK验证结构、媒体身份及可取得的实际尺寸，不保证接缝/语义/画质，也不据宽高比猜投影。

同步 completed 或异步 query succeeded 的原 `data` 使用 [output.schema.json](output.schema.json)：

```json
{"imageArtifacts":[{"mediaId":"out-1","role":"result","projection":"equirectangular360"}]}
```

每个真实 image output 恰有一项，以 media.id 和 role 关联；不按顺序、URL、名称或内容SHA关联。重复、缺失、悬空、非图片引用拒绝；所有图片投影必须等于原请求。实际图像由媒体宿主解码，错误比例不自动裁切、补黑或降为普通图。

accepted 尚无图片，不要求 data。query queued/running/failed/cancelled/expired 的 data 为 `{"imageArtifacts":[]}`，不得发布图片 outputs；成功仍按 outputSlots 和 profile 联合校验，至少一张图。首版不支持 streaming/media_chunk/部分图片；旧普通图包的流式行为不变。取消和超时沿原声明，原受理ID恢复，不重新提交。

retained inspect 的 response/dataJson 及 checkpointResult 保留原 data；历史 handle 不可复用。取回失败不是远端生成失败；保留 checkpoint、原件和错误回执，仅在授权/条件改变后续查原ID。自定义usage的metric及unit都须带包命名空间；不虚构费用。SDK trace 的 managed_port/result_validation/receipt_validation 用于关联失败阶段，不携带私密URL、凭据或原始业务内容。

本目录为公开消费者/作者约定，不替代 bindingId/profileId，也不是 std/ 操作。输入示例的单参考槽可声明 minCount=maxCount=1，禁止静默取多图中的第一张。

2.1.1：媒体broker在解码内存上限内完整解码像素；JPEG使用同一锁定解码器的严格模式，拒绝损坏扫描数据。保留原件字节，不转码修图。
