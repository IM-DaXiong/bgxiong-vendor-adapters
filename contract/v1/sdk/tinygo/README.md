# Vendor Adapter TinyGo SDK

契约发布 **1.2.0** 把可编译的 SDK 放在本仓库的 Rust crate：[`../rust`](../rust)。

TinyGo 和 Rust 一样可以产出预编译的 `adapter.wasm`，不需要额外语言运行时。本目录还没有 Go 绑定或模板，所以没有可以照着编译的库。请使用 Rust SDK 与 [`../../templates/rust`](../../templates/rust)。

若自行写绑定，目标仍是同一 WIT 世界（[`../../wit/vendor-adapter.wit`](../../wit/vendor-adapter.wit)）和同一闭集（[`../../protocol/vendor-adapter-protocol.json`](../../protocol/vendor-adapter-protocol.json)）。Rust README 里的宿主规则同样适用：不要导入 `wasi:sockets` / `wasi:filesystem` / process；出网只走 `host-http`；凭证是不透明句柄；每个响应只能有 `dataJson` 或 `error` 之一。可调的时长、帧率、分辨率必须带非空 binding，并在 submit 的 `appliedParams` 里回显。宿主不解析厂商工作流 JSON。
