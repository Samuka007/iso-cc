# iso-cc 集成设计稿：网络会话（Phase I：09 → 11 → 12 → 13 → 14）

正本：spec.md §变更记录（一）两级资源模型／（二）statelessness 审计／（三）实现手段开放性；`research/alternatives-scan.md`（下称 AltScan，证据 E1–E6）；`research/component-scan.md`（下称 CompScan，行号裁决 #1–#10）；`issues/10 §Answer`（attach 根因 + `-I` 硬规则 + proven invocation）；`issues/README.md` Phase D 裁决 + Phase I 串行序；`audit-code-facts.md`（下称 Facts，行号证据基线 = HEAD 3a528f5 + session.rs 295 行）。本稿只引用上述正本证据；每个裁决带被否替代。

> **修订（PM，2026-09-26）**：§4-L2 收编范围定界为「会话根后裔 ∩ ISO_CC_SESSION 标记」双键；无标记收养子女只登记上报 doctor、不杀——为票 15 的 host 执行语义（在途短命 exec worker）预留。E4（down 接口 socket 绑定 fail-closed）证据锚在 research/alternatives-scan.md:31，不单列本稿。

---

## 0. 总裁决（承 Phase D，设计定形）

**薄编排成熟 CLI（AltScan §8）+ provider 模式 = pasta spawn 转正。** 自研边界收缩为（AltScan §8 修订建议 + CompScan #0 表）：config/doctor/verify、就绪等待、进程树终止三层、清单/GC、locale 注入、mountns bind 薄层。usernet 面与 uid_map 面视作外部依赖（pasta 内建 userns 映射，E2）。

---

## 1. provider 模式裁决：spawn 转正（pasta 为会话根）

### 1.1 裁决与理由

**选 spawn 模式**：`pasta -f -q --config-net --outbound-if4 <if> -I tap0 --dns-forward <dns> --no-ndp --no-dhcpv6 --no-ra -- <bootstrap argv>`。

1. **R8 由内核结构性执行**：spawn 模式下 cc 的父进程 = pasta。bootstrap 在 pre_exec 挂 `PDEATHSIG(SIGKILL)`（对账 pasta pid）→ **网关任何形态的死亡 = 内核即时击杀会话**。attach 模式下 pasta 死则 cc 成 ns 内孤儿（Facts §6「网关中途死无感知（SYN 黑洞静默）」），必须另建监视线程且仍有窗口。这是两模式唯一的结构性差异，裁决性权重最高。
2. **uid_map 竞态整类消灭**：pasta spawn 自建 userns 并映射当前用户（E2：ns 内 `uid=0(root)`、`=ep` 全能力）→ parent 侧 `/proc/<pid>/{setgroups,uid_map,gid_map}` 写（Facts §3 的错误路径孤儿 + 竞态面）在 primary 路径不复存在。
3. **unsafe 面收缩**（对 11 的量化，见 §2.3）：primary 路径 pre_exec 只剩 mountns + pdeathsig；`CLONE_NEWUSER|NEWNET` 建立与映射面移出我方代码（「usernet 面与 uid_map 面视作外部依赖」，AltScan §8）。
4. **D1 天然成立**：会话根 = pasta 进程 = 纯进程生命周期（passt.1:699-702「目标 netns 引用删除时 pasta 自行退出」，CompScan #3 取证）；134ms 冷启（E5）在 N2 预算内。
5. **E1/E3 已证全链可行**：spawn 管线 + ns 内 mountns bind 纯 CLI 组合跑通（E3），zero 自研网络代码。

### 1.2 被否替代

- ~~attach 模式转正~~（proven invocation，issue 10 §Answer）：唯一优势是与 slirp 回退共骨架；代价 = uid_map 写竞态保留（Facts §3）、unsafe 网络段保留（Facts §2 `:119` 三 flag unshare）、R8 需监视线程补丁。其 proven invocation 转为 **slirp 回退路径同构参考**与 09 的对照冒烟。
- ~~双模式并存（config 可选）~~：两套编排形状 = 两倍生命周期证明面，违反 boring；provider 差异（ns 建立归属）收敛到 bootstrap 的一个 mode 位（§5）。
- ~~slirp 保持现状 attach 且 parent 写 maps~~：Facts §3 已裁决该写点为 residue 源；slirp 路径改 bootstrap 自映射（§2.2 entry B），parent 永不写 /proc/<pid>/maps。
- ~~podman/rootlesskit/bwrap 作载体~~：AltScan §4 对比表无一全绿、§8 被否替代段（copy-up 语义/state-dir 模型/subuid 前提）。

