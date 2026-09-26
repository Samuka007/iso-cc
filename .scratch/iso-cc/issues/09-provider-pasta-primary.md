# 09 — 网关 provider 化：pasta spawn 转正 + slirp 回退 + fail-loud egress

**What to build:** 按设计稿 `../design-session-lanes.md` §1/§1.3/§1.4/§5/§8-09 行实施：pasta **spawn 模式**为 primary（pasta 作会话根），slirp4netns 为 provider#2（selfmap 入口 + `-c`），egress 接口 fail-loud（R1/R8）。

**Blocked by:** 无（设计稿已落盘）　**Blocks:** 11
**Owner:** lane-provider-trait（完成）
**Status:** done（2026-09-26，parent 亲验通过）

## Answer（PM 落）

六锚全绿 + parent 复跑确认：
- `run -- true` rc=0，banner `root=pasta(pid=…)` = spawn 模式生效；`exit 7` → rc=7（透传成立，plan B 不需要）
- `--print-plan`：pasta 全实参 + `-I tap0` + egress assert #3/#12 断言行
- `if:lo` 拒绝（#12 文案）；`if:nonexistent0` 拒绝（"R8：绝不回落"）；探针 8 项 6/1/1 与基线一致（P13 按实红）；会后零残留进程
- nextest 25/25；clippy -D warnings 干净

**给后续 lane 的落地取证（lane 回报，入 12/13 票面）**：
1. pasta re-exec 成 passt.avx2 且 /proc/<pid>/environ EACCES → 13 sweep 用 netns-inode/argv 键，勿依赖 pasta environ；cc 树标记经 bootstrap 自设可靠
2. pasta 对 `--outbound-if4 nonexistent0` 容忍不报错 → #3 sysfs spawn 前断言必要（已实现）
3. #4 判别键 = bootstrapped 标记文件（exec 前落），非 status
4. gateway.log 经 `-l` 收 pasta 诊断；stdio 全 inherit
5. **会话内 /sys/class/net 呈宿主视图（sysfs mount netns-tag 伪影）→ 12 ns 内就绪等待用 netlink，勿依赖 sysfs**

## Specification（以设计稿为唯一细则，本票只列验收锚）

1. spawn 模式：`pasta -f -q --config-net --outbound-if4 <if> -I tap0 --dns-forward <dns> --no-ndp --no-dhcpv6 --no-ra -- <bootstrap argv>`；会话根 = pasta；bootstrap pre_exec 挂 PDEATHSIG→pasta + getppid 对账（设计稿 §1.1/§1.3）
2. slirp 回退：bootstrap `--mode selfmap`（pre_exec 三 ns + 自写单条 maps）+ `slirp4netns <pid> tap0 -c` attach；parent **永不**写 /proc/<pid>/maps（§1.3 下树）
3. bootstrap 新参面：`session-bootstrap --plan <json> --session-id <id> -- <cmd…>`，plan = {mode, ipv6_off, binds, iface, timeout_ms}，serde + deny_unknown_fields（§1.4）
4. config 面：`net.gateway = pasta|slirp4netns`（默认 pasta）+ `net.dns`；fail-loud 解析
5. 会话资产移 `sessions/<id>/`（并发互踩修复，audit-facts §7）；双标记 env（pasta 与 bootstrap 各自标 ISO_CC_SESSION）；list.rs:11 注释错位修正
6. fail-loud 点位（设计稿 §5 表 09 行）：#1 config 解析、#2 provider 过渡态（which + 显式报错）、#3 宿主接口存在/UP（sysfs，spawn 前断言）、#4 pasta 早死（1s try_wait + gateway.log tail）、#5 plan 解析、#9 exec、#11 slirp DNS 恒 Warn、#12 `-I` 撞名断言（egress=lo 类输入直接拒绝）

## Acceptance（= 设计稿 §8-09 冒烟全表）

- [ ] `cargo nextest run` 全绿 + `cargo clippy --all-targets -- -D warnings` 零告警
- [ ] `iso-cc run --print-plan` 展开 pasta 全实参（含 `-I`）+ bind 清单
- [ ] `iso-cc run -- true` 端到端（spawn 模式）
- [ ] **退出码透传取证**：`pasta --config-net -q -- sh -c 'exit 7'; echo $?`（首个冒烟项；不透传则按设计稿 §9.1 plan B 上报，不自行预建）
- [ ] `egress="if:lo"` 拒绝（#12）；`egress="if:nonexistent0"` 拒绝（#3）
- [ ] 探针套件不回归（本机 verify 行为不变，P13 按实红）

## 轮子盘点（正本（三）要求）

pasta/passt 2026_07_16（spawn 原语 E1/E2/E5）；slirp4netns 1.3.5 `-c`（main.c:149-227）；无 crate 替代编排壳（component-scan #0 表）。零网络自研。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git commit；一次验证收尾（中途不跑 cargo）
