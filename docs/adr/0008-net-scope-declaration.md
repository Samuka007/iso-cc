# D8: 网络隔离范围声明 net.scope = tree | self

状态：提议（2026-09-26，待定稿——self 是否入 v1 未决）

## 背景

R1 原文把隔离对象定义为整棵进程树（一致性范围：环境表现得像一台 B 区机器）。用户指出：平台风控面是 cc **本体**的平台面流量（API/OAuth/遥测——风控对象是账号），bash 子进程的流量对端是开发基础设施（npm/git/curl），不做账号风控，出口位置无关紧要。因此存在两个合法的隔离范围。

## 决策

`net.scope` 成为 profile 声明：

- `tree`（默认）：R1 原文。整棵树进 netns，全协议覆盖，一致性范围。
- `self`：cc 本体进 netns（fork 继承，单向往墙保证子进程不可逃逸）；Bash 工具的 `bash -c <整段脚本>` 经 unix-socket RPC 转发到宿主 netns 执行（复用 host_exec 通道，P3 前置为该范围的传输层）。

## self 的语义细节

- 拦截粒度 = `bash -c` 的整段 argv（cc Bash 工具即整段调用），不做单命令 DEBUG-trap（复合语句会碎）
- cwd：共享 fs 同路径，零映射
- env：宿主侧执行时注入同一套 locale env，输出一致
- **例外表**：cc 二进制不透传（嵌套 `claude` 留在沙箱，防宿主账号/出口污染——R4 精神）
- DNS/本地镜像：bash 流量走宿主，本地镜像可用（更快），一致性观测面让位

## 代价与边界（诚实披露）

- 一致性范围下的保证（bash 侧任何 egress 观测返回 B）在 self 下不存在；verify 探针 P1/P13–P15 在 self 下语义收窄为"cc 本体出口"（P1 加 self 语义），裁决语义（环境与声明一致）不变
- 透传通道 = 同 uid 任意宿主代码执行面（与 host_exec 同级），必须显式声明
- 复合命令/后台任务/交互 TUI 场景的转发保真度需实测定界（P3/实现期裁决）

## 已拒绝

把 self 说成"等价于放弃隔离"（范围混淆）；DEBUG-trap 单命令转发（语义碎裂）；对 cc 二进制也无差别透传（R4 污染）。

## 附：self 转发的实现机制（2026-09-26 定稿）

self 范围下无 tun、无 netstack——转发是**流级 RPC**，与 tree 范围的包级网关（netstack-smoltcp/SOCKS provider）分属两层，互不依赖。

链路：parent 留宿主 netns，bind `<state>/<sess>/exec.sock`（unix socket，共享 fs）；fork+unshare 子树 exec claude，env 注入 `SHELL=<wrapper>` 与 `ISO_CC_EXEC_SOCK`；cc Bash 工具 spawn `$SHELL -c <整段脚本>` → wrapper 连 socket 发 {script, cwd, env} → parent fork 宿主侧 `/bin/bash -c`（cwd 同路径零映射，env = 宿主原 PATH + 注入 locale 项，进程组 = 会话组）→ stdio 双向泵 + 退出码回传。

决策点：①拦截点 = `SHELL` env（PATH shim 兜底），不动 /bin/bash；②粒度 = `bash -c` 整段 argv；③宿主侧派生进程入会话进程组，teardown 收割（N3）；④后台任务（run_in_background）挂会话组存续至会话结束；⑤SIGINT 经 RPC 转发，tty 场景宿主侧 pty；⑥verify：parent 持 netns fd，fork 探针 helper setns 进 netns 执行（P1/P13–P15 语义收窄为 cc 本体出口，全部可测）；⑦`exec.sock` = 同 uid 任意宿主执行面（self 的声明性代价，0600 + state 目录）。

复杂度：wrapper + server ≈ 300–400 行。CC 二进制例外（嵌套 claude）延后（用户裁决：edge case 留未来）。

