# iso-cc Spec（v1）

> 正本来源：docs/REQUIREMENTS.md（R1–R12/N1–N6/D1–D8）+ ../iso-cc-research/ 5 份研究。本文件是 agent 可执行的收敛版；冲突时以 REQUIREMENTS 为准。

## Problem Statement

同一台 Linux 机器上，Claude Code 的网络出口身份、时区/locale、与宿主 cc 的状态（credentials/sessions/trust）互相污染；环境变量代理是 best-effort，子进程可绕过；root 容器方案破坏原生开发体验（localhost、文件系统、Docker）。

## Solution

rootless 轻量会话：userns+mountns+netns + 用户态网关（pasta）钉死出口到宿主已存在的 L3 接口；mountns 声明式 bind 覆盖 locale 文件与 CC 状态路径（cc 无感）；TZ/LANG env 注入。会话 = 进程生命周期，零宿主持久状态。可选范围 `net.scope=tree|self`（self：bash -c 整段 RPC 转发宿主执行）。

## User Stories

1. As a developer, I want `iso-cc run -- claude` so that cc's API traffic exits via my tunnel while my desktop stays untouched.
2. As a developer, I want all child processes of cc to inherit the egress, so that no tool call leaks my real IP.
3. As a developer, I want fail-closed behavior, so that a dead tunnel means no traffic, never a silent fallback.
4. As a developer, I want TZ/locale declared per profile, so that cc's scheduling and Intl surfaces match my declared region.
5. As a developer with multiple cc installs, I want sandbox cc state under a per-profile directory, so that host cc and sandbox cc never pollute each other.
6. As a developer, I want the sandbox cc to see its default `~/.claude` paths, so that cc behaves identically to a native install.
7. As a developer, I want my filesystem/HOME/docker socket/SSH agent shared, so that development feels native.
8. As a developer, I want dev servers started by the agent reachable at host localhost, so that my browser workflow is unchanged.
9. As a developer, I want host-loopback services (DB, localhost MCP) reachable from the session, so that integration testing works.
10. As a developer, I want `verify` to red/green the environment, so that I can trust cc's observation surfaces before working.
11. As a developer, I want `doctor` to diff config vs reality, so that drift is caught before launch.
12. As a developer, I want `--print-plan`, so that every privileged-adjacent action is inspectable before it happens.
13. As a developer, I want no `up`/`down`, so that nothing can dangle after a crash.
14. As a developer, I want tunnels to stay outside the tool, so that any WireGuard/sing-box/mihomo setup works unchanged.
15. As a developer, I want multiple concurrent sessions, so that parallel agent work shares one egress without interference.
16. As a distro-hopper, I want a single static binary plus pasta, so that any glibc/musl distro works identically.
17. As a developer, I want `net.scope=self`, so that bash tool commands run host-side with full localhost while cc's own identity stays in the sandbox.
18. As a developer, I want sessions killed by Ctrl-C to leave zero host residue, so that I never debug stale state.

## Implementation Decisions

- D1 stateless sessions（进程生命周期）；D2 rootless pasta（engine 变体 `mark` 后置）；D3 agent 宿主管理+声明式入口；D4 egress=接口引用（SOCKS 形态 embedded provider 候选）；D5 Rust musl + provider trait；D6 CC 配置 = mountns 路径重定向（拒绝 CLAUDE_CONFIG_DIR/HOME overlayfs）；D7 网关选型；D8 net.scope 声明（self 转发机制见 ADR 0008 附/附2/附3）。
- 拦截分层 L1 SHELL / L1.5 CLAUDE_CODE_SHELL_PREFIX / L2 PATH / L3 bind bash inode / L4 per-agent 清单（ADR 0008 附 2/附 3）。
- 依赖纪律：版本号由 `cargo add` resolver 决定，禁止手写；Cargo.lock/flake.lock 入库；持续卫生 = dependabot + cargo-deny（对齐 oh-my-pi deny.toml 模式，T7）。

