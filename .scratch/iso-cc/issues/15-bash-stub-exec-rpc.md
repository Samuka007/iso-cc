# 15 — bash stub / exec RPC 通道（exec.bash 轴落地）

**What to build:** `exec.bash = sandbox | host` 声明面 + exec RPC 通道 + 拦截分层。宿主侧执行给 US9 真 localhost（DB/localhost MCP 集成测试），netns 只管身份（正本：spec 变更（四）解耦；机制正本 = docs/adr/0008 附/附2/附3）。

**Blocked by:** 13（宿主侧派生进程依赖其进程组收割/三层防线）　**Owner:** 待派

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
