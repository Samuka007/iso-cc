# 11 — 会话 ns 安装收敛为具名安全 API（去脚本式 unsafe 内联）

**What to build:** `session.rs` pre_exec 里 50+ 行 unsafe 闭包（unshare/mount/bind/remount/pdeathsig + `ck!` 宏内联）重构为最小 unsafe 面 + 具名安全 API。评审定性（用户 0927-1）：脚本思维——一段过程式 sh 脚本被直译成 Rust，unsafe 边界、错误处理、步骤语义全部糊在一个闭包里。

**Blocked by:** 09 A1（同文件手术，串行）　**Owner:** 待派（session.rs lane 序贯）

## Specification

1. 新模块 `src/ns.rs`：`pub fn enter_session_ns(binds: &[(PathBuf, PathBuf)], ipv6_off: bool) -> io::Result<()>` 为唯一对外入口；session.rs 的 pre_exec 只调它，不含任何 unsafe
2. 内部拆小、单一职责、各带 SAFETY 注释（不变量：仅 fork 后单线程 pre_exec 期调用；错误 = `last_os_error` 传播）：
   - `unshare_session_ns()` — CLONE_NEWUSER|NEWNS|NEWNET
   - `make_root_private()` — `/` MS_REC|MS_PRIVATE
   - `bind_ro(src, dst)` — bind + remount RDONLY（含 CString 转换错误归一为 io::Error）
   - `set_pdeathsig(SIGKILL)`
   - ipv6 关闭：随 12 落地 /proc/sys 直写后归入 12 的模块（本票先保持调用点收敛）
3. 禁止"把 unsafe 搬个位置"：每个 unsafe fn 的前置条件必须写成注释并被入口函数保证

## Acceptance

- [ ] session.rs 内 unsafe 块归零；全部收敛在 ns.rs 且每个 unsafe fn 有 SAFETY 注释
- [ ] 行为等价：现有探针套件与单测不改断言照绿（P6/P6c bind 路径不变）
- [ ] `cargo clippy --all-targets -- -D warnings` 绿；`cargo nextest run` 全绿

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git commit；不趁便做 12 的 netlink 改造（串行票）
