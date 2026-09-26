# 环境合格性 canary/mock/stub 现成方案调研

日期：2026-09-26 ｜ 状态：研究笔记（info-only，不改需求基线 `docs/REQUIREMENTS.md`）
前置：`docs/research/2026-09-26-related-projects-and-purity-probes.md`（P1–P12 探针清单、cc 环境行为取证）
方法：官方侧全部溯源到一手来源（官方文档、GitHub 源码逐行、npm 产物本体、本机 2.1.263 实测运行）；社区侧逐个查证仓库/npm 元数据。无法溯源标 [未验证]。

---

## TL;DR

**不存在**任何"弄好环境、跑一个探针、判定能不能放心用 claude code"的现成方案——官方（doctor / srt / devcontainer 自校验 / npm 包）与社区（coding-agent-doctor / dnsleaktest 族 / VPN kill-switch 族）各有覆盖一小块，但没有一个断言 iso-cc 的组合保证面（egress 钉死 + locale 双视线覆盖 + CC 配置重定向 + 宿主零状态）。`iso-cc verify` 把 P1–P12 打包成自校验脚本就是合理终点，形态直接照抄官方 `init-firewall.sh` 的"正/反冒烟内嵌 + exit 1"模式；其中 P9（会话内跑真 `claude doctor`）恰好就是用户设想的"mock 过了再跑真身"的最后一环。

---

## 一、官方检查面盘点

### 1.1 `claude doctor` / `/doctor`：安装健康度，无环境纯净度

两路证据交叉确认检查面：

**文档面**（debug-your-config / troubleshoot-install）[来源 1, 2]：
- `/doctor`（会话内）：installation health、invalid settings files、unused extensions、同目录重复 subagent 名、可从代码推导的 CLAUDE.md，附修复建议。
- `claude doctor`（终端版）：只读诊断，读当前目录 settings 文件不需 trust prompt。
- 扫描 shell 配置文件找过时 alias：`~/.zshrc`、`~/.bashrc`、`~/.config/fish/config.fish` + macOS bash profile 序列；`ZDOTDIR` 生效时扫 `$ZDOTDIR/.zshrc`（v2.1.214 前路径为目录会挂起）[来源 2，原文引用]。
- **检查面中没有任何网络出口一致性、时区/locale 一致性、环境泄漏检查**。文档里最接近"网络"的是安装排障的手工指引：`curl -sI https://downloads.claude.ai/claude-code-releases/latest` 看返回码——那是教用户手动跑的探针，不是自动化 canary [来源 2]。

**实测面**（本机 2.1.263，`claude doctor` 实际输出）[实测]：
```
Running: native (2.1.263) / Commit / Platform: linux-x64 / Path
Config install method / Search: OK (ripgrep 路径)
Auto-updates / Auto-update channel / Last update attempt
Managed settings (remote) / Organization policy
Remote Control 段（API 连接、claude.ai 登录、scope、org、feature-flag）
1 warning: ~/.local/bin 不在 PATH（附修复命令）
```
全部条目为安装/配置/凭据/连通性登记，零环境纯净度断言。

**结论**：doctor 判定"cc 装好了、配置合法"，不判定"cc 跑在哪个出口、哪个时区、哪个 HOME"。与用户设想的 canary 是两个问题域。

### 1.2 `@anthropic-ai/sandbox-runtime`（srt v0.0.77）：无 validate 命令，有 fail-loud 哲学

一手源码证据 [来源 3, 4]：
- `package.json`：唯一 bin `srt` → `dist/cli.js`。
- `src/cli.ts` 全文（590 行）通读：子命令仅 `windows-install`、`windows-uninstall` + 默认包装命令（flags：`--debug`、`--settings`、`-c`、`--control-fd`）。**没有 `validate`/`doctor`/`check` 类命令**。
- **配置加载 fail-loud 语义**（`refuseSettings()`，源码逐字）：settings 文件存在但为空/不可读/校验失败 → 报错 `exit 1`，明确拒绝回退内置默认（"falling back would drop the file's `denyRead`, `allowRead` and credential rules"）；仅"文件不存在"才是合法的默认配置请求。`--settings` 指定的文件不存在同样拒绝。
- `initialize()` 硬失败面：Windows WFP fence 未就绪即报 actionable error；root 调用者缺 `CAP_SETFCAP` 时 bubblewrap uid map 写失败、initialize 拒绝启动 [README 原文]。

