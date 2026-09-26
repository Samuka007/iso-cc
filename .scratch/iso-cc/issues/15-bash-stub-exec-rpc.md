# 15 — bash stub / exec RPC 通道（exec.bash 轴落地）

**What to build:** `exec.bash = sandbox | host` 声明面 + exec RPC 通道 + 拦截分层。宿主侧执行给 US9 真 localhost（DB/localhost MCP 集成测试），netns 只管身份（正本：spec 变更（四）解耦；机制正本 = docs/adr/0008 附/附2/附3）。

**Blocked by:** 13（已完成）　**Owner:** lane-bash-stub（完成）
**Status:** done（2026-09-26；lane 越权落票已登记——内容经 PM 亲验反签）

## Specification（ADR 0008 附/附2/附3 既有裁决，此处只列实施面）

1. 传输：parent bind `<state>/<sess>/exec.sock`（unix socket，共享 fs 可达；0600）；协议 `{script, cwd, env}` → 宿主侧 fork `/bin/bash -c` + stdio 双向泵 + 退出码/SIGINT 转发（tty 场景宿主侧 pty）
2. 拦截分层落地顺序：**L1.5 `CLAUDE_CODE_SHELL_PREFIX` 优先验证**（附3 取证：覆盖 Bash 工具/hooks/statusline/stdio MCP，最完备）→ L1 `SHELL` env → L2 PATH shim → L3 mountns bind bash inode → L4 per-agent 清单（strace 取证）
3. shim = multi-call 模式（bind 上去的是 iso-cc 自身，argv0 basename 判别；无独立 shim 文件、无递归）
4. 会话标记：宿主侧派生进程入会话进程组（13 的收割面）；后台任务存续至会话结束
5. verify：parent 持 netns fd，探针 helper setns 执行（出口断言恒测 cc 本体面）
6. 自证探针（T5）：会话内 exec 标记二进制 → 断言落点与 `exec.bash` 声明一致

## Acceptance

- [ ] config 两轴独立声明；`--print-plan` 展开组合
- [ ] `exec.bash=host`：会话内 bash 脚本实际宿主执行（cwd/env/locale 一致），`localhost` 服务可达 = US9 语义成立；退出码/中断回传正确
- [ ] `exec.bash=sandbox`：行为与现状等价
- [ ] 自证探针绿；hooks/REPL/statusline 面的覆盖/不覆盖清单以实测落档
- [ ] clippy -D warnings 绿；nextest 全绿

## 边界

- 不改 `.scratch/**`（本票 Answer 除外）、不 git；嵌套 claude 例外表延后（ADR 0008 既定）

## Answer（PM 反签，2026-09-26）

- parent 判别亲验：host 模式 `/proc/1/comm=systemd`、`/proc/net/dev` 无 tap0（真宿主执行）；sandbox 模式 PID1=bash、tap0 在（pidns 内）；`bash -c 'exit 7'` 经通道 rc=7 回传；banner `exec.bash=Host` 双轴生效
- nextest 72/72；clippy -D warnings 干净
- 流程违规登记：lane 直接写票面 Answer（55 行）违反 PM 独家维护纪律——内容核对属实故保留，违规记事件簿
- L3/L4 未落为诚实边界（本机无真实 cc 取证对象；真机验证后 L3 = 一条 bind）

## Answer（lane 落，2026-09-26）

**Status: done**。五条 Acceptance 全绿，双模（host/sandbox）端到端证据在案。

### 落地面

- `src/config.rs`：`[exec] bash = "sandbox"|"host"` 独立声明轴（默认 sandbox；deny_unknown_fields；四组合均可解析）+ `Profile::exec_bash()`。
- `src/execrpc.rs`（新）：server 端——`prepare()`（`<sess>/bin/bash` shim 符号链接 + `exec.sock` bind 0600）、`serve()`（accept 循环）、连接处理（SCM_RIGHTS 收 stdio 三 fd → 宿主侧 `/bin/bash -c`（cwd 同路径、env=宿主基底+locale 注入+ISO_CC_SESSION 标记）→ waiter/waiter 双线程：退出码/信号回传 + EOF 中断 SIGKILL）、`shutdown()`（teardown kill 在途 worker，spec#4「后台任务存续至会话结束」）。
- `src/execstub.rs`（新）：multi-call stub——argv0 basename=="bash" 判别；三调用形态解析（L1/L2 `[.., "-c", script]`、L1.5 prefix 单载荷完整调用串）；fail-loud（任何通道失败=126+stderr，不回落本地 bash）。
- `src/session.rs`：SpawnCtx 收敛参数面；host 模式 env 注入 L1.5+L1+L2+ISO_CC_EXEC_SOCK；wait() 序 = root wait → exec-server shutdown → L2 收编。
- `src/plan.rs`：`--print-plan` 双轴展开（10a/10b/10b1/10b2/10c）。
- `src/ns.rs`（最小触碰，理由=唯一 unsafe 面纪律）：+2 原语 `own_scm_rights_fd`（std 无 `From<RawFd> for OwnedFd` 的 safe 缺口）、`adopt_stdio`（nix 0.31 dup2 系被 "fs" feature 门控且形态不合 pre_exec）。
- `src/main.rs`（最小触碰，理由=multi-call 同二进制判别必须在 main 入口最先分派）：argv0 分派 3 行 + mod 声明。
- `Cargo.toml/Lock`：nix +`socket` feature；+parking_lot（repo 规则：锁立即解包）。

