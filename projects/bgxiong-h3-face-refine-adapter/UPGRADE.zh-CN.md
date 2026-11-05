# 升级指南（作者）

1. 只改本包与受管 companion；不要改主程序/EXE。
2. 上游节点：更新 `locks/upstream-comfyui-h3-facerefine.sha` 与 `companion/upstream/` 到新 commit；同步 `companion/upstream-snapshot.json`。
3. 若 1.1.2→更新版变更 picker 路由或 `H3PerFrameDenoise` 接线，同步 `workflows/` 与 `src/lib.rs` 映射。
4. 重新 prepare → probe → 原子激活；失败不得自动回退执行用户新请求。
5. 当前基线：`d8521d14fe0d721d80cd9417fff5a559cbc21aba`。