**可偷的形态**：srt 的"环境不合格就拒绝启动、绝不静默降级"正是 `iso-cc verify` 该有的退出码哲学；但它校验的是 **srt 自己沙箱的可用性**（bwrap/seccomp/WFP/配置），不校验出口 IP、DNS 路径、locale、TZ——它的网络模型是 allowlist 代理，与我们的 egress 钉死不同构。

### 1.3 官方 devcontainer `init-firewall.sh`：唯一现成的"自校验脚本"范式

一手文件逐字确认 [来源 5]（脚本收尾，136 行中的最后一段）：
```bash
if curl --connect-timeout 5 https://example.com >/dev/null 2>&1; then
    echo "ERROR: Firewall verification failed - was able to reach https://example.com"
    exit 1
fi
if ! curl --connect-timeout 5 https://api.github.com/zen >/dev/null 2>&1; then
    echo "ERROR: Firewall verification failed - unable to reach https://api.github.com"
    exit 1
fi
```

要点：
- **形态价值大于内容价值**：自校验不是独立 test 文件，而是内嵌在建立隔离的脚本末尾——隔离建立即自证，两条探针一"反"（期望失败）一"正"（期望成功），失败即 `exit 1`。这正是 P12 已吸收的模式。
- **能否独立于容器复用？** 两条 curl 是一行命令，任何地方都能抄；但它们的**断言语义绑定容器内的 default-deny iptables**（"example.com 必须失败"只在 OUTPUT DROP 的语境下成立）。搬到 iso-cc 语境要换断言对象：反例从"打不通 example.com"换成"`curl -4` 的出口 IP ≠ 宿主 IP"（P1）+ "`curl -6` 必须失败"（P2），正例从"api.github.com 可达"换成"必达域名经 egress 可达"（doctor 的种子域名表）。**复用的是模式，不是工具**。
- 覆盖面对照：2 条探针 ≈ P1/P2 的极简版，无 DNS 钉死（脚本自己 DNS :53 全放行）、无 v6、无 locale/TZ、无写集。

### 1.4 `@anthropic-ai/claude-code` npm 包：无自检入口

一手产物证据 [实测拉取 registry + tarball]：
- latest 2.1.283，tarball 仅 7 个文件：`cli-wrapper.cjs`、`install.cjs`、`bin/claude.exe`、`package.json`、`LICENSE.md`、`README.md`、`sdk-tools.d.ts`。
- `postinstall: node install.cjs` 是下载器；无任何 doctor/probe/diagnose 附加脚本。
- 本机 `claude --help` 全量 flags [实测]：无 `--diagnose`/`--check-env` 类入口；最接近的是 `doctor` 子命令（见 1.1）与 `--safe-mode`（排障用降级模式，非校验）。二进制内 `doctor` 相关字符串仅有 `Doctor/DoctorInfo/DoctorMain/DoctorRedirectMessage` [实测 grep]，与文档检查面一致，无隐藏的环境检查分支。

### 1.5 官方面小结

| 官方入口 | 校验对象 | 覆盖 P1–P12 | 角色 |
|---|---|---|---|
| `claude doctor` / `/doctor` | 安装健康度 + settings 合法性 + shell rc | 0 条（仅 P9 里当**被跑对象**） | canary 的最后一段：真身自检 |
| srt `initialize()` | srt 自身沙箱可用性 + 配置 fail-loud | 0 条 | 退出码/拒绝降级哲学的范本 |
| init-firewall.sh 收尾自校验 | 容器 default-deny + allowlist 可达 | ≈P1/P2 的极简版（语义需重写） | **自校验内嵌形态的范本** |
| npm 包 install.cjs | （下载安装） | 0 条 | 无自检入口 |

---

## 二、社区工具清单

GitHub/npm 检索（2026-09-26，多组关键词族：claude sandbox check / environment probe / readiness / doctor / leak test；vpn leak / dnsleak / quic check），逐个核验仓库与 npm 元数据：

| 工具 | 形态/覆盖面 | 活跃度 | 与 P1–P12 的重叠 | 缺口 |
|---|---|---|---|---|
| `coding-agent-doctor`（npm 0.3.0，MIT）[来源 6] | `agent-doctor` CLI，离线只读跨厂商（claude/codex/cursor）健康检查：重复安装、invalid JSON/TOML、MCP command/runner 解析、MCP 层间漂移、MCP 未设 env key、敏感字段（脱敏）；`--json/--markdown/--sarif`；exit 0/1/2/3 | 2026-08-19 首发，v0.3.0，单人维护 | **理念最近**（"跑一个独立 doctor 再信任环境"），输出/退出码规范可直接参照 | 检查面全部在配置层，0 条网络/locale/TZ/写集断言 |
| `ecc-agentshield`（npm 1.6.0，affaan-m/agentshield）[来源 7] | agent 配置安全审计（漏洞/误配/注入风险） | 2026-02 起 | 配置安全维度（威胁模型面） | 非环境合格性 canary [其详细检查项未逐条核验：未验证] |
| `macvk/dnsleaktest` | bash 脚本，ipleak.net 解析器测 DNS 泄漏 | 584★，2026-09-15 推送，活跃 | ≈P3 的简化版（DNS 路径泄漏） | 无 egress IP/v6/QUIC/locale/写集；非 cc 感知 |
| `orhun/dnsleaktest-tui` | DNS 泄漏 TUI（PoC） | 62★，2024-10 | P3 部分 | 同上；TUI 形态不适合 CI |
| `code3-dev/dnsleak`（Go，32★，2025-10）、`mschwager/dnsleak`（21★，2014 停更）、`emanuele-f/DNSleak`（16★，2017） | DNS 泄漏检测器（本地/远端两路） | 多数停滞 | P3 部分 | 同上 |
| `Maroka-chan/VPN-Confinement` | NixOS module：systemd 服务流量走 VPN + 防 DNS 泄漏 | 249★，2026-09-14，活跃 | **预防侧**先例（服务级 egress 钉死，声明式），非测试工具 | 是囚禁/定向机制，不是 canary；NixOS 专属 |
| `wknapik/vpnfailsafe`（148★，2018 停更）、`adrelanos/vpn-firewall`（178★，2019 停更） | OpenVPN kill-switch（泄漏**预防**） | 停更 | 预防侧（fail-closed 思想的 iptables 实现） | 无验证探针；需 root；OpenVPN 专属 |
| `franccesco/mullpy`（9★）、`hgrahamcs/amimullvad`（3★） | Mullvad 连接状态检查薄包装（打 `am.i.mullvad.net`） | 低 | ≈P1 的单条 | 官方端点 + 一行 curl 即可，无独立价值 |
| GitHub `http3checker` 族（≤9★ 多个） | "服务器是否支持 HTTP/3" | 低/停滞 | 无（方向反了：测服务器不测本机泄漏） | 本机 QUIC 泄漏检测仍是 `curl --http3-only cloudflare-quic.com`（P4），无现成封装 |
| `CaptainMcCrank/SandboxedClaudeCode`（57★） | bwrap/firejail/Apple Container 跑 cc | 2026-03 | README 内嵌**手工**验证步骤（GPG/SSH agent 冒烟） | 手工清单非自动化 canary；无网络/locale 面 |
| 其余 cc 沙箱包装（textcortex 已归档、neko-kai、pvillega、jonn-smith/claude-docker-sandbox 等） | 囚禁型包装 | 参见前篇 Q1 | 无 | 无一家内置环境自校验脚本 |

