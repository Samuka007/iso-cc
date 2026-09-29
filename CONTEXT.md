# iso-cc

一句话定位：在任意 Linux 发行版上，以 rootless、声明式、零持久状态的方式，为 Claude Code（或任意命令）拉起「网络出口定向 + locale 覆盖 + CC 配置隔离」的轻量会话；其余一切放行。

## 词汇表（唯一权威定义）

| 词 | 定义 |
|----|------|
| **会话 (session)** | 一次 `run` 产生的全部东西：一个进程树 + userns/mountns/netns 三个命名空间 + 一个 usernet 子进程。生命周期 = 进程生命周期，无名字、不落盘、kill 即拆除。 |
| **profile** | config 里的声明单元：egress + locale + CC 配置隔离三组参数。会话由 profile 派生，永远可以从 config 重算。 |
| **egress** | 网络出口的**引用**（`if:wg0` 形态）。工具只消费接口，不创建、不管理隧道。 |
| **usernet provider** | 用户态网络网关：pasta（首选）/ slirp4netns（回退）。每个会话一个，几 MB RSS，不是隧道。 |
| **egress 两形态** | `if:<接口名>`（默认，pasta 以该接口为 outbound）\| `socks5://<host>:<port>`（netns 内 tun2proxy worker 在 tun1 上 0/1+128/1 劫持默认路径，流量翻译为 SOCKS5 CONNECT 送经宿主 SOCKS5 服务）。择型证据 docs/REQUIREMENTS.md + /tmp/iso-cc-exp16。 |
| **exec.bash 轴** | `host`（默认，2026-09-28 操作者裁定）：bash 工具/hooks/MCP 经 exec.sock RPC 宿主侧执行（US9 真 localhost）\| `sandbox`（显式）：bash 留在会话内（netns 路由面 + mountns 视图）。 |
| **拦截分层 (L1/L2/L3)** | host 模式把 cc 的各执行面导入会话 shim（multi-call：iso-cc 自身按 argv0 basename 分派）。L1 = `SHELL` env；L2 = PATH 解析；L3 = mountns bind 经典路径（`/bin/bash`、`/usr/bin/bash`、`/bin/sh`）。L1.5（`CLAUDE_CODE_SHELL_PREFIX` 单载荷）已取消注入，由 L3+L1 等价承接（ADR 0008 附 4 + 票 26）。 |
| **值面经典化** | 拦截所需的 env 值全部写经典路径（`/bin/bash`）——L3 bind 使该路径**就是**会话 shim，cc 风控视角零可辨识特征。禁注入任何 `<sess>/...` 会话路径值（操作者裁定 2026-09-28）。 |
| **落点 (landing)** | 命令最终执行的 namespace 归属。host 落点 = shim→RPC→宿主 worker（宿主 netns/mountns，US9 真 localhost）；netns 落点 = 会话内原生执行（出口走会话隧道，身份一致）。非 shell 裸名命令（npx 等）= netns 落点（L2 逐名种子不可穷尽，已撤）。 |
| **tun2proxy worker** | socks5 形态的 netns 内翻译器：tun1 上收 IP 包 → SOCKS5 CONNECT 送经 tap0 网关 → 宿主 SOCKS5 服务。生命周期 = PDEATHSIG→bootstrap，fail-closed。上游无 apt 渠道，分发 = iso-cc 仓镜像组件（静态 musl，sha256 钉定）。 |
| **doctor** | `iso-cc doctor`：配置意图 vs 宿主现实的全量 diff 校验（接口、zoneinfo、locale、provider、userns sysctl、claude 安装探测）。 |
| **泄露 (leak)** | 会话内任何数据包（TCP/UDP/DNS/QUIC/IPv6）未经 egress 离开。唯一的失败模式是断网（fail-closed），不是回落直连。 |

## 架构不变式

1. **无持久状态**：不 `up`/`down`，不命名 netns，不写 nftables/iptables 规则，不改宿主路由。存在具名持久状态的地方就是漂移的温床。
2. **声明式**：config 是唯一输入；有效状态可从 config 重算；不存在改写持久状态的动词。
3. **隧道出界**：隧道协议（WireGuard/sing-box/mihomo）是别人的事；本项目契约只认「宿主上存在一个 L3 接口」。代理形态（SOCKS/HTTP）隧道必须由隧道侧自己出 TUN。
4. **最小侵入**：不 clearenv、不隔离 /proc、不复制 rootfs、不隔离 /etc——只 bind 覆盖 locale 两个文件 + CC 状态路径（`~/.claude`、`~/.claude.json`，声明式重定向，cc 无感）+ 三四个环境变量。

## 决策记录

见 `docs/REQUIREMENTS.md` §决策记录（D1–D5）。硬-to-reverse 的决定升级为独立 ADR。

## 环境

- 开发：NixOS (WSL2)，`flake.nix` devShell 提供 Rust musl 工具链 + pasta + slirp4netns。
- 首个目标宿主：chenyizi-4090（Ubuntu 22.04.2 / 5.19 / userns 开启 / 无 AppArmor 限制 / jammy 无 passt 包）。
