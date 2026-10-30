# A20：工作流节点定义读取超出响应预算

## 原始事实
- 原任务：d2c05025-2a5a-416a-95c4-800a761da9e2。
- 原请求工作目录：主仓 data/adapter-v2/workspaces/local.thirdparty.h3-face-refine/cli/23e74fac81d0541a8d1406f14491ddda9084103bfb6e6e9c9da103de124c44c2。
- before-send 顺序：分析会话 GET、素材 upload/image POST、三个节点定义 GET、全量 object_info GET；没有 prompt POST。
- 原错误：RESOURCE_UNAVAILABLE / BUDGET_EXCEEDED，HTTP response exceeded its explicit buffer budget。
- 只读实测全量节点定义 2,679,900 字节，超过现有 1,048,576 字节响应预算。实际完整图节点各响应远小于预算，见 a20-node-response-sizes.json。

## 根治范围
共享提交预检从全量定义改为按实际待提交完整图 class_type 去重、逐节点读取。路径片段按 UTF-8 百分号编码；缺失或畸形节点定义显式拒绝。保留原响应预算、参数映射/校验和标准未受理事实记录。预览原有三个重复探测由此统一入口覆盖。没有修改SDK或宿主，没有调整用户参数、生成图或目标。

## 验证
- cargo test --lib --offline：15 passed（新增唯一节点读取、URL编码、缺失节点拒绝行为；原拒绝事实/未知受理用例保留）。
- Build.ps1：wasm32-wasip2 release 成功；Cargo/lock/manifest/SOURCE 均为 1.0.0-a20。
- 唯一正式CLI检查输出与耗时见 a20-real-cli-check.log / a20-real-cli-check-timing.txt，工具身份见 a20-check-runtime-hashes.txt。
- 安装入口：install-a20/adapter-install.json；manifest.sha256 为现有两文件inventory，不覆盖A19。
- 没有启动/停止服务、安装、提交生成或修改数据库。软件真实预览/精修由主线程继续验收，不能用包检查替代。
- 额外只读节点探测显示 SAMLoader 当前不存在，现有SAM显式依赖检查保留；用户当前非SAM路径不受影响，本轮不扩展处理。

## 软件真实链路验收更新

主线程确认 A20 经软件真实分析与跟踪预览已通过：任务 `493873b2…` 为 done/terminal，远端任务 `7071510f…`；绿框目标为建国，0–4458 毫秒无 lost。此结论来自主线程的软件链路验收，不是本报告的包检查或旁路提交。

精修尚未执行：主程序误传 encoded 模型 ID，提交在主程序被阻断。主程序负责线已修复，根线程正在定向测试和重新编译；因此不能宣称精修通过。A20 产物继续冻结，本次仅更新报告，没有新增测试、构建或源码修改，也未提交或推送。
