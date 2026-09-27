# 25 — execstub 修复：Bash 工具形态（-l 容忍 + 二跳 sock 注入）+ G3 收割日志分记

**What to build:** 修 G1（阻断级）：exec.bash=host 下 cc Bash 工具面 Exit 126——两层失配。依据 = /tmp/iso-cc-exp24/REPORT.md §G1 + E5 复现。

**Blocked by:** 无（24 已完成取证）　**Owner:** 待派

## Specification（REPORT 修复方向原样）

1. `execstub::parse_script` 容忍 `-c` 与 script 之间的 flag 段（如 `-l`）——不再按参数数=2 拒收
2. `worker_env_vars`（execrpc.rs:118-134）注入 `ISO_CC_EXEC_SOCK`（sock 路径已在 ExecChannel 上）→ Bash 工具载荷二跳 shim 经同一通道转发，一次额外 RPC 跳，终态语义不变（脚本由宿主 /bin/bash -c 执行）
3. 不采纳：server 侧解析 Yhe 产物剥壳（引号逻辑复制进 server，脆弱）
4. **G3 顺手修**：reap_adopted 的 /proc/environ 读失败（ESRCH/ENOENT 退出/僵尸窗口）与「确无标记」分支分记，消除误记

## Acceptance

- [ ] E5 复现用例转绿：host 轴下 Bash 工具 `echo probe-$((6*7))` → probe-42（不再 126）
- [ ] hooks/statusline 同管线（同 `-c` 载荷形态）同绿
- [ ] G3：收割日志分记单测；mark 引擎 SKIP 不变
- [ ] clippy -D warnings 绿；nextest 全绿；E1 MCP 链不回归（回归锚：shim 单载荷契约测试保留）

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾
