# 13 — 生命周期三层防线 + 对账 sweep（residue 清零）

**What to build:** 按设计稿 §4 + PM 修订（design-session-lanes.md 头部修订注）落地；闭合 audit-code-facts §6 五缺口。

**Blocked by:** 12（已完成）　**Owner:** lane-lifecycle-kill（完成）
**Status:** done（2026-09-26，parent 亲验通过）

## Answer（PM 落）

- **L1 亲验**：`kill -9 <pasta>` → run rc=1、passt 进程清零、list 无存活
- **L2 亲验**：收编循环日志实测（无标记收养孤儿 pid=1867684 →「登记不杀」按 PM 修订双键定界）；lane L2b slirp 路径收编 SIGKILL+reaped 证据在案；**机制发现**：pasta spawn 建 pidns（cc=ns pid 1），ns-init 死 → 内核整树 SIGKILL（收敛强于设计假设，收编循环实际生效面 = slirp 回退路径 + 防御纵深）
- **L3 亲验**：指纹假孤儿（bash 复合命令保 argv）→ `[FAIL] sweep/orphan-gateways 孤儿网关 ×1 kind=pasta session=? cmd=…`；判定核 sweep_from 纯函数可单测（+9 测试，nextest 36/36）
- **pasta 可读键再收窄**：/proc/<pid>/ns/ 亦 EACCES（file caps→dumpable=0）→ argv 是唯一键（--outbound-if4 / argv0 基名 / 尾部 --session-id）
- **纪律偏差（登记）**：3 个生命周期原语 unsafe（prctl/kill/waitpid）落在 session.rs（契约文件集未含 ns.rs/Cargo.toml）→ **归 14 票迁移回 ns.rs**，恢复 11 的归零不变量
- 附：顺带清理 54 个历史冒烟遗留会话目录（卫生，非产品代码）

## Specification

1. **L1（已在 11 落地，本票补证据）**：PDEATHSIG 对账（getppid 结构对账/精确对账）——kill 链冒烟证明：`kill -9 <pasta>` → 会话树（bootstrap+cc）死
2. **L2**：iso-cc parent `PR_SET_CHILD_SUBREAPER` + 收编循环；**收编范围 = 会话根后裔 ∩ ISO_CC_SESSION 标记（双键定界，PM 修订）**；无标记收养子女只登记上报 doctor、不杀（票 15 host 执行语义预留）；spawn 后 `?` 错误路径 KillGuard（child 已 spawn 的泄漏面，audit-facts §3）
3. **L3**：sweep 枚举器（list.rs）+ doctor sweep 检查组；**枚举键**（09 取证硬约束）：pasta 本体 environ EACCES（re-exec passt.avx2 加固）→ 用 netns-inode / argv 指纹（`--outbound-if4`）识别网关；cc 树用 ISO_CC_SESSION env（bootstrap 自设，可靠）
4. **标记面**：pasta 自身 env 标记不可行（environ 加固）→ list/doctor 对 root=pasta 的会话以 argv+netns 键判定；注释错位修正（list.rs:11）
5. plan.rs 陈旧文案一行修正（"direct-write lands with issue 12" 已落地）

## Acceptance（= 设计稿 §8-13 冒烟 + audit-facts §6 全闭合）

- [ ] `kill -9 <pasta>` → 会话树死（L1 证据输出）
- [ ] bootstrap 内 `setsid sleep 300 &` 后正常退出 → 进程消失（L2 收编证据）
- [ ] 手工伪造孤儿 pasta（无标记）→ doctor Fail 上报（L3；gc 清除动作归 14）
- [ ] audit-facts §6 五缺口逐条闭合对照表；residue 可枚举
- [ ] clippy -D warnings 绿；nextest 全绿；探针 6/1/1 不回归

## 轮子盘点

subreaper/killpg 均为内核原语（PR_SET_CHILD_SUBREAPER、killpg(2)）；拒绝 systemd-run --scope（D1：systemd 依赖违反 R11；transient 澄清见 CompScan #5）；无第三方进程看护 crate（保持薄）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 14/15 范围（gc 命令、exec RPC）；一次验证收尾
