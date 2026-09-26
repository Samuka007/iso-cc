# iso-cc 能力级轮子盘点（component-scan）

日期：2026-09-26 ｜ 范围：能力级逐项调研（CLI 与 crate 两条线）+ 四选一裁决
分工边界：整工具级 build-vs-reuse（podman rootless、toolbox/distrobox、firejail、claude-code 原生 sandbox 等载体形态）不在本报告，另见项目级裁决报告；本报告只回答"每项能力的现成组件怎么选"。

## 方法与证据口径

- **crates.io registry 元数据**（版本/发布日期/license/下载量/仓库）：2026-09-26 经 registry API 逐个拉取。
- **GitHub API**（最新 release/发布日期/pushed_at/license）：同日拉取。
- **上游一手文档**：passt.1（passt.top plain）、slirp4netns.1.md 与 main.c（raw.githubusercontent）、PR_SET_PDEATHSIG(2const)、unshare(1)、systemd-run(1)/systemd.scope(5)（freedesktop man）、util-linux kernel.org 发布索引、Nix src/nix/profile.cc、tun2proxy/RootlessKit README。
- **本机实证**（NixOS WSL2，内核 6.18.33.2-microsoft-standard-WSL2）：`pasta 2026_07_16.090d739`（`--version` 输出 "GNU General Public License, version 2 or later"、Copyright Red Hat）、`slirp4netns 1.3.5`（libslirp 4.9.3）、`unshare from util-linux 2.42.2`、`iproute2 7.1.0`、`nix 2.34.8`；`kernel.unprivileged_userns_clone` 与 `kernel.apparmor_restrict_unprivileged_userns` 均**不存在**（主线内核默认放行，前者是 Debian/Ubuntu 补丁 sysctl）。pasta attach proven invocation 见 `../issues/10-pasta-attach-tunsetiff.md`（parent 亲验）。
- **四选一语义**：
  - **CLI** = shell-out 外部命令（execve 子进程，一次性语义）
  - **crate** = 进程内 Rust 库
  - **复用工具** = 独立组件二进制，作为受监督子进程长期共存（provider 形态）
  - **自研** = 自己写；可站在低层 crate 之上（自研 ≠ 从零写 syscall）

## 0. 结论速览

| # | 能力 | 裁决 | 一句话理由 |
|---|------|------|-----------|
| 1 | userns/netns 建立 | **crate**（nix/rustix + 自写 uid_map） | 能力是两个 syscall + 一个映射文件写入；CLI 线拒绝理由是 execve 纪律不是能力缺失 |
| 2 | ro bind mounts | **crate**（nix::mount/rustix::mount + 自研 bind_ro 包装） | mount(2) 两次调用；bwrap 仅作手法先例 |
| 3 | 用户态网关 | **复用工具**：pasta primary + slirp4netns fallback | 2026 现状比 spec 时更强：passt 前一日发版、slirp 1.3.5 复活活跃、两者均自配 netns 网络 |
| 4 | tun→socks 流还原 | **crate 组合 + 自研 glue**（netstack-smoltcp/smoltcp + tokio-socks + tun-rs） | D4 路线成立且组件全部活跃；CLI 复用（tun2proxy）降级为退路 |
| 5 | netlink 配网 | **crate**（rtnetlink 家族或 neli/netlink-sys） | 职责收缩为就绪等待 + doctor 断言（新证据：两 provider 均 self-config） |
| 6 | 进程树终止 | **自研**（prctl 三层 + procfs 扫描兜底） | 会话生命周期是本工具语义本体，无可复用件；PDEATHSIG 竞态有 man 原文级防御清单 |
| 7 | 就绪握手 | **自研**（netlink wait；slirp ready-fd 旁证） | pasta 无 ready-fd（一手确认缺口）；sd_notify 限 systemd 场景 |
| 8 | 清单/GC | **自研薄**（serde_json + tempfile；nix profile manifest 作先例） | 无通用件；nix profile.cc 的版本化 manifest + 代际 diff 是同构先例 |
| 9 | locale 注入 | **自研薄**（env + realpath bind；verify 侧可选 jiff） | 行为组合无独立组件；jiff 内嵌 tzdb 服务 P6 探针 |
| 10 | fail-loud 编排载体 | **Rust**（维持 D5） | 全部所需 crate 2025-2026 活跃；shell/Python/Go 各有一手级否决理由 |

