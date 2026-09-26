# iso-cc 替代方案扫描：build-vs-reuse 全项目级裁决

日期：2026-09-26 ｜ 范围：spec 变更记录（三）"实现手段开放性" 要求的全项目复验
需求正本：`docs/REQUIREMENTS.md`（R1–R12/N1–N6/D1–D8）｜ 拒绝记录复验对象：D2（拒容器）、D7（网关选型 pasta）、D4（拒 v1 内置 tun2socks 自研，已修订为 embedded SOCKS 候选）

**方法与证据等级**
- A 级 = 本机一手实验（本报告 §2，2026-09-26，NixOS WSL2 / 内核 6.18，passt-2026_07_16.090d739、bubblewrap 0.11.2、slirp4netns 1.3.5）；
- B 级 = 上游一手文档/man page/release notes（标注 URL 与检索日期 2026-09-26）；
- C 级 = 聚合二手来源（仅作旁证，不单独支撑结论）。
- 隐私：本文件不含私有主机名/隧道商名/订阅信息/出口 IP；宿主以发行版代称。

---

## 1. 结论速览

> **三选一裁决：薄编排成熟 CLI。**
> 网络面（userns+netns+usernet 网关）**零自研**——pasta 一个二进制已完整覆盖 R1/R7/R8 的全部原语（A 级实测 + pasta(1) man page 语义确认）；mountns/locale/路径重定向是 ~10 条 mount 系统调用的薄层，无轮可复用也不值得复用轮子。**没有任何现成工具同时满足本项目的核心需求组合**（§5 对比表全表无一 fits 全绿），因此「直接复用」被否；「继续自研」按 D5 原文（Rust 自行完成 unshare/uid_map/mount 编排）仍可维持，但其内涵应按本报告 §8 收缩为"薄编排壳"：进程树生命周期、config/doctor/verify、fail-loud 断言是自研的全部增量价值所在，namespace 原语面应当作已解决的外部依赖对待。

- 对 D2/D7/D4 三个既有拒绝记录的复验结论：**D2 仍成立、D7 仍成立且证据加强、D4 的 v1 范围裁决仍成立但新增一条"纯组合路径"候选**（详见 §7）。
- 对实现清单的两个具体回赠（不改票，供 09/12 号票参考）：①**pasta attach 形态无阻塞**——issue 10 的 TUNSETIFF EINVAL 根因已由同日专项调查定位为"未给 `-I/--ns-ifname` 时 passt 默认用 outbound 接口名命名 ns 内 tap，与目标 ns 必有的 `lo` 撞名"（passt.1:654-657 文档化行为，podman discussions #22570 同症状），省略该 flag 或配 `-I` 即稳定成功；由此产生一条**硬约束**进 09 票 doctor/plan 断言面（§8 建议①）。本报告 spawn 模式实测（E1/E3/E4/E5）保留为等价形态的补充证据，不再是"绕行"建议。②SOCKS 形态存在 `pasta + tun2proxy` 纯组合方案，embedded 自研 provider（D4 修订稿的 300–500 行）未必是唯一选项（§7.3、§9）。

---

## 2. 本机实证（A 级证据，2026-09-26，命令与输出摘录）

| # | 实验 | 命令骨架 | 结果 |
|---|------|---------|------|
| E1 | pasta spawn 模式管线 | `pasta --config-net -q -- sh -c 'ip -br addr; curl -4 ifconfig.me'` | ✅ namespace 内出现镜像自宿主的 eth0 配置，出口 = 宿主常规出口（值略），`SPAWN_OK` |
| E2 | 组合栈内权限面 | `pasta -- sh -c 'id; capsh --print'` | ✅ `uid=0(root)`，`Current: =ep`（全能力）——pasta 的 userns 只映射当前用户，内部即"root"，无需 newuidmap/setuid |
| E3 | mountns bind（R2 原语） | `pasta -- unshare -m sh -c 'mount --make-rprivate / && mount --bind <file> /etc/hostname && cat /etc/hostname'` | ✅ bind 生效 + 会话内 curl 正常，**纯 CLI 组合零自研代码** |
| E4 | 出口钉定 fail-closed（R1/R8 原语） | `pasta --config-net -q --outbound-if4 docker0 -- sh -c 'curl -4 -m 6 ifconfig.me'` | ✅ curl `rc=7`（连接失败），**无回落**——`--outbound-if4` 对 down 接口的 socket 绑定即为 fail-closed 原语 |
| E5 | 启动开销（N2） | `time (pasta --config-net -q -- true)` | ✅ wall **0.134s**（预算 300ms，且不含本项目壳的解析开销） |
| E6 | bwrap 嵌套组合 | `pasta -- bwrap --unshare-user --dev-bind / / --bind <file> <target>` | ⚠️ 本机 NixOS `/etc` 为符号链接农场（`/etc/hostname → /etc/static/hostname`），bwrap 对 symlink 目标建挂载点报 ENOENT/EPERM；对真实存在的常规文件/目录路径 bwrap 嵌套 userns 本身可用。**结论：bwrap 不必要**——E3 已证明 util-linux `unshare -m` + `mount(8)` 或直接 syscall 在 pasta 的 userns 内即可完成同样的 mountns bind |