**横向结论**：社区工具要么窄（DNS 泄漏一族只覆盖 P3）、要么错位（配置医生/安全审计不碰环境面）、要么是机制不是测试（VPN-Confinement/kill-switch）。**没有任何工具把"出口 + 时区 + locale + cc 配置 + 宿主状态"当作一个合格性整体来断言。**

---

## 三、与 P1–P12 覆盖对照

| 探针 | claude doctor | srt | init-firewall 自校验 | coding-agent-doctor | dnsleak 族 | 现成合计 |
|---|---|---|---|---|---|---|
| P1 v4 出口 IP == egress | – | – | 半（无 IP 对照，仅可达性） | – | – | **半条** |
| P2 v6 必须失败 | – | – | –（脚本自身无 v6） | – | – | 0 |
| P3 DNS == 声明解析器 | – | – | –（脚本 :53 全放行） | – | ≈（泄漏检测，无钉死断言） | **半条** |
| P4 QUIC fail-closed | – | – | – | – | – | 0 |
| P5 拔线 fail-closed | – | – | – | – | – | 0 |
| P6 TZ/locale 四视线一致 | – | – | – | – | – | 0 |
| P7 IP-geo 时区交叉 | – | – | – | – | – | 0 |
| P8 cc 写集 ⊆ 重定向集 | –（doctor 只读，非写集审计） | – | – | – | – | 0 |
| P9 会话内 doctor/status 对照 | 被跑对象本身 | – | – | 部分（跨厂商读配置） | – | **半条** |
| P10 宿主状态 diff 为空 | – | – | – | – | – | 0 |
| P11 并发会话同出口 | – | – | – | – | – | 0 |
| P12 自校验内嵌形态 | – | – | **✔ 范式本体** | （退出码规范） | – | 形态 ✔ |

全部现成方案加起来 ≈ 1.5 条实质覆盖 + 1 个形态范本。**缺口正是 iso-cc 的特性面**：P2/P4/P5/P8/P10/P11 断言的是"钉死 + 零残留"这类本工具自证性质，第三方工具在结构上就不可能提供——它们不知道 iso-cc 的 egress 接口名、locale 声明值和重定向集。这不是"找得不够努力"的缺口，是保证域绑定的缺口。

---

## 四、裁决与组装建议

### 裁决

1. **没有现成的 canary**。官方四个入口（doctor / srt / devcontainer 自校验 / npm 包）全部校验"安装与自身沙箱"，无一校验"会话环境合格性"；社区最接近的 `coding-agent-doctor` 停在配置层，dnsleak 族只看 DNS。
2. **用户的 mock 思路本身成立且已被现成证据支持**：cc 对环境的关键读取面已取证（Intl TZ 被 routines 调度直接上传、`/etc/resolv.conf`、egress 路由），在**同一 netns/mountns/env 内**跑探针（P1–P7、P10–P12）就是"mock 与 cc 观察同样的环境面"；探针全绿对 cc 行为是强预测。唯一 mock 不可替代的是 cc 版本相关的行为（新增必达域名、新增 env 读取），这一条由 P9（会话内跑真 `claude doctor` + `/status` 对照）补上——**先跑 mock 探针，mock 过了再跑真身自检，真身也过了才交付给用户**。
3. **`iso-cc verify`（打包 P1–P12）就是合理终点**，无需再找，也不可能找到。

### 组装建议（`iso-cc verify` 形态）