### 1.3 spawn 模式进程树

```
iso-cc run（宿主侧编排/监督；PR_SET_CHILD_SUBREAPER = L2 执行者）
└── pasta -f -q --config-net --outbound-if4 <egress> -I tap0 --dns-forward <dns>
    │   --no-ndp --no-dhcpv6 --no-ra -- …            [pre_exec 注入 PDEATHSIG→iso-cc + getppid 对账]
    │   env: ISO_CC_SESSION=<id>（Command::env；list/sweep 对 pasta 的识别键）
    └── iso-cc session-bootstrap --plan <json> --session-id <id> -- <cmd…>
        │   pre_exec: unshare(CLONE_NEWNS) + rprivate + bind_ro×N + PDEATHSIG→pasta + getppid 对账
        │   自设 env ISO_CC_SESSION（list/sweep 对 cc 的识别键）
        │   /proc/sys v6 直写（ipv6_off）→ netcfg::wait_ready(tap0) → exec cc…
        └── cc 及其派生 daemon/孙进程（L2 subreaper 收编 / L3 sweep 兜底）
```

slirp 回退树（同一 bootstrap，mode 位不同）：

```
iso-cc run（监督 + subreaper）
├── iso-cc session-bootstrap --mode selfmap --plan <json> --session-id <id> -- <cmd…>
│     pre_exec: unshare(CLONE_NEWUSER|NEWNS|NEWNET) + 自写 setgroups/uid_map/gid_map（单条自映射，
│     user_namespaces(7) 规则内）+ rprivate + binds + PDEATHSIG→iso-cc
└── slirp4netns <bootstrap-pid> tap0 -c     [attach；-c 自配 lo/tap/MTU/IP/默认路由，main.c:149-227]
    env: ISO_CC_SESSION=<id>
```

### 1.4 bootstrap 子命令参数面（替代现 `--egress-iface`，Facts §5 `:202` 零消费）

```
iso-cc session-bootstrap --plan <json> --session-id <id> [--] <cmd…>
plan := {"mode":"mountns"|"selfmap", "ipv6_off":bool, "binds":[[src,dst]…], "iface":"tap0", "timeout_ms":15000}
```

- 同二进制 serde 往返，`deny_unknown_fields` fail-loud；argv 传递 = 子进程交接零状态（无临时文件）。
- `--session-id` 由 bootstrap 自身 `env::set_var` 后 exec → cc 树带标记，list 扫描面不变（不依赖「pasta 是否透传 env」这一未取证行为；pasta 自身的标记由 `Command::env` 保证，仅用于识别 pasta 进程本身）。
- mode=mountns（pasta 路径）/ selfmap（slirp 路径）是 11 的两个入口（§2.2）。
- 未取证行为显式登记为 09 验收项：**pasta 的退出码是否透传 child 退出码**（冒烟：`pasta -- sh -c 'exit 7'; echo $?`）；`-f` 必须显式传（无 `-f` 时 pasta fork 后台 + syslog，AltScan §9-OQ2），iso-cc 的 wait 对象是 pasta 进程。

---

## 2. 11 收敛后的 ns.rs 边界

### 2.1 原则（CompScan #1/#2 裁决落地）

- crate 线：`nix::sched::unshare` / `nix::mount::mount`（或 rustix，cargo add 定版）；拒绝 unshare(1)/mount(8) CLI 线（execve 纪律 + 无法承载 pre_exec 装配序，CompScan #1）；拒绝 `unshare` crate（0.7.0，2021 死亡，CompScan §12）。
- 唯一 unsafe 面 = ns.rs；每个 unsafe fn 的前置条件写成 SAFETY 注释并由入口保证（issue 11 第 3 条）；错误一律 `last_os_error`，无文案（原语层）。
- **CString 在父进程预转换**，pre_exec 闭包只持转换结果——消除现闭包内分配（Facts §2 `:127-144` 语义不变，信号安全更严）。

### 2.2 剩余 unsafe 清单与 SAFETY 草案

