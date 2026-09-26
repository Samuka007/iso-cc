# 11 — ns.rs 安全 API：双入口（mountns / selfmap），session.rs unsafe 归零

**What to build:** 按设计稿 §2 落地（正本 = `../design-session-lanes.md` §2.1/§2.2/§2.3/§8-11 行；更早的票面设想被 §2.3 取代）。

**Blocked by:** 09（已完成）　**Owner:** lane-ns-safe-api（完成）
**Status:** done（2026-09-26，parent 亲验通过）

## Answer（PM 落）

- grep 实测：session.rs unsafe = 0；src/ 全域 unsafe 仅 ns.rs（26 处 SAFETY 注释）
- nix 0.31.3（resolver 定版，--no-default-features --features sched,mount）
- parent 复跑：pasta `run -- true` rc=0；`exit 7`→7（pasta）、`exit 5`→5（slirp selfmap 路径）；nextest 25/25；clippy 干净；零残留
- 三处偏离（合理，SAFETY 锚明）：①`ns::install_pre_exec`——CommandExt::pre_exec 本体 unsafe，安装面收敛进 ns.rs 才能词法归零；②`write_self_ugid_map(uid,gid)` 带参——unshare 后 getuid() 呈 overflow uid 65534，首轮 strace 抓到 EPERM 后修正（与 09 parent 侧捕获值等价）；③`set_pdeathsig_verified` safe fn，expected_ppid=0 = 结构对账（getppid()==1 自尽）

## Specification

1. 新模块 `src/ns.rs`，双入口：
   - `enter_mountns(binds: &[(CString, CString)], expected_ppid: u32) -> io::Result<()>`（pasta spawn primary 路径：仅 CLONE_NEWNS + rprivate + bind_ro + PDEATHSIG 对账）
   - `enter_selfmap_ns(binds, expected_ppid)`（slirp 专用：CLONE_NEWUSER|NEWNS|NEWNET + pre_exec 内自写 setgroups/uid_map/gid_map 单条自映射，user_namespaces(7) 规则序）
   - 内部 unsafe fn 逐个具名（unshare_mountns / make_root_private / bind_ro / set_pdeathsig_verified / unshare_user_net_mountns / write_self_ugid_map），SAFETY 注释按设计稿 §2.2 草案落地
2. **CString 父进程预转换**，pre_exec 闭包只持转换结果（信号安全）
3. PDEATHSIG 对账：prctl 后 getppid 对账 expected_ppid，不符自尽（kill self）——竞态清单见 man PR_SET_PDEATHSIG
4. session.rs 的 pre_exec 闭包改为对 ns.rs 的单调用；session.rs 内 unsafe 归零

## Acceptance

- [ ] grep 证 session.rs 无 unsafe；全部 unsafe 收敛在 ns.rs 且每个 unsafe fn 有 SAFETY 注释
- [ ] 行为等价：nextest 全绿（探针断言不改）；`run -- true`（pasta 路径）+ slirp 路径端到端 rc=0；`exit 7` → 7 仍透传
- [ ] clippy -D warnings 绿
- [ ] 冒烟（设计稿 §8-11 行）：双路径各跑一次端到端

## 轮子盘点

crate 线 `nix::sched::unshare` / `nix::mount::mount`（或 rustix，cargo add 定版）；拒绝 unshare(1)/mount(8) CLI（execve 纪律 + pre_exec 装配序不可承载）；拒绝 `unshare` crate（0.7.0 @2021 死亡）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 12/13/14/15 范围；一次验证收尾