- **执行域**：探针必须在会话内（同 netns/mountns/env）执行——`iso-cc verify` 本质是 `iso-cc run -- iso-cc-verify-inner`（或等价的内置子命令），保证探针与未来 cc 看到同一环境面。宿主侧对照值（P1 宿主 IP、P6 宿主 TZ、P10 基线）在会话外采样。
- **形态**：照抄 init-firewall.sh 的内嵌正/反冒烟 + `exit 1`；退出码规范抄 `coding-agent-doctor`：`0`=全绿、`1`=有 FAIL、`2`=工具自身无法完成（如 curl 无 HTTP3 支持时 P4 显式 **SKIP** 而非 PASS，沿用前篇 P4 的语义）。
- **分级**：
  - 自动全跑：P1/P2/P3/P4/P6/P7/P10/P11/P12 + P9（真 `claude doctor` 以非交互形态跑会话内）。
  - 需真实 cc 会话：P8（写集）——用最小 cc 调用（`claude -p` + `--no-session-persistence`）夹 `find ~ -newer <marker>` diff；canary 排序上放在纯环境探针之后。
  - 破坏性/opt-in：P5（拔线）动宿主接口，默认不跑，`--destructive` 显式开启或打印手工步骤。
- **断言种子**：必达域名校验集用官方 network-config 表（api.anthropic.com + OAuth 三件套 + registry.npmjs.org），P9 结果回填。
- **CI 重放**：全部探针为静态二进制依赖（curl/dig/iproute2/node/jq），符合 N6。

---

## 来源列表

**官方（一手）**
1. debug-your-config（/doctor 检查面）：https://code.claude.com/docs/en/debug-your-config
2. troubleshoot-install（doctor 扫 shell rc 原文、downloads.claude.ai 手工探针、"claude update or claude doctor hangs"节）：https://code.claude.com/docs/en/troubleshoot-install
3. anthropics/sandbox-runtime `src/cli.ts`（全文通读，子命令清单 + refuseSettings fail-loud）：https://github.com/anthropics/sandbox-runtime/blob/main/src/cli.ts
4. anthropics/sandbox-runtime `package.json`（bin 唯一 `srt`）+ README（initialize() 硬失败面）：https://github.com/anthropics/sandbox-runtime
5. 官方 devcontainer `init-firewall.sh`（自校验两探针逐字）：https://github.com/anthropics/claude-code/blob/main/.devcontainer/init-firewall.sh
6. `claude doctor` 实测输出、`claude --help` 全量 flags、2.1.263 二进制 `Doctor*` 字符串 grep、npm 2.1.283 tarball 文件清单：本机 nix store 产物 + registry.npmjs.org [实测 2026-09-26]

**社区（元数据核验 2026-09-26）**
7. coding-agent-doctor：https://www.npmjs.com/package/coding-agent-doctor + https://github.com/DebadityaHait/coding-agent-doctor（tarball README 全文）
8. ecc-agentshield：https://www.npmjs.com/package/ecc-agentshield + https://github.com/affaan-m/agentshield
9. macvk/dnsleaktest：https://github.com/macvk/dnsleaktest ｜ orhun/dnsleaktest-tui：https://github.com/orhun/dnsleaktest-tui ｜ code3-dev/dnsleak：https://github.com/code3-dev/dnsleak ｜ mschwager/dnsleak：https://github.com/mschwager/dnsleak ｜ emanuele-f/DNSleak：https://github.com/emanuele-f/DNSleak
10. Maroka-chan/VPN-Confinement：https://github.com/Maroka-chan/VPN-Confinement ｜ wknapik/vpnfailsafe：https://github.com/wknapik/vpnfailsafe ｜ adrelanos/vpn-firewall：https://github.com/adrelanos/vpn-firewall
11. CaptainMcCrank/SandboxedClaudeCode：https://github.com/CaptainMcCrank/SandboxedClaudeCode ｜ franccesco/mullpy：https://github.com/franccesco/mullpy ｜ hgrahamcs/amimullvad：https://github.com/hgrahamcs/amimullvad
12. GitHub Repository Search API 多组查询（claude sandbox in:name；vpn leak in:name,description；dnsleak in:name,description；mullvad check in:name,description；http3/quic check in:name,description）[实测 2026-09-26]