E1–E5 共同证明：**R1+R2+R5 的全部 namespace 原语在"成熟 CLI 组合"下今天就能跑通**，且每会话开销在 N2 预算内。E4 证明 fail-closed 原语由 pasta 原生提供，不需要宿主持久规则（对比 D2 拒掉的 fwmark 路线的结构性弱点）。

局限声明：E4 用的是 down 接口（本机无隧道接口可测），"拔线即断"的端到端 fail-closed 验收仍属 T5/issue 04 的验收矩阵，本实验只证明 socket 绑定语义不会静默回落。

勘误（2026-09-26 同日）：E4 等用例中 pasta 默认以 outbound 接口名命名 ns 内 tap（passt.1:654-657）；issue 10 专项调查（报告 /tmp/iso-cc-issue10-report.md，行号级证据）证实 `--outbound-if4 lo` 类用法因与目标 ns 必有的 `lo` 撞名而 TUNSETIFF EINVAL，非环境缺陷。本报告用例的 outbound 名（`docker0`）在全新 ns 内无既有同名接口，故 E4 结论不受影响；**硬约束：禁止对 ns 内已存在的接口名使用 `--outbound-if4` 而不配 `-I/--ns-ifname`**。

---

## 3. 需求基线（判定集合）

| 需求 | 一句话判据 |
|------|-----------|
| R1 | 进程树全部出口（TCP/UDP/DNS/QUIC）钉定到宿主既有 L3 接口，不经应用层代理变量 |
| R8 | egress 失效 = 会话断网，绝不回落宿主直连 |
| R2 | `/etc/localtime`、`/etc/timezone` 对会话进程树为覆盖值（mountns bind + env），Node Intl 视线一致 |
| R4 | `~/.claude*` 等声明路径重定向到 per-profile 目录，cc 无感（mountns bind 列表 + 写集断言） |
| R3 | HOME/docker socket/SSH agent/包环境原样透明共享，不复制 rootfs、不 clearenv |
| R5 | 全程 rootless，无需 root/sudo/setuid/newuidmap |
| R6 | 无 up/down、无常名持久状态；kill 会话即零宿主残留 |
| R7 | 工具不感知隧道协议；egress 契约 = 接口引用 |
| R10/R11/N2 | 并发多会话；NixOS/Ubuntu/Debian/Arch 便携（少依赖）；启动 <300ms |
| 可验证 | `verify` 探针矩阵（P1–P15 等）可在会话内外复跑——**这是现有任何轮子都不具备的产品面**（../iso-cc-research 环境 canary 研究结论：现成覆盖 ≈1.5/15） |

---

## 4. 对比表（工具 × 需求 → fits / partial / no）

判定口径：fits = 原生且满足验收判据；partial = 需绕行/仅部分语义/引入状态或特权面；no = 结构性不满足。

