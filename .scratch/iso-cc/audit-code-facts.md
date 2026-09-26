# 过渡交付：生命周期相关代码事实清单（设计稿 HOLD，正本 §变更记录（三））

> 来源：SeamDesign lane（2026-09-26），parent 抽验 4/4 属实后落盘（挂载点预创建 :55-69、sh 吞错 :277-279、list.rs:11 注释错位、doctor.rs:222+ 无条件偏好 pasta）。行号基于 HEAD 3a528f5 + session.rs 基线恢复后（295 行）。

只读枚举，手段中立（不含任何 build-vs-reuse 或模块设计裁决）。标注为初步（可错），证据为逐行阅读所得。

## 1. spawn 编排（parent 侧）— `src/session.rs:27-190` `pub fn spawn`

| 位置 | 行为 | 初步标注 |
|---|---|---|
| `:29-30` | `create_dir_all(state_dir)`，失败带上下文退出 | state_dir 本体：`setup-manifested` 候选（自愈目录，或登记） |
| `:33-41` | TZif 写 `state_dir/<profile>/localtime`、`timezone`（tzdb 内嵌数据） | 会话资产，现写入共享 profile 目录 → `residue` 候选（无清理）；资产内容可再生 |
| `:43-45` | resolv.conf 写 `nameserver 10.0.2.3` 到共享 profile 目录 | 同上；并发会话 O_TRUNC 互踩同一 inode（bind 按 inode）→ `residue` 候选 |
| `:47-52` | redirect `src=dst` 解析入 binds | session-scoped 配置投影 |
| `:55-69` | **挂载点预创建**：`binds.retain` 内对宿主缺失 dst `fs::write(dst, b"")`，写失败打印 note 并丢弃该 bind | 现役 `residue`（宿主 FS 写、未登记、崩溃前残留）→ `setup-manifested` 候选 |
| `:71-76` | session_id = `<profile>-<纳秒截断 u32>` | 命名事实：非全局唯一（u32 截断） |
| `:78, :81-104` | egress_iface 取值后仅作为 `--egress-iface` argv 传给子进程；`:183-187` banner 打印 | 宿主侧无接口存在性检查（R8 fail-open 面） |
| `:106` / `:261-275` | apply_env：`ISO_CC_SESSION`（`:262`）、TZ/LANG、profile.env、`env_remove(CLAUDE_CONFIG_DIR)`（`:274`）；**仅注入 child Command，网关 Command 未注入** | 观测缝：任何 sweep/枚举方案缺「网关↔会话」关联键 |
| `:108` | `binds.clone()` 移入 pre_exec 闭包 | — |
| `:110-153` | **unsafe pre_exec 大闭包**（见 §2） | unsafe 面集中地 |
| `:155-160` | child spawn（stdin/stdout/stderr inherit） | 一旦成功，后续任何 `?` 失败均遗留存活子进程 |

## 2. pre_exec syscall 序列 — `session.rs:110-153`（进程内唯一 unsafe 块）

| 位置 | 行为 | 备注 |
|---|---|---|
| `:119` | `unshare(CLONE_NEWUSER\|NEWNS\|NEWNET)`，`ck!` 宏败即 `last_os_error` | userns/netns/mountns：session-scoped 本体 |
| `:120-126` | `/` MS_REC\|MS_PRIVATE | mountns 私有化：宿主 diff 空的保证点 |
| `:127-144` | 逐 bind：`mount(MS_BIND)` + `mount(MS_BIND\|REMOUNT\|RDONLY)`；CString 转换错误 `?` 传播 | bind 只读语义在此 |
| `:145-148` | ipv6_off 时 `sh(&["sysctl", "-w", ...])` ×2 —— **吞错**（`sh` 定义 `:277-279` 不查退出码） | fail-open 点；外部命令依赖点 |
| `:150` | `prctl(PR_SET_PDEATHSIG, SIGKILL)` | **无 getppid 竞态校验**；PDEATHSIG 只覆盖直接子进程；跨 execve 保持 |

## 3. uid_map 写（parent 侧，spawn 之后）— `session.rs:163-170`

| 位置 | 行为 | 失败模式 |
|---|---|---|
| `:165` | `/proc/<pid>/setgroups` = deny，`?` | 任一失败 → `spawn` 返回 Err，**`:155` 已 spawn 的子进程无人击杀**（无 guard/drop 清理）→ 孤儿以未映射 uid 继续跑 |
| `:167-169` | `uid_map`/`gid_map` = `0 <uid> 1`，`?` | 同上；parent 中途被杀 = 同一失败面 |

## 4. 网关 spawn — `session.rs:172-181` + `:250-252`

| 位置 | 行为 | 备注 |
|---|---|---|
| `:250-252` | `gateway_path()` = `which("slirp4netns")`，PATH 解析，缺失报错 | 与 doctor 的 which（`doctor.rs:223-224`）各自独立 = TOCTOU；绝对路径未钉定 → `setup-manifested` 候选（路径登记） |
| `:172-173` | `gateway.log` = `File::create(state_dir.join("gateway.log"))` | 共享 profile 路径 + 每次截断：并发互踩 + 取证丢失 → `residue` 候选 |
| `:174-181` | spawn slirp4netns：`argv=[bin, pid, "tap0"]`，stdin/stdout null，stderr→log | 网关进程活性 = netns 引用之一；无 pgid、无标记 env、无早死检测 |

