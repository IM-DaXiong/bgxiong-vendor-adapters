# S3：A→B 升级与引用保护（T15–T16）+ 第二引擎（T17）

## T15 产物
| 包 | 版本 | adapter.wasm sha256 | 差异 |
|---|---|---|---|
| A `dist-pack/` | 1.0.0-a | `40570a1ead1b92bc233d1650d9feeeff518eede5b6c75b20fbcc8bcd60074a3f` | 忽略 seed |
| B `dist-pack-b/` | 1.0.0-b | `12aea70e2d8f4030cf746c6b7fc8b052e2af67f4d3b7314ff2c3e898b4ea4b6f` | 转发 `seed` → companion `execution.seed`，`appliedParams.seed` 回显 |

SDK 证明：`powershell -File packages/tts-adapters/breeze/scripts/proof-s3-b.ps1` → `S3_B_SDK_SEED_PASS`，样音 `E:/BreezeTTS2-assets/s3-staging/out/s3_b_sdk_seed.wav`。

## T16 应用内验证（不改宿主合同）
1. 设置 → SDK v2 安装 A（`dist-pack` + `s2-host-config.speechManaged.json`）。
2. 再安装 B（`dist-pack-b` + 同一 speechManaged JSON）。二者 `pluginId` 相同、`version` 不同，应并立。
3. 对同一版本目录再装一次应 `ALREADY_INSTALLED`（同版本不同内容亦拒）。
4. 选 A 模型合成一次；在途任务未完成时卸载 A 应 `ADAPTER_PACKAGE_TASKS_RUNNING` / `ADAPTER_PACKAGE_IN_USE`。
5. 任务结束后可卸载 A；历史 checkpoint 仍钉 A 字节（不可再 query 已卸版本属既有行为）。
6. 后续调用改选 B 模型；`seed` 可观察（appliedParams）。
7. **不**改 H1 宿主代码路径做回退：回退=重新选择 A 模型或重装 A 版本包。

## T17 VoxCPM
见 `packages/tts-adapters/voxcpm/`。真实合成依赖本地 checkpoints（约 4GB CDN/HF）；缺资源时只交包脚手架与构建，不冒称多引擎 plain 已验。