| 工具（版本，2026-09 检索） | R1 钉定 | R8 fail-closed | R2/R4 mountns 路径覆盖 | R3 透明共享 | R5 rootless | R6 零残留 | N2 开销 | verify 可验证 | 综合 |
|---|---|---|---|---|---|---|---|---|---|
| **pasta**（passt 2026_07_16，本机） | ✅ `--outbound-if4/-i`（B: pasta(1)） | ✅ socket 绑定即断（A: E4） | n/a（不管文件系统） | n/a | ✅ 无特权（B: pasta(1)） | ✅ 纯进程生命周期 | ✅ 134ms（A: E5） | n/a | **原语 ✅（编排对象，非产品）** |
| **podman rootless**（keep-id + pasta + bind-only） | partial：`--network=pasta:--outbound-if4,<if>`（B: podman-network(1)） | 同 pasta 语义 | partial：`-v` bind 可做，但 cc 视角路径/uid 语义经容器层 | **no**：rootfs/容器模型 vs "不复制 rootfs" | partial：需 `/etc/subuid` + `newuidmap` 多段映射（B: rootless_tutorial） | **no**：`~/.local/share/containers` 镜像/层/卷为持久状态面 | no：容器启动/存储开销 | no | **no（形态错配）** |
| **toolbox / distrobox** | **no**：默认 host netns；`--unshare-netns` 落 podman bridge NAT，无出口钉定（B: distrobox-create 文档/fedora 讨论） | **no** | partial：bind HOME/X11 等，非声明式覆盖语义 | partial：home 透传但 rootfs 来自镜像 | ✅（容器层） | **no**：容器常驻 + 镜像状态 | no | no | **no** |
| **bubblewrap**（upstream 0.13.0 2026-09-22；本机 0.11.2） | n/a：`--unshare-net` 得到的是**无连通**净 ns，无 usernet | n/a（断网式"fail-closed"过度：连合法流量也断） | ✅ 声明式 `--bind/--ro-bind` | ✅ `--dev-bind / /` 模式 | ✅ userns | ✅ 进程生命周期 | ✅ 毫秒级 | n/a | **原语 ✅，但网络面缺口须 pasta/slirp 补，且 E6 证明可被更薄的 `unshare -m` 替代 → 不引入** |
| **RootlessKit**（v3.2.0 2026-09-19） | partial：`--net=pasta`（**experimental**）与 `--net=slirp4netns`（recommended）；无 `--outbound-if` 透传文档 | partial：同 pasta 语义但被包一层 | **no**：只有 `--copy-up`（tmpfs+symlink 复制语义），无声明式 per-path bind（B: README/docs） | partial：copy-up 复制 ≠ 透明共享 | ✅ | ⚠️ `--state-dir` 落盘 child_pid/socket（/tmp 随机目录，session 期状态文件） | ✅ 量级可接受 | no | **no（mount 面与状态模型不符）** |
| **firejail**（0.9.7x 活跃维护） | partial：`--net` 走 veth，须 setuid root 能力 | partial（同 veth 路线） | partial：profile 机制但非声明式 bind 列表 | partial | **no**：setuid root 二进制；LPE 史（CVE-2022-31214 等，B: oss-security） | ⚠️ | ✅ | no | **no（违反 R5 字面义 + 安全暴露面）** |
| **crun / youki**（1.30.1 / v0.7.0，2026-09 活跃） | n/a：OCI runtime 只建 ns，不管 usernet | n/a | partial：config.json mountpoints 可声明 | **no**：OCI 模型要求 bundle/rootfs | ✅ rootless 能力 | **no**：bundle 目录 + runtime state 目录 | partial | no | **no（= 用更重的壳做本项目本来就要自己写的编排）** |
| **claude-code 原生 sandbox**（code.claude.com/docs/en/sandboxing） | **no**：bwrap+socat 本地代理按域名 allowlist，出口仍走宿主网络栈，无 L3 接口钉定（B: 官方文档/Anthropic 工程博客） | **no**：默认 bwrap/socat 缺失时**回落不沙箱运行**（fail-open 语义） | no：面向"囚禁"（cwd 外写默认拒绝），非声明式路径重定向 | **no**：文件面默认收紧 vs R3 放行 | ✅（bwrap） | ✅ | ✅ | no（其 allowlist 语义与 P 矩阵不同维度） | **no（目标正交：它是"防 cc 干坏事"，本项目是"给 cc 定身份"）** |
| **vopono**（活跃，2025–2026） | partial：per-app netns + 隧道配置管理 | partial：killswitch nft/iptables | **no** | partial | **no**：netns/veth/防火墙设置需 sudo（root daemon 模式）（B: USERGUIDE） | partial（临时 netns 但有防火墙规则面） | partial | no | **no（违反 R5/R6/R7——它管隧道生命周期，正是本项目 R7 排除的边界）** |
| **slirp4netns**（1.3.x） | partial：无按接口名钉定（`--outbound-addr` 按地址） | partial | n/a | n/a | ✅ | ✅ | partial | n/a | **维持"第二 provider"定位（与 ADR 0007 一致）** |
| 其他：nsjail（Go，容器/囚禁模型）、proxychains/LD_PRELOAD（R1 已拒：可绕过）、`systemd-run --user` + PrivateNetwork（D1 已拒 + 用户 session 通常无 netns 能力）、Landlock ABI4（Linux 6.7+ 只能按端口控 connect/bind，**无法按接口钉定出口**，不构成 R1 原语） | — | — | — | — | — | — | — | — | **no（结构或原语面）** |

