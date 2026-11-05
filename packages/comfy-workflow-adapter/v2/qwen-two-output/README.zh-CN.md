# Qwen Image 2.1 单任务双图作者样例

本包复用公开 v2 SDK，不读取宿主源码。一次提交固定生成两张图片，按 ComfyUI 保存节点返回的顺序交付；缺失、额外或重复成果显式失败。此计数属于本操作合同，不是软件批量任务数。

仓库维护者先运行 `Export-QwenExample.ps1 -OutputDirectory <绝对新目录> -Cli <公开CLI路径> -Worker <公开Worker路径>` 导出独立资料。收到独立资料的作者直接进入导出目录，无需仓库或 Export 脚本；按 vendor-adapter-v2/README.md 准备 Rust 1.95.0、wasm32-wasip2 目标和本机原生工具链后，运行 `./New-ComfyExample.ps1 -Project <绝对新项目目录>`，再运行 `./Build-ComfyExample.ps1 -Project <项目目录> -OutputDirectory <绝对新输出目录>`。首次构建需要获取 Cargo.lock 固定依赖；缓存就绪后可加 `-Offline`。

软件中安装 `install/`，导入 `qwen-image-2.1-two-output.source.json` 并关联本包操作；配置自己的 ComfyUI 地址，在图片工作台选择该工作流。输入文字、尺寸、种子和 steps；每次操作应返回两张原图并同时进入历史，默认选中第二张。模型和节点须由用户的 ComfyUI 准备，详见原件内说明。

CLI 使用 `Invoke-QwenExample.ps1` 的 caps、probe、submit、query；submit 输入见 request-t2i.json。query 只续办原受理号，不重新提交。源码与原件身份见 sample-source/SOURCE.json。当前开发版实测已通过，完整安装包仍在 P11 单独交付。

本操作固定两张 PNG，默认选中按原生返回顺序的第二张。其他工作流应有自己的包身份、原件摘要和真实输出声明；不要修改此包的已验原件后仍沿用关联摘要。软件批量两项是后续独立步骤。连接探针仅证明服务可连接，不证明模型、节点或 GPU 已就绪。

## 软件安装配置

安装页的“宿主配置”须填写完整 SDK HostConfig，不能只填 endpointBaseUrl。复制 sample-source/host-config.example.json（独立项目中为 host-config.example.json），把 configuration.endpointBaseUrl 和 network.origins[0] 同时改为自己的服务地址，再将完整 JSON 填入安装页；随后选择标准构建输出的 install 目录。此样例是匿名受信局域网配置，不含密钥。需要鉴权的服务须按公开 SDK 配置凭据，不把真密钥写进样例或安装目录。budgetMs、bindingId、bindingRevision、configurationRef、trust、transport 等字段沿用完整样例，media 由宿主按次提供，禁止手填进安装配置。

输出父目录须已存在，输出目录本身须不存在。SDK 的实际构建目标是 wasm32-wasip2；不要照其他 WASM 模板配置 wasm32-unknown-unknown。软件须包含 P04 的图片列表回收实现；旧程序通过单图不代表已支持本包的双图回收。

2026-10-23 P04 开发版验收：独立4项包测试及标准 build/check/pack、软件安装和原件修订1关联通过；同一远端任务518c7632-de3d-4b36-a27c-cce236c59d08实际采样生成两张1536方图，受理后软件正常退出，重开以原号回收，目标历史恰好两版本，两张分别预览/切换/另存，文件字节与ComfyUI原件一致。SDK192文件和CLI/worker固定；测试断言、验收路径及安装配置三项失败已按原记录保留并最小修正，未删除失败。新主程序基线986959364＋本地P04，SHA90a4b565…；非NSIS发行验收，阶段同步状态见项目PLAN §11。