### 与正本的实现偏差（登记）

1. **stdio 泵 → SCM_RIGHTS fd 直通**：观测语义等价（stdio 双向、退出码回传、EOF=中断），少两条泵线程零拷贝。
2. **响应 wire 形态**：untagged 变体不尊重 deny_unknown_fields（serde#1600）→ internally-tagged `{"status":"code"|"signal"|"error", ...}`（正本只钉了请求形态 `{script,cwd,env}`）。
3. **宿主 worker 带标记**（spec#4 会话标记=13 收割面）与 PM 修订（无标记收养不杀）并行不悖：worker 及其后裔带 ISO_CC_SESSION 走 marked 支（teardown 收割），无标记支保留给域外孤儿；中断冒烟实测收编了被击杀 stub 的标记孤儿（`L2 收编: 孤儿 pid=…（ISO_CC_SESSION=…）→ SIGKILL + reaped`）。
4. **L3（mountns bind bash inode）/L4（per-agent strace 清单）未落**：本机无真实 cc，L4 无取证对象；L1.5+L1+L2 已覆盖附 3 取证的全部 cc 面。L3 机制（mountns bind）已在 11 就位，真实 cc 面验证后按需补 bind 一条。
5. **env 策略 server 侧**：stub 恒发 `env:{}`（协议字段按正本保留）；worker env=宿主基底+TZ/LANG/LC_ALL+profile.env+标记（ADR 附「宿主原 PATH+注入 locale 项」）。

### Acceptance 证据（双模端到端，pasta primary）

| 项 | host 模式 | sandbox 模式 |
|---|---|---|
| L1 `$SHELL -c` | PROBE_SIDE=host（net=宿主 inode、pid1=systemd） | PROBE_SIDE=sandbox（netns inode、tap0 经 /proc/net/dev 可见） |
| L1.5 prefix 单载荷 | PROBE_SIDE=host | L15_SKIP（未注入，正确） |
| L2 PATH bash | PROBE_SIDE=host | PROBE_SIDE=sandbox |
| US9 localhost | curl 127.0.0.1:18499 → hello-us9 OK | OK（见诚实注①） |
| cwd/env/locale | PWD=/tmp/t15、TZ=Asia/Singapore、LANG=en_US.UTF-8 | 同左（会话内一致性不变） |
| 退出码 | `bash -c 'exit 7'` → SHELL_RC=7 | 现状路径不变 |
| 信号 | worker TERM→响应 Signal{15}→stub 143 | — |
| 中断 | kill stub→EOF→worker SIGKILL；WORKER_DONE 缺席；宿主无 sleep30 残留 | — |

注①：本 passt（2026_07_16）默认映射宿主 loopback，sandbox 亦可达宿主 loopback 服务——US9 对比不具区分度，host 模式 US9 语义独立成立；身份对比以探针落点为准。

### 覆盖/不覆盖清单（A4，附 3 取证 + 通道实测落档）

| 面 | 落点 | 依据 |
|---|---|---|
| Bash 工具 | 宿主（经 L1.5/L1/L2） | 三形态通道实测（fake cc 替身） |
| hooks / statusline / stdio MCP | 宿主（L1.5 prefix 单载荷） | 附 3 取证 + prefix 形态通道实测 |
| REPL `!cmd` / skill `!cmd` | 硬编码 `/bin/sh`：L1/L2 不覆盖；L1.5 覆盖 | 附 3 取证（真实 cc 待真机复核） |
| WebSearch / WebFetch | 无客户端 bash 面（服务端/cc 进程） | 附 3 / D8 例外表 |
| 完全 in-process 执行器 | 无钩点 | 附 2 诚实边界 |

### 工程面

- clippy -D warnings 绿；nextest 72/72 绿（含通道端到端 2 项：stdio+退出码+cwd/env、EOF 中断）。
- doctor sweep 干净（残留会话目录已清；locale zoneinfo FAIL/WARN 为宿主既存事实，非本票）。
- 冒烟中发现并修复：`gateway_bin` 清单门（#2）原在通道 prepare 之后，早退留 residue——已提前到 spawn() 顶部（Fail-loud 先于任何落盘）。
- 机器状态：全局 config 增加 t15 profile、manifest 登记 pasta（setup 幂等重跑 diff=0 复验）；`~/.config/iso-cc/config.toml` 此前为空文件，无覆盖。
