# media-target-selection-v1（包内副本）

本目录是计划 §3.2 的**包内** schema，因 Lane A 的公开 `packages/vendor-adapter-contract/v2/profiles/media-target-selection-v1` 尚未合入本树。Lane A 合并后应改读公开 profile，并 rebase 本包。

- 坐标：显示方向校正后的源视频像素；原点左上、x 右、y 下；框半开。
- 时间：毫秒，相对所选片段起点。
- 未选择 ≠ absent；candidateId 仅在同一 analysis revision + shot 内有效。