## 5. bootstrap（ns 内引导）— `session.rs:202-229`

| 位置 | 行为 | 备注 |
|---|---|---|
| `:202` | `_egress_iface` 参数下划线弃用 | 传参零消费 |
| `:206` | `ip link set lo up`（`sh` 吞错） | 外部命令依赖点 |
| `:207-214` | `ip link show tap0` 轮询 ×150×100ms（`sh_ok`，`:281-289` 布尔化） | 固定 15s；外部命令依赖点 |
| `:215-217` | 超时 `bail!("等待 tap0 超时（网关未就绪）")` | 文案无 gateway.log 指针 |
| `:218-222` | `ip link set tap0 up` / `ip addr add 10.0.2.100/24` / `ip route add default via 10.0.2.2`（`sh` 吞错） | 配网失败静默到探针才红；外部命令依赖点 |
| `:223-228` | `Command::exec` 真实命令 | exec 后进程 = 会话进程树根 |

## 6. kill / 收割路径全集

| 路径 | 位置 | 覆盖面 | 缺口 |
|---|---|---|---|
| PDEATHSIG(SIGKILL) | `session.rs:150` | 仅直接子进程（child）；fork→prctl 窗口竞态未校验 | 父先死 → 孤儿；孙进程（cc 的 daemon）不在覆盖面 |
| `Session::wait` 兜底 | `session.rs:232-240` | child.wait → gateway `try_wait`，仍在则 `kill`+`wait` | 仅正常返回路径执行；不 killpg（无进程组）；网关中途死无感知（SYN 黑洞静默） |
| 进程组/subreaper/pidns | 不存在 | — | spawn 未 `process_group(0)`；daemon 孙进程逃逸击杀面且持 netns → 网关不退 → 宿主残留网关+netns |
| parent -9 场景 | 不存在 | 无任何回收者 | residue 现役；无 sweep/对账机制 |
| spawn 错误路径 | `session.rs:163-170` `?` 链 | 无 | 已 spawn child 泄漏（见 §3） |

## 7. state_dir 写入全集 — 定义 `session.rs:243-248`（`$HOME/.local/state/iso-cc/<profile>`）

| 路径 | 写点 | 会话结束清理 |
|---|---|---|
| `localtime`/`timezone` | `:33-41` | 无（无任何 remove 调用） |
| `resolv.conf` | `:43-45` | 无；并发截断互踩 |
| `gateway.log` | `:172-173` | 无；逐会话截断覆盖 |
| `claude/`（redirect 目标，R4 持久态） | 经会话内 cc 写入 | 无（R4 声明持久）→ `setup-manifested` 候选（profile-state） |
| 整目录 | `:29-30` | 无 |

## 8. 观测/对账基础（生命周期证明材料现状）

| 位置 | 行为 | 备注 |
|---|---|---|
| `list.rs:13-37` | /proc 扫 `ISO_CC_SESSION=` env 标记 | `:11` 注释称「网关进程」但标记只注入 child（`session.rs:262`）→ 语义错位；sweep 若建，枚举基础需补网关标记 |
| `doctor.rs:42-52` | `SysInspect` trait（sysctl/iface/path/which/locale/resolve） | 只读缝，sweep 可复用 |
| `doctor.rs:176-198` | egress 接口存在性/默认路由检查（sysfs+/proc/net/route） | 与 spawn 期无检查形成对照 |
| `doctor.rs:222-241` | provider which：无条件偏好 pasta，与 config 无关 | 定位面未钉定 |
| `doctor.rs:278-290` | redirect 挂载点存在性预览 | 只预览，不登记不回收 |
| `probe.rs:32-39, :138, :154, :169, :217, :235` | 探针经 `sh_out` 外部命令（curl/node/getent/date） | verify 路径，非会话引导路径 |

## 9. 生命周期相关依赖面

| 位置 | 事实 |
|---|---|
| `Cargo.toml:13-21` | 运行时依赖：anyhow/clap/libc/serde/serde_json/thiserror/toml/tzdb——无任何 netlink/进程监控类 crate；网络配置全靠外部 `ip`/`sysctl`（`session.rs:145-148,206-222`） |
| `config.rs:21-36, :47-56` | Profile/Net 无 `net.gateway`、无 provider/dns 声明面；`egress` 仅 `if:<name>` |

## 10. 初步标注汇总（正本（一）分类）

- **session-scoped（意图成立，消亡机制有缺口）**：userns/mountns/netns（`:119-126`）、binds（`:127-144`）、tap0 配置（`:218-222`，在 ns 内）、child 进程树。
- **session-scoped 意图 + 消亡未保证**：网关进程（`:174-181`，仅正常路径兜底击杀 `:232-240`）；会话资产三件（localtime/timezone/resolv.conf/gateway.log，无清理）。
- **residue 候选（现役）**：挂载点预创建文件（`:55-69`）；spawn 错误路径孤儿 child（`:163-170`）；PDEATHSIG 竞态孤儿 + daemon 孙进程持 netns 的网关残留（`:150` + 无进程组）；state_dir 无清理积累；并发互踩的共享资产。
- **setup-manifested 候选**：provider 绝对路径+版本登记（现 PATH 解析 `:250-252`）；`claude/` profile 持久态；挂载点预创建（从 residue 迁移）。
- **观测缝**：网关无标记 env（`:262` 仅 child）；list 注释错位（`list.rs:11`）。