```rust
// 入口 A（primary，pasta spawn 之下）：ns::enter_mountns
pub fn enter_mountns(binds: &[(CString, CString)], expected_ppid: u32) -> io::Result<()>;
//   SAFETY(入口)：仅 fork 后 exec 前的单线程上下文调用；binds 已预转换；
//   调用者已处于 pasta 建立的 userns/netns（E2），故 CLONE_NEWNS 合法。

unsafe fn unshare_mountns() -> io::Result<()>;
//   SAFETY: 单线程 pre_exec；CLONE_NEWNS 仅需调用者位于目标 userns（入口 A 保证）。
unsafe fn make_root_private() -> io::Result<()>;
//   SAFETY: mount(2) 于本 ns；MS_REC|MS_PRIVATE 不产生宿主可见变化（mountns 私有）。
unsafe fn bind_ro(src: &CString, dst: &CString) -> io::Result<()>;
//   SAFETY: 两次 mount(2)（BIND → BIND|REMOUNT|RDONLY）；dst 存在性由 setup/run 前置断言保证。
pub fn set_pdeathsig_verified(sig: c_int, expected_ppid: u32) -> io::Result<()>;
//   SAFETY: prctl(2)/getppid(2)/kill(2) 均 AS-safe；对账逻辑见 §4-L1（PR_SET_PDEATHSIG(2const) 竞态清单）。

// 入口 B（slirp 回退专用）：ns::enter_selfmap_ns
pub fn enter_selfmap_ns(binds: &[(CString, CString)], expected_ppid: u32) -> io::Result<()>;
unsafe fn unshare_user_net_mountns() -> io::Result<()>;
//   SAFETY: 单线程 pre_exec；CLONE_NEWUSER 对多线程进程失败——入口保证单线程。
unsafe fn write_self_ugid_map() -> io::Result<()>;
//   SAFETY: user_namespaces(7) 单条自映射规则（先 setgroups=deny，再 uid_map/gid_map 各一行
//   "0 <own> 1"）；open(2)/write(2)/close(2) 均 AS-safe；在挂载操作之前执行（caps 依赖 userns root）。
```

### 2.3 收缩幅度（对现闭包 `session.rs:110-153`）

| 维度 | 现状（Facts §2） | 11 后 |
|---|---|---|
| unshare flag 面 | NEWUSER\|NEWNS\|NEWNET（primary+slirp 共用） | mountns 入口 = 仅 NEWNS；NEWUSER\|NEWNET 移入 selfmap 入口（slirp 专用） |
| uid_map/setgroups 写 | parent 侧 `:163-170`（安全代码但竞态源） | **primary 路径消失**（pasta 内建，E2）；slirp 路径移入 pre_exec 自写（竞态面消失，父死=无窗口） |
| 网络配置 | pre_exec `sh(sysctl)` `:145-148`（吞错） | 移出 ns.rs → 12 的 netcfg `/proc/sys` 直写（bootstrap 期，非 pre_exec 期，非 unsafe） |
| unsafe 操作类型 | unshare/mount/rprivate/bind+remount/prctl 六类混一闭包 | mountns 路径四类（unshare-NS/rprivate/bind-ro/pdeathsig），全部具名函数 + SAFETY |
| session.rs 内 unsafe | `:110-153` 一段大闭包 | **归零**（issue 11 验收原样） |

---

## 3. 12 就绪等待设计

### 3.1 前提修正（README Phase D 第 4 条，CompScan OQ4）

两 provider 均 self-config：pasta `--config-net`（passt.1:720-722）+ slirp `-c`（lo up + tap up + MTU + IP/netmask + **默认路由**，main.c:149-227）→ **issue 12 第 1 条的三条 `ip` 配网命令整体消失**，netlink 职责收缩为「就绪等待 + doctor 断言」；`sysctl -w` → `/proc/sys` 直写（issue 12 第 2 条不变）。issue 12 票面第 1/4 条按此修正（parent 落票）。

### 3.2 就绪等待：provider 无关 netlink dump（CompScan #7 裁决落地）

```rust
// netcfg.rs（12；无 unsafe，同步）
pub fn host_iface_up(name: &str) -> io::Result<()>;   // 宿主侧 sysfs 断言（R8，09 期即在 spawn 前调用）
pub fn disable_ipv6() -> io::Result<()>;              // /proc/sys/net/ipv6/conf/{all,default}/disable_ipv6 直写，fail-loud
pub fn wait_ready(iface: &str, timeout: Duration) -> io::Result<()>;
//   netlink dump 轮询：RTM_GETLINK（iface 存在且 flags 含 IFF_UP）→ RTM_GETROUTE（默认路由且 oif=iface）
//   100ms 间隔；超时错误含步骤名/接口/超时/最后 errno（fail-loud 文案三要素，见 §6）
```

