# RunningHub H3 经典七工作流独立适配包

最新独立包 **1.0.3**，公开 SDK **2.0.0**。原 0.7.1 的七个云端 AI App 迁至当前 SDK v2；没有宿主 Rust、主应用 workspace、SDK 内部实现或本地 ComfyUI 依赖。源码可整目录拷出独立构建。

## 保留的工作流

- `h3.r2v.1slot`：图像参考 1 图，App `2108667855574224898`。
- `h3.r2v.2slot`：图像参考 2 图，App `2101956370017906690`。
- `h3.r2v.3slot`：图像参考 3 图，App `2101957517390737410`。
- `h3.r2v.4slot`：图像参考 4 图，App `2103015554197053441`。
- `h3.i2v.turbo`：单图参考 Turbo，App `2101954317581381633`。
- `h3.fl2va.s20`：首帧视频 20 步，App `2107978021675294722`。
- `h3.soundtrack.1ref`：参考音对口型，1 图加 1 段参考音，App `2107982892470198274`。

所有平台地址、App ID、节点和权重 pin 只在本包。每个工作流独立声明素材数量，不能根据图片数量替用户切模型；不得省略素材而继续使用云端烘焙的 LoadImage 默认图。提示词全文和素材顺序原样送到所选工作流。

1.0.2 修复安装后模型未登记：未声明的 seed/advanced 不由宿主强塞；七个操作以标准 resolution 命名枚举声明四个固定 MP 档，对口型操作声明 advanced.audioUsage=soundtrack_copy。需要包含本轮通用视频消费者修复的主程序，仅更换适配器包不能修复旧 EXE 的强制参数门。

时长沿原包 **1～15 整数秒**；画幅 1:1、16:9、9:16、4:3；MP 为 0.25、0.4、0.6、1。模型原生固定 24fps。旧代码把 CreateVideo 封装帧率 25/30 宣传成生成帧率，这里不再声明。参考音只在音频工作流出现，其他工作流不造音频用途控件。

## 安装

安装 Build.ps1 生成的 `install` 根目录（含 `adapter-install.json` 与标准 SDK runtime 子目录 `package`）；不要导入 SDK v1 旧源码、源码 src 子目录或单独 wasm。应用设置 → 第三方适配器 → 导入安装包，选择该安装根目录，按提示录入 API Key；声明已固定中国站和实例类型。真实 Token 通过应用账号/凭据保管器提供，**不填进源码、不编入 wasm、不提交 Git**。当前包不需要本机 ComfyUI 地址。视频原件以 SDK 真实探测结果为准，保留原 MIME／文件名，不按厂商 outputType 或 HTTP Content-Type 硬限 MP4，不设包内视频容器白名单、不改后缀或转码。只接受完整受管视频事实：非零大小／时长／尺寸、真实 SHA 与内容文件名；非视频、空文件或损坏拒绝并保留原任务续查。安装声明仅授权 API 与已观察到的原片来源 `https://rh-comfyui01.tos-cn-beijing.volces.com`；凭据仅对 API 生效，下载不带凭据。新输出域名明确拒绝并保留原任务，须核实后更新独立包。

安装成功应自动登记七个模型，也保留“加入视频模型”按钮供重新登记。请显式选择新登记的七个模型之一，旧 v1 模型选择不会被静默替换。宿主须已包含 Installed 来源不经过 ComfyUI 地址门控的修复（主仓 c906b1b2f）。


2026-10-29 人工续验发现：若生成时报“视频作者参数格式无效：missing field seed”，问题在宿主 typed IPC 请求仍强制 seed/advanced。宿主必须使用 Option 表达未开放控件，schema.required 决定缺值拒绝；本包1.0.2无需重打或重装，需更新主程序。不能在包里填假种子绕过。宿主故障记录不在本公开仓库，本次未能读取产品仓原文。当前包版本以本目录 `manifest.json` 的 1.0.3 为准；上文 1.0.2 只描述那次不必重打的修复。

## 独立构建与打包

从应用发行的作者资料取得同版完整公开 SDK（含 bin、profiles、protocol、templates、registry/工具）。执行：

```powershell
.\Build.ps1 -Sdk '绝对SDK目录' -OutputDirectory '尚不存在的绝对安装包目录' -Online
```

默认离线；仅首次依赖尚未缓存时使用 `-Online`。脚本依次执行标准 build、source check、pack、runtime check。SDK 镜像和 template-lock 是公开 SDK 的原件，禁止手工修改。Cargo.toml 自有 workspace 且仅依赖随包公开镜像，不引用主应用路径。

原生回归：`cargo test --manifest-path Cargo.toml`。公开 SDK 验证执行 `Verify-PublicContracts.ps1 -Sdk <公开SDK绝对路径> -InstallationDirectory <安装根目录> -OutputDirectory <新验证目录>`，它读取本包实际模拟执行的能力／用量成果，再调用公开 SDK validator 与 NetworkPolicy，校验精确授权和拒绝反例，不出网、不计费。测试通过模拟受管 Host 覆盖七个工作流实际映射、上传/原文/权重钉表、独立续查、凭据隔离、非法输入无副作用及提交回执不确定性。

## 验证范围

本轮真实云端调用 **0 次**，用户要求人工安装和界面验收，不自动替用户安装。build/check/pack 证明标准包与协议可用，模拟提交不等于云端出片。远端 App 权重是否仍存在、账号权限、生成内容、实际工作台生成和播放由用户安装后试验。失败任务不自动重提，不改参数兜底，不把下载失败伪装为完成。

本次修复：schemaJson与摘要生成前统一LF；新版本不得覆盖已安装旧版本。生成的SDK镜像由当前公开CLI init恢复，不手改镜像或锁；包检查通过仍须安装与工作台提交验收。
