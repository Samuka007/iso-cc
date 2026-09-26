# iso-cc Spec（v1）

> 正本来源：docs/REQUIREMENTS.md（R1–R12/N1–N6/D1–D8）+ docs/research/ 5 份研究。本文件是 agent 可执行的收敛版；冲突时以 REQUIREMENTS 为准。

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
- 探针 P1–P15 + P3b（纯净度）、R12 子矩阵 12 条（浏览器，P3）、工具面 P16–P27（MCP/hooks/IDE/OTel）——见 docs/research/ 三份研究。
- 状态不变式：宿主 diff（mountinfo/route/ruleset/lsns/~/.claude*）为零（有界例外登记）。
- 引擎无关：探针不感知 engine=netns|mark。

## Out of Scope

反取证/风控承诺；隧道生命周期管理；囚禁型沙箱；v1 socks 透明重定向；nsswitch 防御性 bind（第二批，P3b 检测先行）；嵌套 claude 转发例外（延后）。

## Further Notes

宿主矩阵实测记录在 docs/DEPENDENCIES.md；工具面取证基线 = claude-code 2.1.263（版本漂移按 T5 登记流程处理）。