- 调用点：bootstrap 内、exec 前（ns 内视角，provider 无关——pasta/slirp/未来 socks 形态统一）。
- **crate 三选一（CompScan §12/OQ1 候选内裁决）**：`netlink-sys 0.9.0 + netlink-packet-route 0.33.0`（同 org 同日发版，MIT，零 tokio——OQ1 明示 bootstrap 路径倾向同步）。被否：`rtnetlink 0.23.0`（tokio runtime 进树，musl 静态目标与同步 bootstrap 语义双输）；`neli 0.7.4`（多一层抽象无增益）。
- **被否：slirp `-r/--ready-fd` 旁证线**（CompScan #7 标注为可选项）：单 fd 传递管线换亚秒级收益，破坏 provider 无关性；pasta 无 ready-fd 是一手确认缺口（CompScan #7），不引入非对称路径。上游若加 ready-fd 再降级 netlink wait 为 doctor 断言（CompScan OQ2 跟踪项）。
- doctor 侧接口断言保持现有 sysfs/proc 实现（`doctor.rs:176-198`），不引入第二条 netlink 路径（boring）。

### 3.3 ipv6 直写

`/proc/sys/net/ipv6/conf/{all,default}/disable_ipv6` std fs 写，任一失败带路径+errno bail（issue 12 第 2 条 + 第 3 条错误传播）。时机 = bootstrap 内、wait_ready 之前；pasta/slirp 的 self-config 与其并发无害（同一 netns 的 sysctl 语义）。

---

## 4. 13 三层防线设计（Facts §6 缺口逐条闭合）

### L1 PDEATHSIG 链 + 竞态校验

- 链：iso-cc ←pasta（pre_exec 注入，`Command::pre_exec` 对外部二进制同样有效）←bootstrap/cc（ns 内对账 pasta pid）。任何一环死亡沿链下传。
- 竞态防御按 PR_SET_PDEATHSIG(2const) 四条 man 原文（CompScan #6 取证）：①「parent」指**创建线程**——iso-cc 在 spawn 前不得有易退线程（现状无线程，纪律写死）；②prctl 时父已死不发信号——prctl 后 `getppid() != expected_ppid` 即 `kill(self, SIGKILL)`（ns.rs `set_pdeathsig_verified`）；③credential 变更清除 PDEATHSIG——我方路径无 setuid 切换（E2 userns root 为映射语义，非 cred 变更点）；④subreaper 死亡亦发信号——与 L2 配套而非冲突。

### L2 进程组/subreaper 收编（执行者 = iso-cc，无守护进程）

- iso-cc spawn pasta 前自设 `PR_SET_CHILD_SUBREAPER(1)`（CompScan #6：subreaper 与 PDEATHSIG 是配套机制）。
- `Session::wait()`：主线程 wait pasta（spawn 模式）/ child（slirp 模式）；**pasta 退出后进入收编循环**：对被 reparent 到 iso-cc 的孤儿（cc 的 daemon/setsid 逃逸者，一律会经 subreaper reparent）逐个 SIGKILL + reap，直至无子女。KillGuard 兜 spawn 后任何 `?` 早退路径（Facts §3 孤儿面）。
- 拒绝 killpg 线：pgid 需跨 pasta 发现 cc pid（/proc 间接读）且 setsid 逃逸者本就不在组内——subreaper 收编按 pid 直杀，覆盖面严格更大、机制更少。

### L3 对账 sweep（parent -9 / 裸孤儿）

- 观测键补齐：pasta 与 slirp 进程均注入 `ISO_CC_SESSION` env（`Command::env`）；cc 经 bootstrap `--session-id` 自设（§1.4）；`list.rs:11` 注释错位修正（Facts §8）。
- sweep 算法（doctor/gc/每次 run 前调用）：list::scan 得活跃会话集 → 活跃 netns inode 集（`/proc/<pid>/ns/net`）→ 全 /proc 扫 pasta/slirp 进程与 `state_dir/sessions/*` 目录：不命中活跃 netns inode / 无活跃标记者 = residue 候选 → SIGKILL/删除 → 复扫 = 空，输出计数。同 uid 可读，CI 可重放（正本（一）「residue 可枚举且为空」）。

