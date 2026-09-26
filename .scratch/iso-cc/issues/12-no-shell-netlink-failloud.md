# 12 — 零外部命令：netlink 就绪等待 + /proc/sys 直写 + fail-loud bootstrap

**What to build:** 按设计稿 §3 落地（正本 = `../design-session-lanes.md` §3.1/§3.2/§5/§8-12 行；本票早期设想已被 §3.1 前提修正取代）。

**Blocked by:** 11（已完成）　**Owner:** 待派

## Specification（前提修正后）

1. **配网命令已消失**（两 provider self-config：pasta `--config-net`；slirp `-c`）——bootstrap 不再有 ip addr/route 调用；本票删除残留的 `sh`/`sh_ok`（tap 轮询）并整体废除吞错模式
2. `netcfg.rs`（新，无 unsafe，同步）：
   - `wait_ready(iface, timeout) -> io::Result<()>`：**netlink dump**（tap 存在 + UP + 默认路由）fail-loud 超时（文案含 gateway.log 指针，修正 audit-facts §5 反例）。**09 取证：ns 内 /sys/class/net 呈宿主视图（sysfs netns-tag 伪影）——必须 netlink，勿用 sysfs**
   - `disable_ipv6()`：`/proc/sys/net/ipv6/conf/{all,default}/disable_ipv6` 直写（bootstrap 期消费 plan.ipv6_off），fail-loud
   - `host_iface_up(name)`：若 09 已实现则复用/迁移，不重复
3. crate：netlink-sys 0.9 + netlink-packet-route 0.33（MIT，同步栈）或 neli 0.7.4（BSD）——cargo add resolver 定版
4. 错误传播：任何 bootstrap 步骤失败 = 步骤名/接口/io::Error 上下文 bail

## Acceptance（= 设计稿 §8-12 冒烟）

- [ ] `strace -f -e trace=execve` 全会话无 `ip`/`sysctl` execve（输出留 /tmp）
- [ ] PATH 最小化冒烟：`env PATH=/usr/bin:/bin` 下双 provider 端到端 rc=0
- [ ] 单测：wait_ready 喂超时 → 带上下文失败；disable_ipv6 写失败路径 fail-loud
- [ ] `curl -6` 会话内必败（P2 复核，设计稿 §9.4 时序确认）；探针 8 项 6/1/1 不回归
- [ ] clippy -D warnings 绿；nextest 全绿；`sh`/`sh_ok` 从 session.rs 消失

## 轮子盘点

netlink 原语 = 现成 crate（CompScan #6/#7：netlink-sys/netlink-packet-route/neli 全活跃）；无自研 netlink 解析。iproute2 不再被调用。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 13/14/15 范围；一次验证收尾