## Testing Decisions

- 唯一高层测试缝：`iso-cc` CLI（`run --print-plan` / `doctor` / `verify` / `list`）。探针实现与其共享（verify = 产品化的测试矩阵）。
- 探针 P1–P15 + P3b（纯净度）、R12 子矩阵 12 条（浏览器，P3）、工具面 P16–P27（MCP/hooks/IDE/OTel）——见 ../iso-cc-research/ 三份研究。
- 状态不变式：宿主 diff（mountinfo/route/ruleset/lsns/~/.claude*）为零（有界例外登记）。
- 引擎无关：探针不感知 engine=netns|mark。

## Out of Scope

反取证/风控承诺；隧道生命周期管理；囚禁型沙箱；v1 socks 透明重定向；nsswitch 防御性 bind（第二批，P3b 检测先行）；嵌套 claude 转发例外（延后）。

## Further Notes

宿主矩阵实测记录在 docs/DEPENDENCIES.md；工具面取证基线 = claude-code 2.1.263（版本漂移按 T5 登记流程处理）。

## 变更记录

### 2026-09-26（一）：资源两级模型（替代"绝对 rootless stateless"）

**Requirement**：用户裁决——不介意 rootful 与持久化，条件是（a）rootful 只允许出现在可 install/setup 的声明阶段（一次性、幂等、显式审计），绝不允许每会话 sudo；（b）持久化资源状态必须被定义且回收有保证。

**Specification**：资源分两类 + 一个不可接受态：
- `session-scoped`：随会话进程树消亡（netns、tap、mounts、网关进程）
- `setup-manifested`：`iso-cc setup` 创建，清单登记（类型/路径/创建记录），`iso-cc gc`/`uninstall` 精确按清单回收，doctor 双向校验（清单→现实漂移、现实→清单未登记物）
- `residue`：既不随会话消亡又未登记回收 = 唯一不可接受态

被否替代：维持绝对 rootless（实测迫使每会话 hack：/etc 预创建、PATH 解析网关、杀路径竞态——即用户所指"cosplay stateless"）。R5/R6/N3/D1 措辞修订由设计稿提出行级提案（parent 落 docs/REQUIREMENTS.md）。

**Acceptance**（并入设计稿与后续票）：每个宿主持久物可归类两级之一；residue 集合可被工具枚举且为空；setup 幂等重跑 diff 为零；gc 后清单与现实一致。

### 2026-09-26（二）：Statelessness & Fragility Audit（设计稿强制章节）

**Requirement**：设计稿必须用代码证据硬审计"rootless stateless"主张，禁止叙事自证。

**Specification**：逐条对照 R6/D1/N3 标注 `session-scoped / setup-manifested / residue` 三级 + 代码行号证据；必查嫌疑清单（PDEATHSIG 竞态与 daemon 孙进程持 netns、uid_map 写失败模式、/etc 预创建残留、PATH 解析网关、网关中途死静默断网、state_dir 积累、slirp DNS 不走 egress、locale 仅 TZ env）；每项修复归属 09/11/12 或新票。

**Acceptance**：审计表完整覆盖上述嫌疑；每个非 session-scoped 项有归属票或 open_question。

### 2026-09-26（三）：实现手段开放性（step back and look around）

**Requirement**：需求正本（R1-R12）不动，但实现手段完全开放。任何实施票开工前必须先回答三个问题：①有没有现成工具/项目已满足该需求（build-vs-reuse 裁决）；②该能力有没有维护中的现成组件（CLI 或 crate）；③用成熟 CLI 组合是否优于自研 Rust 组件。禁止在未登记"轮子盘点"的票上写实现。

**Specification**：全项目级复验已有裁决的时效性——D2 对 Docker/容器、D7 对网关选型、D4 对 tun2socks 自研的拒绝理由必须对照 2026 当前的上游现状重新验证（podman rootless+pasta、bwrap、RootlessKit、claude-code 原生 sandbox 等），确认拒绝仍成立才可沿用手写路线。已有代码（session.rs 等）视为沉没成本，不构成继续手搓的理由。