**表判**：没有任何一行全绿。「直接复用任一现成工具」不成立；网络原语已被 pasta 单点完整覆盖（首行）。

---

## 5. 逐工具裁决（要点与证据）

### 5.1 podman rootless（`--userns=keep-id` + pasta + bind-mount-only）
- **最近似的轮子**：podman 5.x 起 rootless 网络默认即 pasta，且 `--network=pasta:<opts>` 可透传 `--outbound-if4`（B: https://docs.podman.io/en/stable/markdown/podman-network.1.html ；`pasta_options` 见同页 containers.conf 节）。
- **仍不 fits 的结构性理由**：
  1. R3"不复制 rootfs"：容器模型以镜像/bundle 为前提；`--rootfs <dir>` 虽是文档化特性（podman-run(1)），但"拿宿主根目录当 rootfs"非官方支持路径，且 overlay/fuse-overlayfs/vfs 的层语义与"项目文件读写即宿主文件"的目标持续打架。
  2. R5 细节：rootless podman 依赖 `/etc/subuid`/`/etc/subgid` + `newuidmap`（setuid 助手）多段映射（B: https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md ）；iso-cc 只映射自身 uid、内核直写 uid_map，零宿主配置前提（A: E2）。
  3. R6：`~/.local/share/containers` 的镜像/层/卷是设计内的持久状态（同上引用），与"config 之外无任何持久状态"直接冲突；`--rm` 清容器不清存储。
  4. R4 写集断言（T4 核心：实测写集 ⊆ 声明集）无法穿过容器层成立——容器内 `find ~ -newer` 的宿主可见性依赖挂载传播细节，断言面变脆。
- **裁决**：no。podman 的 pasta 集成是 D7 的旁证（生态全量检验了我们要用的网关），但容器壳本身与 R3/R6 正交冲突。

### 5.2 toolbox / distrobox
- 默认与宿主共享 netns（`--network=host` 语义）；`--unshare-netns` 后落入 podman bridge NAT，**无出口钉定**，也无 fail-closed 语义（B: https://distrobox.it/usage/distrobox-create/ ；fedora 讨论串 https://discussion.fedoraproject.org/t/network-isolation-when-using-toolbx-or-distrobox/129932 ）。
- 镜像 rootfs + 常驻容器，R3/R6 同 5.1。裁决：no（连 R1 都不满足）。

### 5.3 bubblewrap（± pasta/slirp）
- bwrap 本体活跃（v0.13.0，2026-09-22）。声明式 bind（`--bind/--ro-bind/--dev-bind`）与 R2/R4 语义同构，userns 干净（无 setuid）。
- **但**：`--unshare-net` 只给孤立 netns，无 usernet；网络面必须外挂 pasta/slirp，而嵌套方向只能是 pasta 在外（B: bwrap 文档/flatpak 实践；A: E6 嵌套在 pasta userns 内可用）。此时 bwrap 的增量价值 = 比 `unshare -m + mount(8)` 多一个参数化 CLI——E3 已证明后者够用（且 iso-cc 是进程编排者，直接 syscall 更符合 D5"最脆的两件事自己做"的原判断）。NixOS 的 `/etc` 符号链接农场还暴露了 bwrap 建挂载点的边界 bug 类风险（A: E6）。
- 裁决：原语 ✅ 但**不引入**——被更薄的 `unshare -m`/直接 syscall 替代；列为 slirp/pasta 之外的备选挂载面仅当未来需要"每路径只读化"等 bwrap 特性。

### 5.4 RootlessKit
- 活跃（v3.2.0，2026-09-19）。提供 userns+netns+usernet+端口驱动+`--state-dir` 全家桶，`--net=pasta` 存在但上游标注 **experimental**、自荐序仍是 `--net=slirp4netns`（B: https://github.com/rootless-containers/rootlesskit/blob/master/docs/network.md ，含 2026-07 基准表：pasta 大 MTU 31.9 Gbps vs slirp 8.11）。
- 结构性不符：①mount 面只有 `--copy-up`（tmpfs+symlink **复制**语义，B: README/issues/355），做不了 R2/R4 的声明式 bind 覆盖（把宿主文件 bind 盖到会话视角），copy-up 反而破坏 R3"原样共享"；②`--state-dir` 落盘 child_pid/socket，与 D1"无状态会话"模型相逆（虽然可归入 spec 变更（一）的 session-scoped 资源，但无收益）；③无 `--outbound-if` 透传出口钉定。
- 裁决：no。它是"给 dockerd 当底座"的形状，不是"给单会话 CLI 当底座"的形状。

