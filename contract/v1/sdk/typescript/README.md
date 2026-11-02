# Vendor Adapter TypeScript SDK

契约发布 **1.2.0** 把可编译的 SDK 放在本仓库的 Rust crate：[`../rust`](../rust)。

本目录没有另一份 TypeScript 库。工作站当前导入路径是 Rust guest（`wasm32-wasip2`）或 `language: "profile"` 的声明式 HTTP 适配器（见 `schemas/profile.schema.json`）。按同一份 WIT（[`../../wit/vendor-adapter.wit`](../../wit/vendor-adapter.wit)）和字段辞典开发时，请使用 Rust SDK 与 `templates/rust`。

计划中的入口形状，仅作对照：`export default async function invoke(op: string, payload: unknown): Promise<unknown>`。一致性样例在 [`../../fixtures/`](../../fixtures/)。
