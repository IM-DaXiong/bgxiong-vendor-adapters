# Rust 作者起步模板

本模板是可独立构建的本地同步 echo 示例，不连接服务、不读取环境凭据。它不是外部服务适配或 S3 独立作者验收证据。

前提是安装 Rust 1.95.0 和 `wasm32-wasip2` target。复制整个本目录到空目录，保留 `vendor-sdk/`、`Cargo.lock` 和 `template-lock.json`。第三方 crate 均来自公开 Rust registry；首次联网获取和之后离线构建是两步：

```powershell
cargo fetch --locked
cargo test --locked --offline
cargo build --release --locked --offline --target wasm32-wasip2
```

首次 fetch 下载锁中各平台的完整公开依赖，使离线 metadata 检查也不依赖其它目录的缓存。Rust 编译 target 仍须预先安装。离线前必须完成依赖下载；缺缓存时 Cargo 应明确失败，不能将联网首次构建称为离线成功。

结果位于 `target/wasm32-wasip2/release/bgx_adapter_template.wasm`。`manifest.json` 中全零摘要表示**尚未构建和打包**，不能直接安装或声明包验证通过。SD21 作者 CLI 完成后，`build/pack` 会明确生成 `adapter.wasm`、真实摘要与 inventory；当前本步骤直接使用 Cargo 验证依赖闭包。

开发自己的适配器时修改 `src/lib.rs` 的能力、schema、模型及业务操作，并同步 `manifest.json` 的插件身份与权限。所有服务语义放在作者适配器内。媒体、凭据、HTTP、事件和 checkpoint 使用公开受管端口；不要自己读取用户文件或拿到原始密钥。修改 schema 时重新计算对应真实摘要；示例用公开 SHA-256 实时计算，宿主仍独立验证。

`vendor-sdk/` 与 `template-lock.json` 是同步生成的公开 SDK/WIT 镜像，禁止手改。SDK 维护者用公开包中的 `tools/sync_templates.py --write` 更新，再用 `--check --self-test` 验证。模板依赖锁从公开 workspace lock 裁出作者所需完整闭包，不引用主程序或开发仓库私有 crate。SDK 与作者包的发布许可分别审查；当前未作 MIT 等授权声明。