### 与 `systemd-run --scope` cgroup kill 的等价性论证

- cgroup kill 的本质优势 = 内核按成员关系杀全树，不依赖 reparent。L1（链式 PDEATHSIG）覆盖「祖先死→后代死」；L2（subreaper 收编）覆盖「同辈/子代孤儿」——**在有监督的每条退出路径上与 cgroup kill 等终态**；唯一时间差：L2 是 iso-cc 存活期同步收敛，cgroup 是即时。
- iso-cc 自身被 -9 时 subreaper 随死，孤儿 reparent 到 init 逃出 L2 → L3 sweep 给**有界滞后**的相同终态（R6 L60 修订词，§7）。即：三层合成 = 终态等价、即时性分级（即时/同步/有界滞后），且全程 rootless。
- 被否：systemd-run --scope / cgroup 委派线——REQUIREMENTS D1（L133）已拒 systemd-run 变体（unit 名持久=状态面）；且 cgroup 委派依赖 systemd 用户会话，违反 R11 便携（NixOS/musl/无 systemd 宿主）。

---

## 5. 14 两级清单设计

### manifest

- **路径/属主**：`~/.local/state/iso-cc/manifest.json`，user 属主（v1 setup 全程 rootless，正本（一）的 rootful 许可不消费，仅为未来系统级条目保留通道 → R5 修订留一句，§7）。
- **格式**：JSON（`serde_json`，已在依赖面）+ `tempfile` 原子写。**采纳 nix profile 同构先例并保留其三条机制**（CompScan #8 一手源码）：版本字段 + 不认识的版本即报错（profile.cc:144-145）→ `schema` 字段 fail-loud；整清单重建后原子落盘（:254）→ 无就地编辑；`printDiff` 式代际 diff（:921）→ doctor 双向校验的展示形态。**拒绝耦合其 store 语义**（CompScan #8 CLI 线拒绝理由：强绑 /nix/store 违反 R11），只取「版本化 manifest + 双向 diff」模式。被否 TOML：serde_json 零新增依赖 + 先例同构。
- **entries**：`[{kind: provider|mountpoint|profile-state, key, path?, version?, registered_at, reason(指名 R/ADR)}]`（分类 = audit-code-facts §10 汇总）。

### setup（`iso-cc setup [--profile <n>] [--json]`）

幂等收敛循环：①provider 收敛（which + `--version` → 绝对路径+版本 upsert；缺失报包名/上游静态源，不代装——passt 上游单文件，AltScan §10-2）；②挂载点收敛（profile redirect 声明集内缺失者 mkdir/touch + upsert；`/etc/timezone` bind 已弃——Facts §10/F9：cc 语义视线 = TZ env + `/etc/localtime` + Intl，R2 验收集不依赖该文件；`/etc/localtime` 在 NixOS 经符号链接解析 bind 真实路径，E3/bwrap 手法先例佐证）；③manifest 重建 + 原子写。重跑 action diff = 0（正本幂等验收）。

### doctor 双向校验

- forward（清单→现实）：entry 存在性、provider 可执行 + 版本一致（漂移 Warn/缺失 Fail）、mountpoint 存在（缺失 Fail = setup 未跑）。
- reverse（现实→清单）：§4-L3 sweep（孤儿网关/无主 session 目录 = Fail）；stale 条目（指向已消失 profile）= Warn + prune 提示。诚实边界：不扫全盘，校验域 = config 可推导路径 ∪ state_dir ∪ /proc。

### gc（`iso-cc gc [--prune] [--profile <n>] [--all] [--yes]`）

- 默认 sweep：§4-L3，安全条件 = 无活跃会话引用（标记 + netns inode + mountinfo 持有三重判定）。
- `--prune`：清 stale 条目（仅登记簿）。
- `--all`：有活跃会话拒绝；mountpoint 条目自创建后 size/mtime 变化 = 已用户数据化 → 拒绝除非 `--force`；`profile-state` 仅随显式 `--profile` 回收；**provider 二进制永不删除**（非本工具所有），只除名。终态：清单与现实一致（正本验收）。

---

## 6. fail-loud 断言点位（跨 lane 汇总，文案三要素：步骤名/对象/底层原因）