### 5.5 firejail
- `--net` 需要 setuid root 能力做 veth/桥接；setuid 单体二进制有 LPE 前科（CVE-2022-31214，B: https://www.openwall.com/lists/oss-security/2022/06/08/10 ）；2025 年仍在活跃发版（0.9.7x，B: https://firejail.wordpress.com/download-2/release-notes/ ）。
- 裁决：no。违反 R5 字面义（spec 明确"不需要 root/sudo/setuid"），安全暴露面与 D7"网关是最暴露组件、要经全量用户检验"的选型哲学相逆。

### 5.6 crun / youki（OCI rootless runtime 直接编排）
- 两者都活跃（crun 1.30.1 2026-09-25；youki v0.7.0 2026-07-25）。rootless 能力成熟。
- 但 OCI runtime 只解决"把 config.json 变成 ns 里的进程"：usernet、出口钉定、声明式 bind 列表、生命周期收割、doctor/verify 全部仍要自己做——等于把 D5 的 syscall 编排外包给一个要求 JSON bundle + state 目录 + rootfs 惯例的更重协议。youki 是 Rust，但它复制的是 runc 的协议面，不是我们的产品面。
- 裁决：no。**自研编排比编排 OCI runtime 更薄**。

### 5.7 claude-code 原生 sandbox
- 官方能力：Linux 上 bwrap（文件/内核 ns）+ socat 本地代理（按 `sandbox.network.allowedDomains` 域名 allowlist）做 bash 沙箱（B: https://code.claude.com/docs/en/sandboxing ；https://www.anthropic.com/engineering/claude-code-sandboxing ）。
- 目标正交 + 三个硬缺口：①网络是**域名 allowlist**，流量仍从宿主网络栈出去——与 R1"L3 接口钉定"完全不同维度，也覆盖不了 R12 的出口三面一致；②依赖缺失时默认**回落不沙箱**（fail-open 语义，与 N1 直接相反）；③文件面是"囚禁"（cwd 外默认拒写），与 R3"其余一切放行"相反，也无法实现 R4 的"cc 无感路径重定向"（cc 看到的必须仍是 `~/.claude` 默认路径）。
- 裁决：no（不可作为替代）；作为互补不冲突（沙箱开在本会话进程树内与 pasta 组合无依赖冲突），不在本项目范围内集成。

### 5.8 vopono（调研中发现的最近邻项目）
- Rust、per-app netns + 隧道配置管理（WG/OpenVPN + 众多商用 provider 配置文件）、DNS 处理完善、活跃维护（B: https://github.com/jamesmcm/vopono/blob/master/USERGUIDE.md ）。
- 结构性越界：netns/veth/防火墙 killswitch 需要 sudo（推荐 root daemon 常驻）——违反 R5/R6；它管理隧道生命周期——正是 R7 划出的边界。
- 裁决：no（不可复用），但作为 UX 参照与"per-app netns 已被验证受欢迎"的市场证据记录。

### 5.9 slirp4netns（provider 参照）
- 上游生态定位已被 pasta 取代：slirp4netns 处于维护模式、podman 5 默认 pasta（B: https://oneuptime.com/blog/post/2026-03-17-switch-slirp4netns-pasta-podman/view 旁证；RootlessKit 文档自荐序；passt(1) 与 podman-network(1)）。
- 无按接口名钉定（只有 `--outbound-addr` 按地址绑定），R7 契约 `if:<name>` 无法 1:1。
- 裁决：维持 ADR 0007 定位——trait 第二实现（T6），仅服务 jammy 等无 passt 包宿主的回退。

### 5.10 其他简评
- **nsjail**：囚禁型 + 容器惯例，同 5.6/5.7 缺口。no。
- **proxychains / LD_PRELOAD / env 代理**：R1 已拒（子进程可绕过），2026 年无变化。
- **`systemd-run --user` + `PrivateNetwork=yes`**：用户 manager 通常无权创建 netns（无 userns 包裹）；且 unit 名为具名持久状态——D1 已拒，无新证据翻案。
- **Landlock 网络（ABI 4，Linux 6.7+）**：只能按端口允许/拒绝 `connect()/bind()`，**没有"按接口钉定出口"原语**，不满足 R1；可作 P3 阶段纵深防御备选记录，不入 v1。

---

## 6. 为什么"每个候选都差一步"是结构性的而非巧合

