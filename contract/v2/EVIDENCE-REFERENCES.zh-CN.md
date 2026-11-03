# 作者证据引用与原件组合合同

本合同约束 `INDEPENDENT_AUTHOR_EVIDENCE_V1` 到 `external-author` joined 链的引用路径。只组合引用，不改作者 proof、command record、stdout/stderr、Invocation 或历史摘要；不执行作者命令，也不据索引创建成功宣称 C36 或 SDK_READY。

## 唯一引用形状

作者路径的文件引用必须恰有 `root`、`path`、`sha256` 三字段。`root` 是 `[A-Za-z][A-Za-z0-9_-]{0,31}` 内的已授权名字；`path` 是该根内规范 POSIX 相对路径，不含绝对前缀、反斜杠、父目录跳转、空段或文件名别名；`sha256` 是原文件字节的 64 位小写十六进制 SHA-256。selector 必须恰有 `file` 和 `pointer`，其中 file 是上述三字段引用，pointer 是 RFC6901 或空串。

作者路径不接受省 root 的两字段引用、字符串日志、未知字段或目录授权字段。文件缺失、摘要错误或授权失败直接拒绝；不在其他根重试，不回读旧索引。源码库存里的 `{path,sha256}` 是身份行，不是取得文件访问权限的引用，不按本合同补 root。

`evidence` 表示调用方明确提供的证据根，`sdk` 表示工具所用 SDK；其余根由调用方通过 `--artifact-root NAME=绝对目录` 逐一授权。JSON 中的路径、artifactRoots、candidateRoot 或 alias 声明都不能增加目录授权。绝对 argv 是原始执行数据，也不因此授予读取权限。

## 导入上下文

导入原 proof 时，selector.file.root 对应的已授权目录成为原 proof 内部 `root=evidence` 的局部解释。它不取 proof 文件的父目录，不按文件是否存在推断根，也不受外层同名 evidence 文件影响。pointer 选择子对象不改变这一上下文。其他显式 alias 保持调用方原有授权，不自动重命名或扩大范围。

例如原 proof 位于某个授权根的 `subdir/proof.json`，内部 `root=evidence,path=stdout.bin` 仍指向该授权根下的 stdout.bin；不是 subdir/stdout.bin。两个作者可各用局部 evidence 名，但它们的导入 selector 必须明确选择各自的根。若两份原件使用同名显式 alias 指向不同目录，调用方须分别提供明确的上下文，不能让 JSON 自行覆盖授权。

## 新组合索引

`tools/compose_author_evidence.py` 只创建 `AUTHOR_JOINED_REFERENCE_INDEX_V1`，字段为：

- `schemaVersion=1`、`kind=AUTHOR_JOINED_REFERENCE_INDEX_V1`。
- `independentAuthorEvidence`：指向原 proof 的 selector。
- `executions`：按原 proof 顺序指向每条原 command JSON 的 `{record:selector,expectedExitCode:int}`；退出码取自对应原记录。
- `newOperationInvocation`：指向原 Invocation 的 selector。
- `sdkReady=false`、`authorCommandsReexecuted=false`。

索引只在新 outer selector 中，把原件局部 evidence 解释为调用方明确选中的导入 alias。原 proof 与 command 的字节不变，command 内 stdout/stderr/executable 的引用也不重写。索引不能作为新的执行记录，不能把当前源码摘要补记成历史编译观测。

该工具验证组合所需引用、命令记录和流的字节/授权关系，但不是完整作者验收器。完整原 proof 字段、候选库存、浏览器导出、参数身份、实际媒体、失败/恢复链和进程清理仍由 `check_author_evidence.py` 验证。外层 joined 链还需自己的 sourceClosure 和实际 binaryIdentities；这些不能从索引凭空生成。

external-author 消费者重建同一索引并要求 executions 和 newOperationInvocation 完全一致。binaryIdentities 的 selector 及其前后 CLI/worker 文件引用也必须明确 root。合法原始失败命令保留其非零退出码，不改写成成功。原件不足或仍为旧式缺 root 记录时明确拒绝，不生成“修正过的原记录”来补过验收。

## 显式 CLI 用法

先由调用方明确创建一个与原件、候选及其他授权输入根分离的新外层证据目录。输出父目录必须已存在；工具只以独占创建方式写一个新索引，不覆盖既有文件，不在输入根内落盘。下面是参数模板，`CAPTURED_PROOF_SHA256` 必须替换为原 proof 的实际已记录摘要，目录也须由调用方明确提供：

```sh
python tools/compose_author_evidence.py \
  --evidence-root /new/joined-evidence \
  --author-proof-root authorEvidence \
  --author-proof-path proof.final.json \
  --author-proof-sha256 CAPTURED_PROOF_SHA256 \
  --artifact-root authorEvidence=/original/author/evidence \
  --artifact-root author=/original/author \
  --artifact-root candidate=/immutable/candidate \
  --out /new/joined-evidence/author-reference-index.json
```

只授予原件确实需要的 alias；未使用的 alias 不必增加。CLI 的 proof pointer 为空，读取整个原 proof；Python `Context.author_index(selector)` API 支持明确的 RFC6901 pointer。`--help` 仅显示参数，不创建索引。

SD31 作者消费者只读取候选公开包中的这一组合工具，明确使用已经由 CLI 授权的 authorEvidence 目录；候选缺少新工具时失败，不转用旧投影或开发仓工具。它只写新索引及其外层 joined 清单；原作者文件和摘要不变。

## 明示尚未迁移的范围

本批仅收口 external-author。下列 13 条链没有完成本合同迁移，不能因作者路径通过就宣布全部 joined 已统一：

<!-- UNMIGRATED_JOINED_CHAINS -->
```json
["real-probe","three-packages","upload","multi-output","stream-events","recover","unknown-acceptance","cancel-isolation","replay-current","profile-parity","form-cli","matrix","candidate"]
```

这些消费者仍可使用已有 `Context.legacy()` 两字段/字符串/绝对路径合同；某些相对旧记录仍在多个既有授权根中查找并拒绝歧义。SD31 candidate/repro 路径仍生成明确标记为派生物的 command projection。它们均未改成作者路径的新索引，相关历史字节及历史验收结论保留，不能当作新合同的当前正例或在作者路径失败后启用。

将来要声明“当前必需消费者全部迁移”或封存/发布，必须先逐条审查这些剩余消费者，缺口未闭合不得靠作者路径的局部成功掩盖。当前公开源码尚需后续独立封存与受影响验收；本合同及工具不授予构建、导出、晋升或模块接入权限。