## 附 2：拦截分层（2026-09-26）

转发钩子的健壮性分层（mountns 使 bind 覆盖可作用于沙箱内任意绝对路径而不碰宿主——OverlayFS 思想的单文件简化版）：

| 层 | 机制 | 覆盖 | 成本 |
|---|---|---|---|
| L1 | `SHELL` env → wrapper | 尊重 SHELL 的 agent（cc 实测主路径：`process.env.SHELL` + fallback `SHELL\|\|COMSPEC\|\|...`） | 零（本就是 env 注入） |
| L2 | PATH shim 目录 | 经 PATH 解析 `bash` 的 agent | 一个目录 |
| L3 | mountns bind 覆盖 bash inode（`/usr/bin/bash`，`/bin/bash` 符号链接随之覆盖） | **硬编码绝对路径**的 agent | 一条 bind |
| L4 | per-agent 重定向清单 | vendored 私有 shell（如 omp 自spawn 的 shell 二进制）：一次性 `strace -f -e execve` 取证 exec 路径 → 声明式 bind 扩列（R4 同机制） | 每agent 一次性取证 |

shim 实现采用 **multi-call 模式**：bind 上去的就是 iso-cc 自身（argv0 basename == `bash` → 转发模式），无独立 shim 文件、无递归（转发执行发生在宿主侧 mountns 外）。

**不可拦截边界（诚实披露）**：完全 in-process 的 shell 执行器（不 exec 任何 shell 二进制）无钩点——但编码 agent 不能 exec 程序即不成立；vendored shell 路径随版本漂移 → L4 清单按版本更新（与 R4 写集探针同一哲学：声明由观测验证）。自证覆盖探针（T5）：会话内 exec 标记二进制 → 断言落点为宿主侧。

## 附 3：二进制取证修正（claude-code 2.1.263，2026-09-26）

cc 实测主路径确认为 L1（读 `process.env.SHELL`，fallback `SHELL||COMSPEC||...`，二进制含 `/bin/bash`、`/usr/bin/bash` 字面量兜底 → L3 补齐）。**但**：Hooks、REPL `!cmd`、skill `!cmd`、statusline 全部硬编码 `/bin/sh -c`，不走 `$SHELL`——L1/L2/L3（针对 bash 的）不覆盖这些面，self 范围下它们留 netns 直跑。官方 `CLAUDE_CODE_SHELL_PREFIX` env（实测覆盖 Bash 工具/hooks/statusline/stdio MCP）是更完备的拦截层候选——登记为 L1.5，实现时优先验证。D8 例外表需显式声明：hooks/statusline/REPL 面 + WebSearch（服务端执行，无客户端网络）+ WebFetch（cc 进程 fetch，拒绝 localhost + api.anthropic.com preflight）。详见 ../iso-cc-research/2026-09-26-cc-tool-surface.md。

## 附 4：L1.5 注入取消，L3 bind 升为主拦截（2026-09-28）

操作者裁定：显式注入 `CLAUDE_CODE_SHELL_PREFIX` 等于向 cc 自陈 hook 行为（env 是公开面，cc 与任意子进程可读）。票 24 源码级取证同时证实：stdio MCP spawn 为 **shell:false 直接 execvp 裸名 `npx`**，不经任何 shell——L2 PATH 前置即可拦截；Bash 工具/hooks/statusline 的硬编码路径由 L3 mountns bind（/bin/bash、/usr/bin/bash、/bin/sh → 会话 shim）拦截。

处置：host 模式取消 L1.5 注入；L1.5 降级为 L3 不可用环境（无 mountns bind 权限面）的备选。保留 env：`SHELL`（指向会话 bin，自然形态）、`PATH` 前置（自然形态）、`ISO_CC_SESSION`/`ISO_CC_EXEC_SOCK`（无 hook 点名语义；路径推导去 env 化列后续票）。详见票 26。