各候选的失格点不是随机缺口，而是目标函数差异：容器系（podman/toolbox/distrobox/crun/youki/nsjail）的目标是"把任意 rootfs 跑起来"，因此 rootfs/存储/镜像状态是**特性**，恰是本项目 R3/R6 要消除的**缺陷**；囚禁系（firejail/claude-code sandbox/nsjail）的目标是"限制被执行方写坏宿主"，默认拒绝语义与 R3"其余一切放行"相反；隧道编排系（vopono、sshuttle 类）把隧道生命周期纳为己任，正是 R7 划出界的部分；底座系（RootlessKit/RootlessKit+dockerd）为守护进程而非单会话 CLI 设计，状态目录与 copy-up 复制模型与 D1/R3 相逆。iso-cc 的需求组合（钉定+透明共享+零状态+可验证）决定了它只能是"编排壳"，而这个壳没有任何上游愿意替你写——因为对别人它是 use-case，对 iso-cc 它是产品本身。

## 7. spec 既有拒绝记录复验（2026 上游现状对照）

### 7.1 D2 拒容器（root netns / cgroup 打标 / Docker）→ **仍成立**
- 2026 现状变化点：rootless 容器的 pasta 集成已成熟（podman 5 默认），容器路线的"网络能力"短板已消失——**但 D2 拒容器的真实理由是形态而非网络**：rootfs 复制与 R3 冲突、存储/镜像状态面与 R6 冲突、subuid+newuidmap 宿主前提与 R5 精神冲突、写集断言（T4）穿不透容器层（§5.1 逐条证据）。
- cgroup/fwmark 升格 `engine="mark"` 变体的修订不受影响：本扫描未发现新的"无宿主持久规则"的打标机制。
- 结论：**拒绝维持**，且新增一条论据（podman 生态恰恰把 pasta 当默认网关，说明网关类别正确、容器壳多余）。

### 7.2 D7 网关选 pasta（类别必须、实例择优）→ **仍成立，证据加强**
- pasta(1) 权威语义确认 R7 契约 1:1：`--outbound-if4 <name>` = "Bind IPv4 outbound sockets to host interface name"（B: https://man.archlinux.org/man/pasta.1.en ，检索 2026-09-26）；`-i/--interface` 派生地址/路由；`--dns-forward/--dns-host` 支撑会话内 DNS 拦截设计；`--map-host-loopback` 支撑 R3 宿主服务回访（本机 `--help` 实证，见 issue 09 证据）。
- 生态地位：podman 5 默认网关（B: podman-network(1)）；slirp4netns 维护模式（§5.9）；RootlessKit pasta 后端性能基准大 MTU 31.9 Gbps（B: rootlesskit docs/network.md）。
- 本机实证补强：spawn 模式 134ms（E5）、`--outbound-if4` 对 down 接口 fail-closed（E4）、attach 模式同日证实可用（issue 10 专项调查：TUNSETIFF 根因为 tap 命名撞名而非环境，proven invocation 含 ns 内 tap+默认路由+宿主 loopback 经网关地址双向+外部 egress 200；上游旁证 https://github.com/podman-container-tools/podman/discussions/22570 ）。
- 结论：**维持 pasta primary、slirp4netns 回退**；"自研用户态栈"的拒绝也维持（libslirp/passt 十年积累，无理由重造）。

### 7.3 D4 拒 v1 内置 tun2socks 自研（2026-09-26 修订：embedded SOCKS provider 为 SOCKS 形态候选）→ **v1 范围裁决仍成立；但"自研 300–500 行"的必要性被新组合路径部分稀释**
- 新发现：**`pasta + tun2proxy` 纯组合**可达 SOCKS 形态——pasta spawn 模式建 netns 并把出口钉在隧道接口（或以 `--map-host-loopback` 网关地址回访宿主 loopback-only 的 mixed-port），tun2proxy 在 netns 内起 TUN、把流量泵向上游 SOCKS。tun2proxy 为活跃上游（ADR 0007 后记已评估过其"面向 SOCKS 上游模型"的定位）。
- 与 embedded 自研 provider 对比：组合路径零自研代码、依赖多一个二进制、双层用户态栈（tun2proxy netstack + pasta netstack）吞吐/延迟劣化、DNS 拦截需 tun2proxy 侧配置；embedded 路径少一层、可控性强但要维护 glue。
- 结论：**D4 的 v1 拒绝维持**（不把 SOCKS 适配塞进 v1），但 D4 修订稿应把 `pasta + tun2proxy` 组合登记为 embedded 自研的**被否/待比替代**，择型时机不变（T3 之后按隧道形态）。"WG 跨 ns socket / env 代理"两项维持拒绝，无新证据。

---

## 8. 三选一裁决（正式）

