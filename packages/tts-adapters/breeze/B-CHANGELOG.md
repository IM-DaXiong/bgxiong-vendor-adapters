# Breeze B-pack 1.0.0-b

相对 A-pack `1.0.0-a` 的**唯一业务差异**：

- 当冻结输入含 `seed` 时，转发到 companion `execution.seed`，并在 `appliedParams.seed` 回显（键 ⊆ 输入）。
- A 包忽略 `seed`（companion 走默认 seed）。
- `pluginId` 不变：`local.tts.breeze`；版本：`1.0.0-b`。
- A 产物保留：`dist-pack/`（adapter.wasm sha256 `40570a1ead1b92bc233d1650d9feeeff518eede5b6c75b20fbcc8bcd60074a3f`）。
- B 产物：`dist-pack-b/`（构建后写入）。

不改 speech-v1 / local-runtime-v1 合同，不改 H1 宿主。