**对既有裁决的复验结论**：未发现 2026 现状推翻 D2/D4/D5/D7 任一条；D5（Rust musl + provider trait）、D7（pasta primary / slirp fallback）、D4（SOCKS 形态 crate 线）全部维持且证据增强。**唯一实质性修正**：slirp4netns `-c/--configure` 源码证实会一并配置**默认路由**（main.c:209-227），issue-12 第 4 条"slirp = 等待+netlink 配置"的假设需要按此修正——slirp provider 与 pasta 一样只剩"等待"职责。

---

## 1. userns/netns 建立（R5/T2/T3）

**CLI 线**
- `unshare(1)`（util-linux）：能力完备——`-r/--map-root-user`（= `--map-user=0 --map-group=0`）、`--map-current-user`、`--map-users auto|subids`、`--kill-child[=sig]`、`-U/-m [file]`（持久化 ns bind mount，恰是 R6 要消灭的形态）；unshare.c 头为 GPL-2.0-or-later。项目活跃：kernel.org v2.42.0(2026-04-01)→2.42.4(2026-09-21) 一年四发。https://man7.org/linux/man-pages/man1/unshare.1.html 、https://www.kernel.org/pub/linux/utils/util-linux/v2.42/
- `bubblewrap`：v0.13.0 @ 2026-09-22，LGPL-2.1（COPYING 首行）。作为 userns 原语可用，但形态是完整沙箱载体（整工具级裁决范围）。https://github.com/containers/bubblewrap
- `RootlessKit`：v3.2.0 @ 2026-09-19，Apache-2.0。README 明示依赖 `newuidmap/newgidmap + /etc/subuid`（"requires /etc/subuid and /etc/subgid to be configured by the real root user"）——与我们"仅映射自身 uid、无需 setuid 助手"的 R5 路线冲突；pasta driver 标 experimental。https://github.com/rootless-containers/RootlessKit

**crate 线**
- `nix`：v0.31.3（2026-05-11），MIT，8.2 亿下载——`sched::unshare`、`mount`、`prctl` 全覆盖，事实标准。
- `rustix`：v1.1.5（2026-09-16），Apache-2.0 WITH LLVM-exception OR MIT——syscall 薄封装备选。
- `unshare` crate：**v0.7.0 @ 2021-05-04，crates.io 无仓库链接——死亡 5 年，排除**。

**裁决：crate。** `nix::sched::unshare(CLONE_NEWUSER|NEWNET|NEWNS)` + 自写 `/proc/self/uid_map`（仅映射自身 uid，~20 行 fs 写入，R5 规格原文）。CLI 拒绝理由不是能力（unshare(1) 什么都能做）而是形态：issue-12 的零外部 execve 纪律 + fork/pre_exec 内原子进入语义 + unshare(1) 把"建 ns"和"跑命令"耦在一个二进制里，无法承载我们的 pre_exec 装配序列。

## 2. ro bind mounts / mountns（R2/R3/R4，票 11）

**CLI 线**：`mount(8)`（util-linux，同上活跃）——同 execve 纪律拒绝。`bwrap --ro-bind`：手法先例（bind 后 remount RDONLY），但作为执行载体属整工具级范围。

**crate 线**：`nix::mount::mount` / `rustix::mount::mount`——MS_BIND → 再 remount(`MS_BIND|MS_RDONLY|MS_NOSUID…`) 两步；`MS_REC|MS_PRIVATE` 根隔离。无更上层维护中 crate 值得引入（上层封装反而藏住 remount 时序）。