> **裁决：薄编排成熟 CLI。**（形态上与"继续自研"中的 D5 路线兼容——差异仅在自研面的边界被本次扫描进一步收缩）

**理由**
1. **无轮子覆盖需求组合**（§4 表判）：逐项看，每个候选都恰好在 R1/R3/R5/R6 的某一个上结构性失格；组合不同轮子补位（如 RootlessKit+bwrap、podman+bwrap）只会叠加各自的状态模型，比单薄壳更重。
2. **网络面已被 pasta 单点覆盖**（A: E1/E4/E5 + B: pasta(1)）：R1/R7/R8 的原语层今天零自研可得，D7 选型经 2026 生态复验仍是最优实例。
3. **mountns 薄层不值得引轮**（A: E3/E6）：声明式 bind 列表 ≈ 10 条 mount syscall；bwrap/RootlessKit 都为这 10 条引入各自的状态/复制模型，负资产。
4. **产品价值全在编排壳**：config/doctor/verify/fail-loud/进程树生命周期/写集断言，在所有候选中零覆盖（claude-code sandbox 的 allowlist 是不同维度）。这正是自研的合理边界，也是 R1–R12 中最难复用外部的部分。
5. **已有代码非沉没成本绑架**：spec 变更（三）明言 session.rs 等不构成继续手搓的理由——本次扫描结论相反地支持保留现有路线，因为按 1–4，现路线（薄 Rust 壳 + pasta/slirp trait）**就是**"薄编排成熟 CLI"的直接实现；真正要改的是叙事与边界（见下）。

**对 D5 的边界修订建议**（不改需求正本，落实现票时生效）
- 自研面收缩为：TOML 解析、doctor、`--print-plan`、进程树生命周期（PDEATHSIG/收割/exec RPC）、mountns bind 序列、env 注入。**usernet 面与 uid_map 面视作外部依赖**（pasta 已内建 userns 映射，E2），`--print-plan` 的展开终点从"等价 syscall 序列"放宽为"等价 CLI 组合序列"（pasta 实参 + bind 清单）。
- **建议①（供 issue 09/12 参考，非本票职责）**：attach 形态已无阻塞（issue 10 根因 = tap 默认命名撞 outbound 接口名，passt.1:654-657；省略 flag 或配 `-I` 即稳定成功），spawn 与 attach 均为可选形态，差异只剩设计权衡（生命周期根归属 iso-cc 还是 pasta）。无论择哪形态，建议 09 票 doctor/plan 层新增一条 fail-loud 断言：**展开后的 pasta 实参中 ns-ifname 不得与目标 ns 既有接口名冲突**（等价于"egress 接口名若可能与 ns 内接口撞名必须自动补 `-I`"）；"宿主 loopback 可达"语义应走默认网关地址映射 / `--map-host-loopback`，绝不以 `--outbound-if4 lo` 实现。
- **建议②（供 D4/T3 后择型参考）**：SOCKS 形态择型时把 `pasta + tun2proxy` 组合与 embedded 自研 provider 并列比选（§7.3）。

**被否替代（裁决对比）**
- ~~直接复用现成工具~~：无候选全绿（§4）；最接近的 podman 仍带 rootfs/存储状态/subuid 前提三座山。
- ~~继续自研（D5 原义：连 userns/uid_map/netns 原语都自己拼）~~：原语面自研是负价值——pasta 把 userns+netns+usernet 打包成一个无特权进程，重写它没有收益只有边界协议（PMTU/半关闭/ICMP 语义）风险。
- ~~RootlessKit 作为编排底座~~：copy-up 复制语义 + state-dir 状态模型与 D1/R3 相逆。
- ~~claude-code 原生 sandbox 作为实现~~：域名 allowlist + fail-open 默认 + 囚禁文件面，与 R1/R8/R3 全部反号。

---

## 9. open_questions