**Acceptance**：alternatives 扫描报告落 `.scratch/iso-cc/research/` 并给出三选一裁决（直接复用 / 薄编排成熟 CLI / 继续自研）+ 逐条证据；每个实施票含"轮子盘点"小节。

### 2026-09-26（四）：身份范围与 bash 执行位置解耦（修正 D8/US17 捆绑）

**Requirement**：`net.scope=self` 把两件正交的事捆成了一个原子声明（cc 本体进 netns + bash 工具宿主侧执行）。两轴必须可独立声明。

**Specification**：两条正交轴——
1. `net.scope = tree | self`：**身份范围**。tree = 整棵进程树进 netns（US2 一致性）；self = 仅 cc 本体进 netns。
2. `exec.bash = sandbox | host`：**bash 工具执行位置**。sandbox = 会话内执行（继承 netns，US2 满满但 localhost 指向会话自身）；host = 经 exec RPC 通道宿主侧执行（US9 真 localhost；代价 = bash 工具流量走宿主出口，身份一致性让位——显式声明换取，不是静默）。

组合语义：`tree+sandbox`（现状默认）；`tree+host`（身份钉定 + 集成测试真 localhost——bash 流量走宿主出口是声明性代价）；`self+host`（原 self 全语义）；`self+sandbox`（合法但少见）。verify 语义按组合收窄：P1/P13–P15 恒测 cc 本体出口；host 组合下 bash 面不在出口断言范围（探针经 netns fd setns 保持可测，ADR 0008 附⑥）。

机制正本 = ADR 0008 附/附2/附3（exec.sock RPC、拦截分层 L1/L1.5/L2/L3/L4、multi-call shim、cc 2.1.263 工具面取证：hooks/REPL/statusline 硬编码 /bin/sh，L1.5=CLAUDE_CODE_SHELL_PREFIX 优先验证）。

**Acceptance**：config 出现两独立声明面；`--print-plan` 展开组合；票 15 落地后自证探针（会话内 exec 标记二进制 → 断言落点宿主侧/沙箱侧与声明一致）。

### 2026-09-27（五）：引擎重审——路由表方案从"已拒绝"升回候选（触发条件已实际触发）

**Requirement**：用户连续质疑（--publish 为何存在 / 为何不是路由表方案 / 不是不要 netns 吗）= D2 预设的 mark 引擎触发条件（netns localhost 摩擦）实际触发。且变更（一）的两级模型移除了当初拒绝 B 方案的全部实质理由（root → setup 许可；持久规则 → 清单+gc+doctor 可管理）。

**Specification**：引擎改为双候选并存、按 config `engine` 声明——
1. `engine = "netns"`（已实现）：pasta spawn + tun2socks 组合；强项 = R8 结构性 fail-closed（pidns 整树杀）、端口空间隔离、v6 语义；弱项 = localhost 需桥接（17 的 auto 转发）
2. `engine = "mark"`（待实验，原 D2 降格方案的 uid 路由变体）：setup 一次性安装 uidrange/ip-rule 策略路由 + 表内黑洞兜底（fail-closed）+ 专用 uid；会话零 netns，**localhost 双向零摩擦**；socks 隧道形态下 mihomo 直接 TUN 化，省 tun2proxy；强项 = US8/US9 结构性成立；弱项 = 原语持久面（清单+doctor+gc 管理）、端口空间与宿主共享
3. 裁决依据 = 票 18 实验（mark 引擎全链本机实证：setup 装规则 → 无 netns 会话 localhost 双向 + 出口走 mihomo TUN → fail-closed 断网取证 → gc 回收干净）

**Acceptance**：两引擎各有可跑通的验收矩阵；`--print-plan`/doctor/verify 引擎无关（R12 前置）；用户按环境择引擎。
