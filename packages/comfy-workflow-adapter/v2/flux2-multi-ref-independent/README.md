# FLUX.2 有序双参考图独立作者包

身份：`local.independent.flux2-ordered-reference@1.0.0`。能力 `image.generate`；操作 `local.independent.flux2-ordered-reference/flux2-multi-ref`；模型标识 `flux2-dev-ordered-two-reference`。固定使用安装公开 SDK/CLI/worker，通用上传、checkpoint与query机制来自公开Qwen双图样例，实际模型图为ComfyUI原生导出的FLUX.2 API图，保留两条独立参考条件链。

输入必须恰好两张不同媒体身份的有序原图和文字。本包容量为2，来自目标图的两个LoadImage，不代表SDK的统一上限。不拼图、不截掉第二图、不改成单图。`workflow/source.json`是用户原件的原始字节，`src/workflow-api.json`是父智能体用ComfyUI原生“导出(API)”取得的28节点图；来源摘要及明确参数化在SOURCE.json。

## 输入与界面

标准根字段沿安装公开样例：`prompt`（1–8000字符，原文传入，不改写）、`width`、`height`（256–4096，16的倍数）、`seed`（0–9007199254740991）、`referenceImages`（恰好两个仅含`mediaId`的对象）和`advanced`。所有这些字段必填；schema的default仅为界面起始建议，缺字段会失败，不补造值。

自定义标量只在`advanced`闭集中：`steps`整数1–50；`guidance`数值0–100；`enableTurbo`布尔。UI声明使用公开number/integer/toggle控件。模型文件名、Euler采样器和两路原生1MP area参考缩放固定。本原生图没有负面提示词条件，故不声明或接受negativePrompt。

参考顺序严格为：数组第1张→LoadImage42→62:45→VAEEncode62:44→ReferenceLatent62:43；第2张→LoadImage46→62:41→VAEEncode62:40→ReferenceLatent62:39。原图受管multipart上传，PNG/JPEG/WebP原字节交付服务，不拿缩略图代替。

输出尺寸采用用户明确确认的width/height。原生图本来由第1张参考图缩放后GetImageSize62:66同时控制latent和scheduler；本包**同时**把EmptyFlux2LatentImage62:47与Flux2Scheduler62:48的width/height改为同一确认值，保留两路参考缩放、VAE和条件链。steps直接写62:48，取代原生step-switch链接；enableTurbo只切62:95所控制的base/Turbo模型分支，**不会暗改步数**。推荐Turbo用8步、base用20步，均由用户明确填写。

返回SaveImage9的恰好一张真实PNG。下载通过受管media sink，检查真实PNG魔数及IHDR尺寸与提交合同一致，再返回`outputs[0]`（kind=media、role=artifact、media含id/kind/handle/mime/fileName/byteLength）；`data.description`给出任务结果。远端受理即checkpoint，查询只沿原vendorTaskId与resumeToken；无自动重提。不支持远端单任务取消，也不调用全局interrupt。

## 构建与标准包

需PowerShell7、Rust1.95.0、wasm32-wasip2和本机MSVC。`vendor-adapter-v2/`为安装公开SDK，封存`vendor-sdk/`与`template-lock.json`未修改。初次锁定依赖已通过公网registry下载到作者自身`.bgx-sdk/cargo-home`，未复制其他作者target/WASM。

```powershell
$Kit = 'D:/my-flux2-kit'
$Project = "$Kit/flux2-author"
# 新复制工程在尚无缓存时去掉-Offline，显式按锁取依赖。
& "$Kit/Build-Flux2.ps1" -Project $Project -OutputDirectory "$Kit/build-next" -Offline
```

输出父目录必须存在，输出目录自身必须不存在。脚本依次执行公开CLI `build --sdk ... --project ...`、`check --sdk ... --worker ... --project ...`、`pack --sdk ... --project ... --out ...`，以及runtime目录的同一check。runtime是严格标准包；install额外含workflow-templates.json供原件关联，其旁置清单不在runtime inventory签名范围内。

行为验证命令如下；先用显式fetch或标准build准备依赖，禁止拿空缓存声称离线构建通过。

```powershell
$env:CARGO_HOME = "$Project/.bgx-sdk/cargo-home"
$env:CARGO_TARGET_DIR = "$Project/.bgx-sdk/native-target"
cargo +1.95.0 test --manifest-path "$Project/Cargo.toml" --lib --locked --offline
```

五个现有行为用例覆盖确认参数写图、保持FLUX原生其他节点、错误参数、双图数量/身份/租约与顺序、PNG尺寸。它们不接网络、不证明GPU或软件入口。

## 配置、CLI与软件安装

`host-config.example.json`为完整公开应用配置。将configuration.endpointBaseUrl和network.origins同时改成实际匿名HTTP(S)服务origin；如需鉴权须另按公开凭据合同明确配置，本示例不覆盖鉴权。不要只提供endpoint的两字段配置。

```powershell
$Runtime = "$Kit/build-1/runtime"
$Workspace = "$Kit/live"
$Endpoint = 'http://YOUR-COMFY-HOST:8188'
& "$Kit/Invoke-Flux2.ps1" -Action caps -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-Flux2.ps1" -Action probe -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint
& "$Kit/Invoke-Flux2.ps1" -Action submit -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -RequestFile "$Kit/request-two-reference.json" -ReferenceFiles @('E:/images/first.png','E:/images/second.png')
# 仅对真实accepted的原submit requestId查询，不再submit。
& "$Kit/Invoke-Flux2.ps1" -Action query -Runtime $Runtime -Workspace $Workspace -Endpoint $Endpoint -SubmitRequestId 'ACTUAL-ORIGINAL-SUBMIT-REQUEST-ID'
```