| # | 点位 | lane | 证据锚 |
|---|---|---|---|
| 1 | `net.gateway`/`net.dns` 解析失败（deny_unknown 纪律） | 09 | config.rs 现有纪律 |
| 2 | 清单缺失/provider 路径不可执行（Fail，提示 setup）；版本漂移（Warn） | 14（09 期过渡=which+显式报错） | Facts §4 `:250-252` |
| 3 | 宿主 egress 接口不存在/非 UP（R8 绝不回落；sysfs 实现） | 09 | Facts §1 `:78` 现状只打印 |
| 4 | pasta spawn 失败 / `-f` 前台早期退出（1s try_wait）+ gateway.log tail | 09 | E5 形态 + Facts §6 |
| 5 | bootstrap plan JSON 解析失败（deny_unknown） | 09 | issue 09 spec 2 |
| 6 | pre_exec 任一步（unshare/bind/pdeathsig 对账失败自尽） | 11 | Facts §2 |
| 7 | `/proc/sys` v6 直写失败 | 12 | issue 12 §2 |
| 8 | `wait_ready` 超时（含 gateway.log 指针）| 12 | Facts §5 `:215-217` 反例 |
| 9 | exec 失败 | 09 | 现状保持 |
| 10 | doctor sweep 现役 residue（孤儿网关/无主目录）Fail | 13/14 | Facts §6/§8 |
| 11 | slirp 模式 DNS 上游=宿主 resolver → doctor 恒 Warn（R1 降级，P3b/P13 现形） | 09 | Facts §8 + probe.rs `:169,235` |
| 12 | `-I` 硬规则断言：plan/doctor 校验展开后 ns-ifname 与目标 ns 既有接口名不冲突（egress=lo 类输入直接拒绝） | 09 | issue 10 §Answer 硬规则 + AltScan §8 建议① |

---

## 7. REQUIREMENTS.md 行级修订提案（只提案，parent 落盘）

| 行 | 现文要点 | 两级模型改法 |
|---|---|---|
| L59（R6 验收 b1） | TOML 外无持久状态文件 | 会话过程零持久物；宿主持久物仅限单一清单文件 + 清单登记条目，gc 精确回收 |
| L60（R6 验收 b2） | kill -9 后 diff 为空 | 保留 + 「或被下一次 run/doctor/gc 的 sweep 有界收敛」 |
| L61（R6 验收 b3） | doctor 全量 diff 幂等 | 补「含清单双向校验」 |
| L51-54（R5） | 全程 rootless | 补：setup 条目默认 rootless；未来引入需 root 的条目必须单条登记 + doctor 报告 + gc 回收（正本（一）许可的显式审计通道） |
| L117（N3） | 有界例外①挂载点预创建 | 例外①移至 setup 期并登记；N3 diff 拆「运行期空 / setup 期必须命中清单」 |
| L119（N5） | 运行时 = 1 二进制 + 1 provider | 不变，补「setup 零运行时依赖」 |
| L132（D1） | 不落任何状态 | 会话过程零状态；宿主持久物一律 setup-manifested；「拒绝 systemd-run 变体」理由保留（L2/L3 替代，§4） |
| L146-147（D5） | Rust musl + trait | 按 AltScan §8 收缩自研面表述：「usernet/uid_map 面视作外部依赖；`--print-plan` 展开终点 = 等价 CLI 组合序列（pasta 实参 + bind 清单）」 |
| issue 12 票面 §1/§4 | 三条 ip 配网 + 「slirp=等待+netlink 配置」 | 按两 provider self-config 修正：配网命令删除，netlink=就绪等待+断言；slirp=等待（CompScan OQ4） |

---

## 8. 串行施工计划（README Phase I 序）

**09 → 11 → 12 → 13 → 14；每 lane parent 亲验后放行。** 依赖说明：09 先于 11——spawn 模式当期落地只需把现 pre_exec 闭包**机械裁剪**（删 NEWUSER\|NEWNET、删 maps 写段、删 sysctl sh），不新增 unsafe；11 随后按 §2 抽出双入口。13 先于 14——gc 的 sweep 复用 13 的枚举器。

