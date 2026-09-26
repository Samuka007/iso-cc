# D6: CC 配置隔离用 mountns 路径重定向，不用 CLAUDE_CONFIG_DIR

状态：已接受（2026-09-26）

## 背景

R4 要求沙箱内 cc 与宿主 cc 状态互不污染。两条路线：

1. cc 官方 envvar：`CLAUDE_CONFIG_DIR` 指向 per-profile 目录。
2. 我们自己的重定向：mountns 内把 cc 的默认路径（`~/.claude`、`~/.claude.json`）bind 到 per-profile 存储。

## 决策

路线 2。

## 理由

- **无感**：cc 看到默认路径，无版本相关的行为分支。
- **消灭暧昧**：`~/.claude.json` 是否走 envvar 因版本而异（社区文档证实的现存反例）；路径级重定向下此问题不存在。
- **文件系统锚定**：覆盖集可观测、可测试（写集探针）；envvar 路由集合只能靠读 cc 源码/抓行为维护。

## 后果与边界

- 挂载点缺失（宿主从未跑过 cc）需预创建空挂载点：有界宿主痕迹，doctor 报告，N3 例外条款。
- cc 未来新增 `$HOME` 顶层文件在加入声明集前会泄漏：由 T4 写集探针捕获，发现即补默认绑定 + 测试夹具。
- 会话内 unset `CLAUDE_CONFIG_DIR`。

## 已拒绝

- HOME 级 overlayfs：copy-up 把 agent 对项目文件的写截留进 upperdir，宿主看不到改动，违反 R3。
- 直接共享 `~/.claude`：污染即需求本身。
