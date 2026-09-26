# 12 — 会话路径零外部命令：netlink 直配 + /proc/sys 直写 + fail-loud bootstrap

**What to build:** 会话/引导路径废除对 `ip`（iproute2）与 `sysctl` 二进制的依赖，改为进程内原语；废除 `sh()` 吞错模式。评审定性（用户 0927-1）：脚本思维——用 sh+字符串参数编排网络，发行版无 iproute2 即哑掉（D5 的 distro-hopping/静态 musl 目标形态不允许）；且 `sh()` 不查退出码，配置失败静默继续到探针才红，违反 fail-loud。

**Blocked by:** 09 A1（session.rs 串行）；参考 10 结论（若 pasta `--config-net` 成立，pasta provider 的 guest 侧配网整体消失，netlink 仅服务 slirp provider 与等待逻辑）
**Owner:** 待派（session.rs lane 序贯，在 11 之后）

## Specification

1. netlink 配网（`rtnetlink`/`netlink-packet-route`，版本 `cargo add` resolver 决定，禁手写；同步封装——bootstrap 是同步上下文）：
   - `link_up(iface)`、`addr_add(iface, cidr)`、`default_route(via, dev)` 替代三条 `ip` 命令
   - tap0 就绪等待（现 `ip link show` 轮询）改 netlink dump/事件
2. `/proc/sys/net/ipv6/conf/{all,default}/disable_ipv6` 直写替代 `sysctl -w`（含写失败 fail-loud）
3. bootstrap 全链路错误传播：任一步骤失败 → 带步骤名/接口/`io::Error` 上下文 bail；禁止 sleep-继续、禁止退出码不查
4. provider trait 接口下：`tap_plan` 生成"等待+配置"计划，pasta（`--config-net`）= 仅等待，slirp = 等待+netlink 配置

## Acceptance

- [ ] 会话/引导路径 execve 集合不含 `ip`/`sysctl`（验证：`strace -f -e trace=execve` 冒烟输出为证）
- [ ] PATH 无 iproute2 的最小环境冒烟通过（`env PATH=/usr/bin:/bin` 或容器最小 shell）
- [ ] 错误路径 fail-loud：单测覆盖 netlink 失败 → bootstrap 带上下文报错退出（不许静默）
- [ ] clippy -D warnings 绿；nextest 全绿；探针行为不回归

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git commit；11 未合入前不动 session.rs