| lane | 触碰文件 | 冒烟命令 | 验收（对齐票面） |
|---|---|---|---|
| 09 | `src/session.rs`（spawn 重写/闭包裁剪/资产移 `sessions/<id>/`/双标记 env）、`src/provider/{mod,pasta,slirp}.rs` 新、`src/config.rs`（net.gateway/net.dns）、`src/plan.rs`（argv 预览 + `-I` 断言）、`src/doctor.rs`（gateway 选择 + slirp DNS Warn）、`src/main.rs`（SessionBootstrap 新参面）、`src/list.rs`（注释修正） | `cargo nextest run && cargo clippy --all-targets -- -D warnings`；`iso-cc run --print-plan`（含 pasta 全实参 + `-I`）；`iso-cc run -- true` 端到端；`pasta --config-net -q -- sh -c 'exit 7'; echo $?`（退出码透传取证）；`egress="if:lo"` → 拒绝（断言 #12）；`egress="if:nonexistent0"` → 拒绝（#3） | 票 09 A1/A2 合并态：trait + config 选择 + fail-loud + pasta primary（`-I` 硬规则）+ slirp provider；探针不回归 |
| 11 | `src/ns.rs` 新（§2.2 双入口）、`src/session.rs`（pre_exec → 单调用，unsafe 归零） | nextest 全绿（探针断言不改=行为等价）；grep 证 session.rs 无 unsafe | 票 11：unsafe 收敛 ns.rs + SAFETY 注释全（§2.2 草案落地） |
| 12 | `src/netcfg.rs` 新、`src/session.rs`（bootstrap 就绪等待 + v6 直写、删 sh/sh_ok）、`src/provider/*.rs`（就绪 spec）、`Cargo.toml`（netlink-sys/netlink-packet-route，cargo add 定版） | `strace -f -e trace=execve -o /tmp/st iso-cc run -- true` 且无 `ip`/`sysctl` execve；`env PATH=/usr/bin:/bin iso-cc run -- true`（清单钉路径前提）；单测：wait_ready 喂超时 → 带上下文失败 | 票 12 修正后：零外部命令 + fail-loud bootstrap + provider 无关等待 |
| 13 | `src/session.rs`（subreaper + 收编循环 + KillGuard）、`src/list.rs`（sweep 枚举器）、`src/doctor.rs`（sweep 检查组） | kill 链冒烟：`kill -9 <pasta>` → 会话树死（L1）；bootstrap 内起 `setsid sleep 300 &` 后正常退出 → 进程消失（L2）；手工伪造孤儿 pasta → doctor Fail → gc 清空（L3） | audit-code-facts §6 五缺口全闭合；residue 可枚举且为空 |
| 14 | `src/{setup,manifest,gc}.rs` 新、`src/doctor.rs`（双向校验）、`src/main.rs`（Setup/Gc 子命令） | `iso-cc setup && iso-cc setup`（第二次 diff=0）；改版本漂移 → doctor Warn；`gc --all` 后清单与现实一致 | 正本（一）四条验收 |

交叉证明：session.rs 被 09/11/12/13 串行触碰（单写者序）；doctor.rs 被 09/13/14 串行；main.rs 被 09/14；provider 被 09/12——全部无并行交叉。14 合入后 doctor 检查 #2 从过渡态切清单态。

---

## 9. 风险与 open_questions

1. **pasta 退出码透传未取证**：spawn 模式 iso-cc 的 wait 对象是 pasta，child 退出码经 pasta 是否原样透传无正本证据 → 09 首个冒烟项（§8）；若不透传，plan B = bootstrap 落盘退出码 + iso-cc 补读（设计中无此依赖前不预建）。
2. **spawn 模式下 list 的会话根指纹**（AltScan OQ2）：设计以 env 标记（pasta 自身）+ argv 链（bootstrap 自设）双键定案，cmdline 指纹（`--outbound-if4`）仅作 doctor 旁证——09 落地时验证。
3. **pasta+tun2proxy 的 DNS 承接方**（AltScan OQ3）：D4 SOCKS 择型时实测（tun2proxy 虚拟 DNS vs `--dns-forward` 链路）；本稿不预设。
4. **IPv6 直写与 provider self-config 的时序**：理论无冲突（同 netns sysctl），12 冒烟含 `curl -6 必败`（P2）复核。
5. **netstack-smoltcp bus factor**（CompScan OQ3）：仅涉 D4 远期，非本五票范围。
6. **slirp selfmap 入口的 pre_exec 自写 maps**：open/write/close AS-safe（man 约定），但属 11 唯一新增 unsafe 面——如评审不可接受，回退 = slirp 期保留 parent 写 + KillGuard（降级已登记，非默认）。