上述命令为公开操作说明，作者未运行CLI GPU。Invoke脚本会输出实际host-config.app.json、原请求与原始回执。caps与package_validation只验证包；connectivity probe只检查连接，不宣称节点、权重或GPU就绪。

软件设置→第三方适配器→SDK v2填完整配置并安装`build-1/install/`。工作流库导入`flux2-author/workflow/source.json`，在详情明确关联上述身份/版本/操作；在线“检查并更新”后使用图生图。自由画布连接两张原图到目标图片节点，先在现有工作台尺寸控件明确选择1:1并确认显示1024×1024，不能保留“自动”；然后确认顺序与全部参数；重排是会话顺序，“×”仅剔除会话参考。软件真实出图/回收/预览/另存已通过；续办复用既有有效基础结果，最终边界见FINAL-ACCEPTANCE.zh-CN.md。

目标服务需原生节点及四个确切权重：`flux2_dev_fp8mixed.safetensors`、`mistral_3_small_flux2_fp8.safetensors`、`flux2-vae.safetensors`、`Flux2TurboComfyv2.safetensors`。无模型自动下载、替换或降级。SOURCE及logs如实区分原生前提、作者构建、CLI与软件；不将构建绿灯称为用户端可用。

阅读范围仅限安装user-examples公开树和P12 inputs。已收到父智能体纠正初始“所有标量放advanced”的过宽任务文字，并收到原生API及尺寸链提示；不是零提示盲验。实际阅读文件、复制的公开SDK文件和每次正式失败记录在`logs/read-and-failure-register.json`。未读取主仓、宿主私有实现或P01–P11私有QA。

## 本次实测记录

五项行为测试及冻结CLI标准build/check/pack/packed check均通过，作者构建阶段无正式失败，无重复重跑。软件验收已出现下述首次失败，不能称整体无失败。编译得到adapter.wasm SHA256 `70600be9f92f7d22207131fa1655e282933253bbd23b679050ed4894efe7a583`。父智能体提供原生前提回执`cee95881-c127-4087-ad73-2a70f62fde61`，已核对28节点、仅五处允许输入改动、无节点缓存，真实1024×1024 PNG 1,686,137字节。该原生前提是base20步/原图尺寸派生，不冒称Turbo8步参数化包的GPU实测。作者未运行GPU或软件。SOURCE/README仅为inventory外源码文档；此次更新未动已封runtime或其摘要。GUI后续结果由父智能体记录。

父验收补充：冻结软件安装、原件导入与明确关联成功；首次GUI任务446aac94…在4ms内、出网前因“请选择图片尺寸”失败，未产生远端ID或submit回执。工作台当时为画幅“自动”。包与安装公开Qwen样例同样必填显式根width/height、default1024、范围256–4096；差异仅FLUX要求16倍数而Qwen要求8倍数。本包不接受自动尺寸，需在现有工作台明确选择如1024×1024。父验收已确认原因为验收操作漏选显式尺寸；通过现有UI选择1:1、显示1024×1024后再次正常提交获得真实远端ID。没有修改执行源、SDK、宿主或封包，也没有重建。

软件后续状态（父验收报告）：明确选择1:1/1024×1024后，GUI任务`c1df6e96-22e0-475a-8b4d-ee3b854534bc`受理，远端任务`d97bcd1e-612b-4736-9dd7-36a8a7752071`正在运行。冻结payload与确认请求一致：红熊猫/纸机器人两张原图按序、1024×1024、seed20261032、steps20、guidance4、Turbo=false。受理时尚未证明成果回收；随后真实完成结果见下段。首次操作失败仍保留，运行包继续冻结。

P12软件最终验收通过（父验收报告）：同一local任务`c1df6e96-22e0-475a-8b4d-ee3b854534bc`为done，远端`d97bcd1e-612b-4736-9dd7-36a8a7752071`为success，无缓存，耗时141081ms。真实28节点仅含已文档化参数变换，两路服务/view原图逐字节与授权红熊猫/纸机器人一致。输出`Flux2_00042_.png`，1024×1024、1,605,543字节、SHA256 `7d3c4515d08db4cd7fb830bbe5833e3300eafb35943e3d8fafcc09ce34311133`；冻结安装宿主自动回收至新目标且仅一条history，version `b815e2ab-ddc4-440f-8bd4-2f8a7b326c95`。GUI预览和原图另存通过，保存路径`E:/CodexWork/comfy-generation-p12-20261023/p12-flux2-two-reference-original.png`与成果同SHA。应用正常退出0。

本步没有重复退出续办GPU，沿用已验证基础消费者的相应结果；没有实测Turbo GPU、其他模型、额外参考图数量或其他工作台。保留首次尺寸操作失败、父智能体纠正/尺寸链提示，不能改称零失败或零提示。最终文档更新仅涉及runtime inventory外README、SOURCE和登记，执行源码、封存SDK、标准封包与其摘要保持不变；无需重建。