1. ~~pasta attach 模式 TUNSETIFF 与 spawn 模式的关系~~ **已解决**（2026-09-26）：根因 = passt 默认以 outbound 接口名命名 ns 内 tap，与 `lo` 撞名触发内核 `tun.c:2711-2720` attach-existing EINVAL；非时序/权限/WSL2 问题。残留的开放点降级为设计权衡：spawn（pasta 为根，天然满足 D1 进程树）vs attach（iso-cc 控根）最终择一，及撞名断言入 doctor/plan 的归属票（09/12）。
2. spawn 模式下 pasta 后台化行为与 `list` 的会话发现：默认 fork 到后台（无 `-f`）+ syslog；`-f` 前台时 iso-cc 是父进程。`list` 扫 `/proc` 识别"会话根 = pasta 进程"的指纹是否稳定（cmdline 含 `--outbound-if4` 等可作标记）？需在 09 号票定案。
3. `pasta + tun2proxy` 组合的 DNS 与双层 netstack 开销实测：DNS 应由 tun2proxy 虚拟 DNS 还是 pasta `--dns-forward` 链路承接？性能是否劣化到不可接受？（归属：D4 择型时的 T3 后评估）
4. `engine="mark"` 变体（D2 修订）与本次结论无冲突，但其"规则丢失静默 fail-open"在 doctor 侧的 fail-loud 断言粒度（表存在 vs 表内规则逐条 hash）未定——本扫描未发现 2026 年新原语可消除该弱点。
5. bwrap 是否值得作为 P3 备选挂载面保留（如未来需要 per-path 只读化/`--chmod` 类特性）：当前结论是不引入，暂无触发条件。
6. RootlessKit 的 pasta 后端自 2023-12 引入至 v3.2.0 仍标 experimental：若未来 iso-cc 需要"netns 外代执行 host_exec"类能力，RootlessKit 的 port-driver/implicit 模型可作参考，但无近期迁移理由。

---

## 10. 来源清单

**一手文档 / man page / 官方 release**
1. pasta(1)/passt(1) man page（flags 语义：`--outbound-if4/-i/--dns-forward/--dns-host`，attach 与 spawn 模式）— https://man.archlinux.org/man/pasta.1.en （检索 2026-09-26；上游 passt.top/passt.git）
2. passt 上游静态构建（jammy 可用单文件）— https://passt.top/builds/latest/x86_64/ ；本机 nixpkgs passt-2026_07_16.090d739
3. podman-network(1)（rootless 默认 pasta；`--network=pasta:<opts>` 透传；containers.conf `pasta_options`）— https://docs.podman.io/en/stable/markdown/podman-network.1.html
4. podman rootless tutorial（subuid/subgid、newuidmap、`~/.local/share/containers` 存储状态）— https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md
5. podman-run(1) `--rootfs` — https://docs.podman.io/en/stable/markdown/podman-run.1.html
6. distrobox-create（默认 host 网络；`--unshare-netns` 行为）— https://distrobox.it/usage/distrobox-create/ ；Fedora 讨论（toolbox/distbox 网络隔离缺口）— https://discussion.fedoraproject.org/t/network-isolation-when-using-toolbx-or-distrobox/129932
7. RootlessKit docs/network.md（`--net=pasta` experimental、`--net=slirp4netns` recommended、2026-07 基准表）— https://github.com/rootless-containers/rootlesskit/blob/master/docs/network.md ；仓库 README（`--copy-up` tmpfs+symlink 语义、`--state-dir`）— https://github.com/rootless-containers/rootlesskit
8. firejail(1) — https://man7.org/linux/man-pages/man1/firejail.1.html ；CVE-2022-31214 oss-security 披露 — https://www.openwall.com/lists/oss-security/2022/06/08/10 ；release notes — https://firejail.wordpress.com/download-2/release-notes/
9. Claude Code sandboxing 官方文档（bwrap+socat、allowedDomains、依赖缺失回落）— https://code.claude.com/docs/en/sandboxing ；Anthropic 工程博客 — https://www.anthropic.com/engineering/claude-code-sandboxing
10. slirp4netns 维护模式与 pasta 生态迁移旁证 — https://oneuptime.com/blog/post/2026-03-17-switch-slirp4netns-pasta-podman/view （C 级）+ RootlessKit docs（B 级，同 7）
11. vopono USERGUIDE（sudo/root daemon 模型、隧道配置管理边界）— https://github.com/jamesmcm/vopono/blob/master/USERGUIDE.md
12. tun2proxy 上游 — https://github.com/blechschmidt/tun2proxy （ADR 0007 后记已评估其定位）
13. release 时间戳（GitHub API，2026-09-26 查询）：rootlesskit v3.2.0（2026-09-19）、bubblewrap v0.13.0（2026-09-22）、crun 1.30.1（2026-09-25）、youki v0.7.0（2026-07-25）

**本报告 A 级实验**：见 §2；实验脚本均为一次性命令，未入库。
**内部交叉引用**：docs/DEPENDENCIES.md §3/§5（"不需要 bwrap/unshare 二进制"的原判断与本扫描 E3/E6 一致）；docs/adr/0007-usernet-gateway-pasta.md；.scratch/iso-cc/issues/04、09、10。
