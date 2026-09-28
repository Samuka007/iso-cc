# 26 — 拦截去注入化：L3 bind + L2 承担全部拦截，移除 CLAUDE_CODE_SHELL_PREFIX 注入

**What to build:** host 模式的拦截从「注入 L1.5 环境变量」改为「L3 mountns bind + 既有 L2」——cc 的会话 env 中不再出现任何点名 hook 的变量。动因（操作者 2026-09-28）：`CLAUDE_CODE_SHELL_PREFIX` 可被 cc 自身与任意子进程观测（env 是公开面），注入即自报拦截行为。

**Blocked by:** 无（票 15/25 的 execstub/execrpc 底座已就位）　**Owner:** 邻居 agent（w12:p1，handoff 交接）

## 机制依据（票 24 源码级取证，字节偏移在 REPORT.md）

- stdio MCP spawn：cc 经 SDK StdioClientTransport **shell:false 直接 execvp 裸名 `npx`**，不经任何 shell → **L2 PATH 前置即可拦**（<sess>/bin/npx = shim）
- Bash 工具：$SHELL（L1）或硬编码 /bin/bash、/usr/bin/bash（附3 取证字面量）→ **L3 bind 拦**
- hooks/statusline/REPL !cmd：硬编码 `/bin/sh -c` → **L3 bind /bin/sh 拦**
- 结论：L1.5 的全部覆盖面可由 L3 + 既有 L2/L1 等价承接，且 env 零新增

## Specification

1. **删**：host 模式注入的 `CLAUDE_CODE_SHELL_PREFIX`（L1.5）——会话 env 中不得出现
2. **增 L3**：netns 内 mountns bind——`/bin/bash`、`/usr/bin/bash`、`/bin/sh` → `<sess>/bin/<name>`（multi-call shim 符号链接到 iso-cc；bind 源=会话 bin 目录，逐会话隔离；mountns 作用域，宿主零 diff）
3. **execstub 增 argv 直通模式**：argv0 basename ∉ {bash, sh}（如 npx）→ 不走 -c 解析，宿主侧按原 argv execvp（宿主 PATH 解析真 npx）——长生命周期语义与 24 E1/E2 已证的 SCM_RIGHTS 通道相同
4. **保留**：L1 `SHELL=<sess>/bin/bash`（自然形态，普通用户自定义 shell 常态）；L2 PATH 前置；ISO_CC_SESSION / ISO_CC_EXEC_SOCK env（无 hook 点名语义；可在后续票做 argv0 路径推导去 env 化，非本票范围）
5. **sandbox 模式**：不 bind、不前置 PATH、不动 SHELL——行为零变化（回归锚）

## Acceptance

- [ ] 会话 env 断言：`CLAUDE_CODE_SHELL_PREFIX` 不存在（host/sandbox 两模式）；`env` 输出无新增 hook 点名变量
- [ ] `/bin/bash` 在 netns 内 `readlink -f /proc/self/exe` 于 shim 调用时 = iso-cc（L3 生效取证）
- [ ] stdio MCP 全链不回归：真 @playwright/mcp 经 L2（裸名 npx）宿主侧落点 + 常驻多轮工具调用（24 E1 回放）
- [ ] Bash 工具（$SHELL 与硬编码 /bin/bash 双形态）、hooks、statusline 三面绿
- [ ] sandbox 模式回归：/bin/bash 为真 bash、PATH 无会话 bin 前置
- [ ] clippy -D warnings 绿；nextest 全绿

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾；ISO_CC_* env 的路径推导去 env 化留后续票