**裁决：crate + 自研薄语义包装**（票 11 已规格 `bind_ro(src,dst)` + SAFETY 注释）。bwrap 引为"解析符号链接后 ro bind"的手法先例（R2 的 `/etc/localtime` 真实路径 bind）。

## 3. 用户态网关（usernet provider，R1/R7/R8/D5/D7，票 09）

**复用工具（胜出）**
- **pasta/passt**（primary）：GPL-2.0-or-later（本机 `pasta --version`）；发布节奏极密——2026 年已发 2026_01_17/01_20/05_07/05_26/06_11/07_16/07_28/**09_25**（本报告前一日！），https://passt.top/passt/refs/tags 。关键 flag 面一手取证（passt.1 + 本机 --help）：
  - `--outbound-if4/--outbound-if6 NAME`：**"Bind IPv4 outbound sockets to host interface name"**（passt.1:242-256）——接口级 egress 钉定原语，R7/R8 契约直接映射；slirp4netns 仅有地址级 `--outbound-addr`，契约贴合度 pasta 更高。
  - `--netns PATH`/`--userns PATH` attach 模式（passt.1:681-690）；本机 proven invocation（票 10）+ TUNSETIFF 撞名根因（`-I` 默认取 outbound 接口名，passt.1:653-656）。
  - `--config-net`：ns 内地址+路由+up 一步配齐（passt.1:720-722）→ guest 侧 netlink 配网消失。
  - `--map-host-loopback`：guest→addr 重定向宿主，宿主侧呈现 src/dst 127.0.0.1（passt.1:376-379）→ R3 hostgw 别名原语。
  - `--dns-forward ADDR`：addr:53 UDP/TCP 转发（passt.1:276-279）→ resolv.conf bind 的对端。
  - `-t/-u/-T/-U` 端口转发 `default: auto`（本机 --help L88-125）→ R3 dev server 透传。
  - 生命周期耦合良性：**目标 netns 引用删除时 pasta 自行退出**（passt.1:699-702）→ R6 零残留的机理之一。
  - 无状态 TCP 反射（passt.1:48-51 "doesn't implement a full TCP stack"）→ 每会话一个 pasta 的内存代价低（D1 代价论证的机理支撑）。
  - **缺口**：全文无 ready-fd（--help 与 passt.1 均无）→ 见第 7 行。
- **slirp4netns**（fallback，Ubuntu 22.04 无 passt 包的 D5 约束）：v1.3.5 @ 2026-08-27（repo pushed 同日），GPL-2.0（COPYING 为 GPLv2 文本）。**新证据**：`-c/--configure` 源码（main.c:149-227）证实配置序列 = lo up + tap up + MTU + IP/netmask + **默认路由**（SIOCADDRT gateway=10.0.2.2）；`-r/--ready-fd` 初始化完成后写 "1"（slirp4netns.1.md）。性能注：内核 tcp_rmem 默认值上调拖慢其端口转发（README 引 issue #128）——回退档位即可，非阻塞。https://github.com/rootless-containers/slirp4netns
- 其他：`gvisor-tap-vsock/gvproxy`（Apache-2.0，podman machine 生态）无契约优势，不取。

**CLI 线**：不适用——网关是长驻受监督子进程（`GatewayProvider::spawn` + 监控/收割），不是一次性命令。

**裁决：复用工具，pasta primary + slirp4netns fallback（D5/D7 维持）。** 自研 netstack 网关不在此行（归第 4 行 socks 形态通道）；两条 provider 均自配 netns 网络后，编排侧网络职责只剩等待与断言。

## 4. tun→socks 流还原（D4 SOCKS 形态候选）

**crate 线（胜出）**
- `smoltcp`：v0.14.0（2026-08-17），**0BSD**，4.6k stars——纯 Rust TCP/IP 栈核心。https://github.com/smoltcp-rs/smoltcp
- `netstack-smoltcp`：v0.2.4（2026-07-10），MIT OR Apache-2.0，repo github.com/cavivie/netstack-smoltcp——smoltcp 之上的会话层封装（TUN fd → TCP 流/UDP 包事件），D4 点名件，持续更新中。https://crates.io/crates/netstack-smoltcp
- `tokio-socks`：v0.5.3（2026-05-29），MIT（sticnarf/tokio-socks）——SOCKS5 客户端 pump。
- `tun-rs`：v2.8.11（2026-09-17），Apache-2.0——TUN 分配（旧线 `tun` 0.8.14 为 WTFPL、meh/rust-tun，不取）。

**CLI 线（降级为退路）**
- `tun2proxy`（tun2proxy/tun2proxy）：v0.8.3 @ 2026-07-23，MIT，Rust，活跃（pushed 2026-09-21）。功能重合度最高：virtual-DNS（53 拦截 → 198.18.0.0/15 虚拟池）、SOCKS5 UDP、`--tun-fd`、甚至自带 `--unshare`（自建非特权 netns）——**反证 D4 形态是业界标准做法**。否决为首选的理由：`--setup` 需 root（README 明示）、路由/DNS 是宿主面操作（即使限定 netns 内，也引入第二个自带路由逻辑的外部二进制，与票 12 fail-loud/零外部命令纪律冲突）。可作为 provider trait 的第三实现备胎。https://github.com/tun2proxy/tun2proxy
- `hev-socks5-tunnel`（heiher）：2.17.1 @ 2026-08-12，MIT，C，YAML 配置，TUN 自管——引入 C 组件无收益。https://github.com/heiher/hev-socks5-tunnel
- `xjasonlyu/tun2socks`：v2.7.0 @ 2026-07-12，MIT，Go/gvisor——重运行时，同上无优势。

**裁决：crate 组合 + 自研 glue**（D4 维持）。首选 `netstack-smoltcp + tokio-socks + tun-rs`（tokio 仅此形态进树）；退路 `smoltcp` 直用 + 自写会话层（glue 从 300–500 行涨到 ~1000 行，D4 估计失真，故 netstack-smoltcp 优先）；最后退路 tun2proxy 子进程化。

## 5. netlink 配网（票 12）

**CLI 线**：`ip`（iproute2，本机 7.1.0）——票 12 已拒（PATH 依赖 + 吞错），维持。

**crate 线**
- `rtnetlink` 家族：v0.23.0（2026-08-18）+ `netlink-sys` 0.9.0 + `netlink-packet-route` 0.33.0（同日发版），MIT，rust-netlink org（pushed 2026-09-18）。rtnetlink API 为 async（tokio）。
- `neli`：v0.7.4（2026-01-28），BSD-3-Clause——sync 友好的另一栈。https://github.com/jbaublitz/neli

**裁决：crate + 自研薄同步封装**（票 12 已规格 `link_up/addr_add/default_route` + tap 就绪等待）。**范围修正（新证据）**：pasta `--config-net` 与 slirp `-c` 均自配网络后，运行时 netlink 职责收缩为 **tap0 就绪等待 + doctor 断言**（宿主 egress 接口存在且有默认路由，R8 fail-closed 前置检查）；配网三连仅在自定义 netstack 形态（第 4 行）下才需要。sync 栈选型（netlink-sys+packet-route 直用 vs neli vs rtnetlink+block_on）待第 4 行是否引入 tokio 裁决后定，见 open_questions。

## 6. 进程树终止 / 会话生命周期（R6/N3，spec 审计章）

**CLI 线**
- `systemd-run --user --scope`：一手语义（systemd-run(1)）——transient scope unit，systemd-run 自己当父进程、同步执行、受 service manager 管理并出现在 `systemctl list-units`。systemd.scope(5)：scope 生命周期 = "the existence of at least one process in the scope"，无 main process，退出状态由原父进程收割。**D1 拒绝理由修正**：spec 写"unit 名持久、终端关闭后 unit 存活"对 `--scope` 不准确（scope 是 transient，随进程消亡）；**真实成立的拒绝理由**是：①systemd 依赖违反 R11（musl/非 systemd 宿主直接哑掉）；②`list-units` 可见状态面与 R6 精神冲突；③父子关系反转（carrier 变成 systemd-run，我们的 exec RPC/信号面被中介）。能力面（cgroup 整树 SIGKILL 含 pasta 孙进程）确实是它独有的强项——我们的自研三层防线必须证明等价覆盖。https://www.freedesktop.org/software/systemd/man/latest/systemd-run.html
- `unshare --kill-child[=sig]`：PDEATHSIG 的 CLI 先例，仅作参照。

**crate 线**
- `nix`/`libc` `prctl`：`PR_SET_PDEATHSIG` + `PR_SET_CHILD_SUBREAPER`。**man 原文级竞态清单**（PR_SET_PDEATHSIG(2const)）：
  1. "parent" 指创建它的**线程**，不是进程——父进程内线程退出即触发（CAVEATS 原文）；
  2. prctl 执行时父已死 → **无信号发出**（L91-93）——必须 prctl 后自查父存活（race 窗口）；
  3. credential 变更（euid/egid/fsuid/fsgid）**清除** PDEATHSIG（L105-107）；
  4. 信号在 reparent 到 subreaper 后、subreaper 死亡时也会发出（L87-90）——subreaper 与 PDEATHSIG 是配套机制而非替代。
- `procfs`：v0.18.0（2025-08-30），MIT OR Apache-2.0——`/proc` 扫描：`list` 子命令的会话发现、netns 持有者枚举（`/proc/*/ns/net` inode 对比）= doctor 残留检测的实现载体。https://github.com/eminence/procfs

**裁决：自研**。会话生命周期（subreaper 收割孙进程 + 进程组 killpg + PDEATHSIG 兜底 + procfs 扫描断言净空）是本工具的需求本体（R6），无现成组件可"复用"；spec 状态审计章列的嫌疑（PDEATHSIG 竞态、daemon 孙进程持 netns）逐条有上面的 man 原文对应防御。

## 7. 就绪握手（T3 bootstrap）

- **slirp4netns**：`-r/--ready-fd=FD`——初始化完成后写 "1" 并关闭（slirp4netns.1.md；main.c:446-47 帮助原文）。
- **pasta**：**无 ready-fd**（本机 --help 全文 + passt.1 全文均无 ready 字样）——一手确认的缺口。
- **sd_notify 路线**：`sd-notify` crate v0.5.0（2026-03-09，MIT OR Apache-2.0）可用且活跃，但仅 systemd 场景有意义——D1/R11 下拒绝。

**裁决：自研 netlink 等待**——RTMGRP_LINK/路由 dump 轮询：tap 接口出现 + UP + 默认路由存在即就绪，超时 fail-loud 报错退出（票 12 第 1 条后半的规格化）；provider 无关，pasta/slirp/未来 netstack 形态统一走它；slirp 的 ready-fd 可作加速旁证（可选项）。规模 ~100–150 行，站在第 5 行的 netlink crate 上。

## 8. 清单/GC（setup-manifested 资源两级模型，spec 2026-09-26 修订一）

**先例（一手源码）**
- Nix profile：`src/nix/profile.cc:126` `manifestPath = profile / "manifest.json"`；JSON 带版本字段、不认识版本即报错（:144-145）；安装/卸载重建整个 manifest 后原子落盘（:254）；`nix profile list` 打印元素、代际间 `printDiff`（:921）——**"版本化 manifest + 声明态 vs 现实态双向 diff"与我们 doctor+gc 的同构先例**。https://github.com/NixOS/nix/blob/master/src/nix/profile.cc
- systemd transient scope：session-scoped 资源"随进程消亡"的先例（第 6 行）。

**crate 线**：`serde_json` + `tempfile` v3.27.0（2026-03-11，MIT OR Apache-2.0）原子写。

**CLI 线**：无通用件——nix profile 强绑 store 语义（违反 R11 不可作依赖），systemd unit 非资源清单。

**裁决：自研薄清单**（manifest JSON：类型/路径/创建记录/版本字段 + `gc` 按 manifest 精确回收 + doctor 双向 diff；~200–300 行）。规格归属票 02，本行只裁决"无现成件、先例成立"。

## 9. locale 注入（R2，票 03）

- 无现成"组件"：`timedatectl/localectl`（systemd，宿主级系统配置器）、`tzselect`（交互式）形态均不符。
- 手法先例：bwrap `--ro-bind`（符号链接解析后 bind 真实路径）；票 03 已按此验收（P6a/P6b 过，tzdb 内嵌 TZif 为 bind 缺失时的覆盖路线）。
- verify 侧可选 crate：`jiff` v0.2.37（2026-09-12，Unlicense OR MIT，BurntSushi，内嵌 tzdb）——P6 探针的独立时区事实源（与会话内 glibc/Intl 视线交叉验证）；备选 `chrono-tz` 0.10.4（2025-07-11）。

**裁决：自研薄**（env TZ/LANG/LC_* + realpath 后 bind `/etc/localtime`、`/etc/timezone` + C.UTF-8 回退 + doctor 警告，R2 规格原文；jiff 仅进 verify 探针，不进核心）。

## 10. fail-loud 编排载体语言（D5 复验）

**裁决：Rust（维持 D5），本轮全量证据增强**：
- 所需 crate 全部 2025-2026 活跃且 license 宽松（nix/rustix/rtnetlink 家族/neli/smoltcp/netstack-smoltcp/tokio-socks/tun-rs/procfs/tempfile/clap/toml/thiserror，见各行与 §12 表）；
- musl 静态目标（R11 单二进制 + pasta 的分发形态）无生态障碍；
- fail-loud 纪律有语言级承载：`io::Error` 上下文链 + `thiserror` v2.0.21（2026-09-23）；反面教材为票 12 记录的 `sh()` 吞错（退出码不查、静默续跑到探针才红），正面范本为 T1 的 srt `refuseSettings`（环境不合格拒绝启动）。

**被否替代**：
- **shell**：票 12 一手证据（`ip`/`sysctl` 依赖 + 吞错），D5 原文"shell 最脆的两件事（syscall 编排 + 子进程生命周期）"；
- **Python**：解释器依赖破坏单二进制分发；
- **Go**：可行但无优势——同域先例 RootlessKit/gvproxy 均为整编排层形态（与 thin 目标相悖），且 RootlessKit 路线强制 newuidmap/subuid（与 R5 自映射路线冲突，见第 1 行）；Go 线未做 crate 级深查，若未来 Rust 生态某关键件断维护再复评。

## 11. 相邻能力附带盘点

| 能力 | 选择 | 证据 |
|------|------|------|
| TOML config 解析 | crate `toml` v1.1.6+spec-1.1.0（2026-09-10） | 已到 1.x；spec 依赖纪律：版本由 `cargo add` resolver 定，禁手写 |
| CLI 解析 | crate `clap` v4.6.7（2026-09-14） | 事实标准，MIT OR Apache-2.0 |
| `/proc/sys` 直写（v6 off） | std fs（无依赖） | 票 12 第 2 条 |
| TUN 分配（socks 形态） | crate `tun-rs` 2.8.11（Apache-2.0） | 旧 `tun` 0.8.14 为 WTFPL 不取 |
| 原子文件写 | crate `tempfile` 3.27.0 | manifest/resolv.conf 暂存 |

## 12. 全量组件元数据表（crates.io / GitHub API，2026-09-26）

| 组件 | 最新版 | 发布日期 | License | 活跃度证据 |
|------|--------|----------|---------|-----------|
| nix | 0.31.3 | 2026-05-11 | MIT | 下载 8.2 亿 |
| rustix | 1.1.5 | 2026-09-16 | Apache-2.0 W/ LLVM-exc OR MIT | 下载 11.7 亿 |
| rtnetlink | 0.23.0 | 2026-08-18 | MIT | repo pushed 2026-09-18 |
| netlink-sys | 0.9.0 | 2026-08-18 | MIT | 同上 org |
| netlink-packet-route | 0.33.0 | 2026-08-18 | MIT | 同上 |
| neli | 0.7.4 | 2026-01-28 | BSD-3-Clause | pushed 2026-06-19 |
| smoltcp | 0.14.0 | 2026-08-17 | 0BSD | 4.6k stars |
| netstack-smoltcp | 0.2.4 | 2026-07-10 | MIT OR Apache-2.0 | 单人 repo（见 OQ） |
| tokio-socks | 0.5.3 | 2026-05-29 | MIT | sticnarf |
| tun-rs | 2.8.11 | 2026-09-17 | Apache-2.0 | 下载 54 万 |
| procfs | 0.18.0 | 2025-08-30 | MIT OR Apache-2.0 | pushed 2026-06-26 |
| unshare | 0.7.0 | 2021-05-04 | MIT/Apache-2.0 | **死亡，排除** |
| sd-notify | 0.5.0 | 2026-03-09 | MIT OR Apache-2.0 | 仅 systemd 场景 |
| tempfile | 3.27.0 | 2026-03-11 | MIT OR Apache-2.0 | 下载 8.3 亿 |
| clap | 4.6.7 | 2026-09-14 | MIT OR Apache-2.0 | 下载 11.6 亿 |
| toml | 1.1.6+spec-1.1.0 | 2026-09-10 | MIT OR Apache-2.0 | 1.x 落地 |
| thiserror | 2.0.21 | 2026-09-23 | MIT OR Apache-2.0 | 下载 15 亿 |
| jiff | 0.2.37 | 2026-09-12 | Unlicense OR MIT | BurntSushi |
| passt/pasta | 2026_09_25 | 2026-09-25 | GPL-2.0-or-later | 年内 ≥8 发版 |
| slirp4netns | v1.3.5 | 2026-08-27 | GPL-2.0 | released=pushed 同日 |
| RootlessKit | v3.2.0 | 2026-09-19 | Apache-2.0 | pushed 2026-09-24 |
| bubblewrap | v0.13.0 | 2026-09-22 | LGPL-2.1 | pushed 2026-09-25 |
| util-linux unshare | 2.42.4 | 2026-09-21 | GPL-2.0-or-later（unshare.c 头） | 年发 4 版 |
| tun2proxy | v0.8.3 | 2026-07-23 | MIT | pushed 2026-09-21 |
| hev-socks5-tunnel | 2.17.1 | 2026-08-12 | MIT | 多平台产物 |
| xjasonlyu/tun2socks | v2.7.0 | 2026-07-12 | MIT | pushed 2026-09-13 |

## 13. Open questions

1. **sync netlink 栈选型**：netlink-sys+netlink-packet-route 直用（零 tokio）vs neli vs rtnetlink+block_on——待 D4 SOCKS 形态是否让 tokio 进树后定；bootstrap 路径倾向保持同步（票 12 语义）。
2. **pasta readiness 上游动向**：上游无 ready-fd；若未来版本加入，netlink wait 可降级为 doctor 断言。跟踪 passt 发布说明即可。
3. **netstack-smoltcp bus factor**：单人 repo（cavivie），采用证据（clash 系）非组织背书；D4 票开工前做 glue spike 复核 300–500 行估计，并保留 smoltcp 直用退路。
4. **票 12 spec 修正**：slirp `-c` 已含默认路由（main.c:209-227），第 4 条"slirp = 等待+netlink 配置"应改为"slirp = 等待"；netlink 配网三连与 socks 形态绑定。
5. **manifest schema 归属**：nix 先例的"版本字段 + 双向 diff"是否进票 02 doctor 规格，由该票裁决；本报告仅供先例。
6. **RootlessKit v3 的 pasta driver 仍标 experimental**：若未来考虑把 ns 建立外包给 RootlessKit 需先复验该标记；当前不取（第 1 行）。
