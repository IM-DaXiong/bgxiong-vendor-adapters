# Wan 2.2 Animate 独立适配包 0.2.7

人物替换（Mix）及动作驱动（Move）在提交前自动准备参考图：EXIF方向校正、完整等比缩放、居中放置并补普通中性灰背景。无需把图片手动裁成视频比例，不裁人物、不拉伸、无AI扩图。透明部分在灰底上合成；原图保持不动。可解码且在资源预算内的任意尺寸/比例均走同一准备策略，低清或极端比例参考仍可能缺少有效面部细节。

处理属于此独立包，SDK、通用宿主与原有认证工作流节点接线不变。包内 Comfy companion 提供 `/bgx/wan22-animate/reference` 的GET策略探测及POST准备接口，策略为 `contain-gray-v1`；图片处理在CPU工作线程执行，不加载生成模型。

安装：从软件第三方适配器正式安装入口选择交付目录中的 `adapter-install.json`。须让Comfy加载本包匹配的四个companion文件；旧Comfy进程需在现有任务完成后正常重启。旧组件不提供准备接口时，新提交明确失败，不会偷偷使用被裁头的原图。当前选择迁移由软件统一安装owner处理；不可直接覆盖0.2.0或修改运行任务。0.2.1跟踪预览接受公共 `sourceIdentity`/`range`/`selection` 输入，媒体槽为 `/sourceIdentity/mediaId`。

准备后的PNG位于Comfy input下 `bgx-wan22-reference/`，是普通输入资产；文件名由服务生成，接口不接受input之外路径。原图、源视频、选人点、遮罩与原任务记录均不改写。派生输入按Comfy输入资产管理，不自动删除可能仍被任务引用的文件。

源码打包：运行本目录 `build-package.py --cli <公开SDK CLI绝对路径> --sdk <SDK根绝对路径> --worker <worker绝对路径> --out <新的交付目录绝对路径>`。入口执行build/check/pack/runtime check，从companion唯一源码生成安装文件与逐文件SHA；拒绝覆盖既有目录/ZIP。

本次修复保证参考几何保留，不保证扩散模型每次都精确换脸或去墨镜。完整任务出片、原节点回写、非目标人物保持及人工身份效果需分别验收，实际记录见 `docs/dev/plans/20261101-PLAN-Wan22人物替换参考图尺寸自适应.md`。

## 0.2.0 视频目标多关键帧跟踪预览

新增操作 `local.bgx.wan22-animate/tracking-preview`（profile `video-target-keyframes-v1`）。多关键帧同一 SAM2 状态；nearest-keyframe 归属禁止 OR；mask-only 导出 maskArtifact；Mix 可读同一遮罩不再重跑 SAM。参考准备仍为 contain-gray-v1。不覆盖已发布 0.1.4。

## 0.2.3 跨机器构建

内嵌公共合同及测试fixture仅引用本公开仓contract/v2，不再依赖作者机器盘符。JSON与Python以LF检出，避免生成schema被CRLF误判为过期。companion identity与包同为0.2.3；不覆盖0.2.2。GPU实际验收单独记录，不以编译通过代替。

## 0.2.4 跟踪续办合同修复

跟踪专用续办令牌声明 promptId、trackingRequestId、selectionDigest、direction，普通生成仍用原令牌。修复实际GPU受理后checkpoint校验失败。复用字节未变的0.2.3 companion；安装包版本0.2.4不覆盖旧版。已受理旧任务不得自动重交。

## 0.2.5 跟踪产物身份修复

跟踪导出直接携带SDK已校验的提交inputSchemaDigest，消除错误能力层级读取导致的空摘要；缺项明确拒绝。完整实际查询回执由公开SDK验证，旧版冻结任务与原产物不篡改。companion沿字节未变0.2.3。

## 0.2.6 遮罩任务生命周期

遮罩跟踪通过companion manifest发布成果，prompt成功后不要求Comfy history包含生成视频文件。普通生成仍校验declared output唯一文件。沿用不变的0.2.3 companion与SDK，不覆盖旧包/改写旧任务身份。

## 0.2.7 单任务取消

跟踪取消只POST /api/jobs/{job_id}/cancel，不使用全局interrupt。服务返回true仅表示remote_requested，false为local_only；HTTP或响应错误显式失败，不假称已停止。SDK/companion与生成unsupported合同不改。
