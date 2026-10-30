# H3 人脸精修独立适配器（第三方包）

- **位置选择**：本包在主仓**平级**目录 `L:\00Dev\bgxiong-h3-face-refine-adapter`（非 git subtree）。Lane B 工作树 `L:\00Dev\wt-face-b`（`codex/face-b`）仅提交指针与 A03 说明。
- **上游锁定**：[ComfyUI-H3-FaceRefine](https://github.com/Carasibana/ComfyUI-H3-FaceRefine) **1.1.2** commit `d8521d14fe0d721d80cd9417fff5a559cbc21aba`（见 `locks/upstream-comfyui-h3-facerefine.sha`）。
- **能力槽**：`videoFaceRefine`；业务操作：`subjects.analyze` / `subjects.preview` / `video.refine`。
- **选脸合同**：`schemas/media-target-selection-v1/` 为计划 §3.2 **包内副本**（Lane A 公开 profile 未合入；合并后需 rebase）。

## A03 隔离

禁止覆盖共享 `custom_nodes`。companion 通过 `--extra-model-paths-config` 挂载包内 `companion/upstream/`。现有公开 `local-runtime-v1` + CodeInventory 足够；**未扩展 host port**。

## 构建（公开 SDK）

```powershell
$env:CARGO_TARGET_DIR = 'L:\00Dev\bgxiong-h3-face-refine-adapter\target-face-b'
rustup run 1.95.0 cargo build --release --target wasm32-wasip2 -j 2
```

需要 wasm32-wasip2 与 toolchain 1.95.0。也可用主仓公开 SDK CLI `init/build/check/pack`（勿相对引用主仓私有 crate）。

## 三操作（A05）

见 `harness/OPS-CHECKLIST.md`。用户 107 帧素材仅本机 env，不入库。
