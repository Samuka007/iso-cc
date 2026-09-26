# iso-cc 需求与实现清单

状态：草案 v1（等待评审 → `/to-spec` → `/to-tickets`）。
遵循：每条需求 = 需求陈述（what/why/约束）+ 验收判据（fit criterion）+ 规格要点（how + 理由 + 已拒绝方案）。每条规格必须能回指需求。

---

## 〇、交付物定义（两个待开发物）

**A. 沙箱本体（iso-cc core）**——命令面：`run` / `doctor` / `list` / `--print-plan`。职责：config 解析校验（D8 scope 声明）、拦截器装配（D8 附 2 分层：SHELL/PATH/bind inode）、userns+mountns+netns 编排、provider attach（pasta / slirp4netns / embedded-socks）、exec RPC 通道（self 范围）、信号与生命周期收割。验收锚：T2/T3/T4 + N1/N3（隔离成立 + 零宿主残留）。

**B. 纯净性/功能性 stub（iso-cc verify）**——cc 的确定性替身：把 cc 换成 stub，在同一会话环境内执行 P1–P15 探针（+R12 子矩阵），输出红绿 + JSON，退出码 0/1/2。构成 = 探针套件（可独立于 cc 运行）+ 声明比对器（期望 TZ/区域/必达域名 vs 实测）。裁决语义：「环境与声明一致」而非「账号安全」（§六）。与 A 同一个 musl 二进制（verify = 子命令），定义分开因验收对象不同：A 验「隔离成立」，B 验「验证可信」（探针自身经故障注入测试）。

## 一、功能需求

### R1 网络出口定向
**需求**：会话内进程树（claude 及其全部子进程）的所有出口流量——TCP、UDP、DNS、QUIC、可选 IPv6——经指定 egress 离开；宿主其他进程零影响。不依赖 `HTTP(S)_PROXY` 等应用层变量（子进程可绕过，无法强制）。
**为什么**：claude 的网络身份需要与宿主日常流量解耦；env 代理是 best-effort，不是隔离。
**验收**：
- 会话内 `curl -4 ifconfig.me` = egress 出口 IP；宿主同命令 = 原出口。
- `curl -6`（默认禁 IPv6）失败而非走宿主 v6；UDP/QUIC 探测（`curl --http3`）同出口或失败，绝无直连。
- 宿主路由表、nftables ruleset 在会话前后 diff 为空。
**规格**：userns+netns（`unshare(CLONE_NEWUSER|NEWNET)`）→ pasta/slirp4netns attach，egress 钉死 `-i <if>` → 命名空间内 resolv.conf 指向经 egress 可达的解析器（doctor 校验可达性）。nsswitch 旁路（发行版 `resolve` 模块经 D-Bus 直达 systemd-resolved）——**第一批不做防御性 bind，由 P3b 探针检测告警（fail-loud），bind 防御列入 §七 第二批**。
已拒绝：root netns+veth+策略路由（需 root、产生持久规则=漂移面）；cgroup/fwmark 打标（DNS 漏可经 resolv.conf bind 解；真拒绝理由 = 宿主持久规则违反 R6 + 规则安装需 root 违反 R5 + 规则丢失即静默 fail-open。已升格为 `engine="mark"` 变体，见 D2 修订）；LD_PRELOAD/env 代理（可绕过）。

### R2 locale 覆盖
**需求**：会话内进程树看到指定的 TZ、LANG/LC_*；`/etc/localtime`、`/etc/timezone` 对该进程树而言是覆盖后的值。宿主不受影响。
**补充依据**（研究 2c 实证）：cc 的 routines 调度直接读取并上传 Intl 时区（`userTimezone`）——TZ 覆盖不是纯伪装面，是 cc 功能语义的一部分；必须同时满足文件路径与 Node Intl 两条视线。
**验收**：会话内 `date +%Z`、`locale`、`readlink /etc/localtime`（经验证的真实路径而非悬空符号链接）、`node -e 'Intl.DateTimeFormat().resolvedOptions().timeZone'` 全部一致且等于声明值；宿主同命令不变。
**规格**：独立 mountns 内 `make-rprivate` + 对 `/etc/localtime` 的**解析后真实路径**做 bind 覆盖（它通常是符号链接，直接绑会绑到 zoneinfo 原文件上）+ `/etc/timezone` 同理 + `--setenv` 等价物注入 env。locale 未生成的发行版回退 `C.UTF-8` 并由 doctor 警告。
已拒绝：tmpfs 盖整个 `/etc`（破坏宿主配置可见性，违反 R3）；仅 env 不盖文件（`readlink` 类探查露馅）。

### R3 其余一切放行（最小侵入）
**需求**：文件系统、HOME、Docker socket、包环境、SSH/GPG agent 原样共享；不 clearenv、不隔离 /proc、不复制 rootfs。
**为什么**：沙箱的目的是定向（网络/locale/CC 配置），不是囚禁；全隔离破坏正常使用（对话已明确否决）。
**验收**：会话内 `docker ps` 操作宿主 daemon；`ssh-add -l` 可见 agent；项目文件读写即宿主文件；`env` 除声明覆盖项外与宿主一致；**会话内起的 dev server，宿主 `localhost:同端口` 直接可达**（pasta `-t/-u` 默认 auto 转发，passt.1 取证；宿主端口冲突时 doctor 报告并提示显式 `--publish` 改道）。
**规格**：mountns 只做 R2 的两个 bind；userns 仅用于获得 rootless 挂载能力，uid 映射回自身。
**localhost 语义分层**：netns 内 `127.0.0.1` 无条件保留给会话自身（dev server 原生语义；127/8 由内核 local 表钉死，不跨出）。宿主 loopback 服务两条覆盖：①`hostgw` 别名——mountns bind 自定义 `/etc/hosts`（`hostgw → 网关IP`，pasta `--map-host-loopback` 默认映射网关→宿主 loopback，源地址呈现为宿主本机），供可改写配置使用（HTTP MCP URL 等）；②`net.localhost_forward = [端口…]`——声明式端口预占，netns 内 127.0.0.1:port 由内嵌 relay 转发至网关→宿主 loopback（覆盖 stdio MCP 连本地数据库等不可改写场景），撞号先到先得 + doctor 警告。全局把 127.0.0.1 指向宿主被拒绝：与 agent 自身 dev server 语义冲突（同地址同端口两个主人）+ 内核 local 表/martian 防线。

### R4 Claude Code 配置隔离（防互相污染）
**需求**：沙箱内 claude 与宿主 claude 的状态互不污染：credentials、sessions、history、settings、项目信任、plugins、hooks、memory 各自独立；双向隔离（沙箱会话不写宿主，宿主升级/重登不破坏沙箱 profile）。
**为什么**：同一台机器上宿主 cc 与沙箱 cc 并存，共享 `~/.claude` 会导致登录态互踢、会话/信任串扰、settings 漂移。
**验收**：
- 会话内 cc 视角无感：`claude /status` 显示默认 `~/.claude`；底层落盘可验——会话内登录后，profile 目录出现 `.credentials.json` 而宿主 `~/.claude` 无新增。
- 双开（宿主 cc + 沙箱 cc）同时工作互不干扰；会话结束后宿主 `~/.claude` 与 `~/.claude.json` 内容/mtime 不变。
**规格**：mountns 内**声明式路径重定向**（bind 列表）：`~/.claude/` → `~/.local/state/iso-cc/<profile>/claude/`，`~/.claude.json`（含 `.backup`）→ per-profile 文件；cc 看到的仍是其默认路径，完全无感。会话内 **unset `CLAUDE_CONFIG_DIR`**（防外部环境把 cc 引向第三处）。挂载点在宿主缺失时预创建空文件/目录（有界例外，doctor 报告）。config 可追加重定向条目。profile 目录跨会话持久（登录态/会话史留在 profile）；默认 clean-room 不继承宿主 `~/.claude`（杜绝反向污染），可选一次性 seed。**T4 核心断言：实测写集（`find ~ -newer` 探针）⊆ 声明重定向集**——把"cc 未来新增全局文件"的风险变成受测不变式。
R12 落地时扩列：`~/.config/orca/`（orca 的 Electron userData：Partitions 浏览器 profile、claude-accounts、claude-runtime-auth 等）。
**写集例外登记**（CcToolSurface 取证，P8 探针白名单）：cc 的 `autoInstallIdeExtension` 写 `~/.vscode/extensions`；Chrome native messaging manifest 写浏览器配置目录；宿主 `/etc/claude-code/managed-settings.json` 为只读渗入（非写点）。声明式关闭项随 profile env 提供。
已拒绝：`CLAUDE_CONFIG_DIR`（路由集合由 cc 版本决定、cc 可感知、`~/.claude.json` 是绕过 envvar 的现存反例）；HOME 级 overlayfs（copy-up 会把 agent 对项目文件的写截留进 profile upperdir，宿主看不到改动，破坏核心工作流，违反 R3）；直接共享 `~/.claude`（污染即需求本身）。

### R5 rootless
**需求**：全程不需要 root/sudo/setuid；在 `kernel.unprivileged_userns_clone=1` 的发行版上开箱即用。
**验收**：以普通用户在 NixOS、Ubuntu 22.04、Debian 12、Arch 上完成全部功能验收。
**规格**：userns 内映射自身 uid→0（仅映射自身 uid，无需 newuidmap setuid 助手）。AppArmor 限制（Ubuntu 23.10+/24.04 的 `apparmor_restrict_unprivileged_userns`）**延后到 P3**：doctor 检测并给出安装 profile 的明确指令。

### R6 声明式 + 无漂移
**需求**：config 是唯一输入；无 `up`/`down` 类改写持久状态的动词；有效状态永远可从 config 重算；kill 会话即完整拆除，不遗留任何宿主痕迹。
**验收**：
- 仓库/`~/.config` 里的 TOML 之外，不存在任何由本工具写入的持久状态文件。
- `kill -9` 会话根进程后：无 pasta 孤儿、无残留命名空间（`lsns` 对比）、宿主状态 diff 为空。
- `doctor` 输出 = 配置意图 vs 现实的全量 diff，重复运行幂等。
**规格**：会话 = 进程生命周期（见 D1）。`list` 扫 `/proc` 报告存活会话。

### R7 隧道协议解耦
**需求**：工具代码与配置中不出现任何隧道协议字样；egress 契约 = 「宿主上存在一个已配置路由的 L3 接口」。
**为什么**：隧道是独立关注点（do one thing）；每个隧道只归一化一次，不是每个沙箱一次。
**验收**：`egress = "if:wg0"` 在 WireGuard / sing-box TUN / mihomo TUN 下行为一致；doctor 校验接口存在且有默认路由。
**规格**：代理形态（SOCKS/HTTP）隧道由隧道侧自出 TUN（mihomo/sing-box 均为单 flag）；文档写死此边界。已拒绝：工具内置 tun2socks 适配（v1 越界）。

### R8 fail-closed
**需求**：egress 失效时会话内表现为断网，绝不回落宿主直连。
**验收**：会话中 `ip link set wg0 down`（在宿主操作），进行中的连接挂起、新连接立即失败；`ifconfig.me` 永远不返回宿主 IP。
**规格**：pasta 网关唯一出口，命名空间内不存在第二条路径；v6 默认关闭（`net.ipv6 = "off"`）。

### R9 agent 入口：宿主管理 + 声明式定义（决策，见 D3）
**需求**：`iso-cc run -- claude` 运行的 claude 由宿主安装管理；本项目只负责**发现 + 声明 + 校验**入口，不复制、不接管包管理。
**验收**：doctor 报告发现的安装方式（npm-global / native `~/.local/bin/claude`）与版本；config 可声明 `agent.command`（默认 `claude`，走 PATH）与 advisory `agent.version`（不匹配=警告不阻止）。
**规格**：见 D3。

### R10 多 profile 与并发会话
**需求**：多个 profile（不同 egress/locale）与同一 profile 的多个并发会话互不干扰；并发会话共享同一 egress 接口（出口一致），各自独立 usernet 子进程与命名空间。
**验收**：两个会话并发 `curl ifconfig.me` 同出口；`list` 正确报告两个会话；各自 kill 独立拆除。

### R11 便携
**需求**：单静态二进制 + 尽量少的宿主依赖，覆盖：NixOS、Ubuntu 22.04+、Debian 12+、Arch。
**验收**：musl 静态二进制在上述发行版（或等价容器）通过 T5 验收矩阵。
**规格**：Rust + musl；运行时依赖仅 usernet provider——pasta（Debian 12+/Ubuntu 23.04+/NixOS 有包；上游发布静态构建 [passt.top/builds/latest/x86_64/](https://passt.top/builds/latest/x86_64/) 单文件可直接用于 jammy）或 slirp4netns（Ubuntu 22.04 universe 1.0.1-2）。provider 可插拔（见 D5）。完整依赖面矩阵见 `docs/DEPENDENCIES.md`（本机实证：最小会话运行时外部依赖 = 0 个二进制，网络定向 +1）。

### R12 浏览器身份同域（后期，P3）
**需求**：用于 cc 登录（OAuth）及一切"以账号身份"访问的浏览器，其出口与指纹保持在会话虚拟环境内：与 cc API 流量同 egress、TZ/locale 一致、浏览器 profile 状态跨会话稳定。
**为什么**：这是 v1 的**显式缝隙**——cc `/login` 打开的 OAuth 浏览器若在宿主（xdg-open），登录事件的出口/指纹与后续 API 流量不一致。闭合方式：orca（stablyai/orca，MIT，78.6k★）remote-server 模式把 browser handoff 与全部运行时放 server 端，`iso-cc run -- orca serve` 把登录浏览器收进沙箱。关键实证（研究报告）：**网络归属与渲染位置正交**——client 渲染的页面也强制经 loopback SOCKS 隧道把 HTTP/WS/DNS/loopback 送回 server（`proxyBypassRules '<-loopback>'`），账号身份浏览不受渲染位置选择影响。
**验收**（绑定探针：orca 研究报告 §2.2，全部可会话内执行）：
- 出口三面一致：会话内 `curl` == orca 浏览器探针页出口 == cc API 出口；v6/QUIC 不直连（P1，复用 R1/R8 矩阵）。
- TZ/locale 三方一致：Intl TZ、`navigator.languages`、Accept-Language 头 == 声明值（P2/P3；orca 不覆写 locale/TZ，R2 注入天然传导）。
- 跨会话指纹稳定：UA（clean 模式，无 Electron token）/canvas/WebGL/fonts 两次会话一致；cookie/localStorage 跨会话存活且落盘在重定向集内（P4/P6/P7/P8）。
- **OAuth 回环闭合**：会话内 `/login` 完成（callback = `http://localhost:<port>/callback`，PKCE S256，cc 二进制实证），凭据落重定向 profile，浏览器外联全经 egress（P9）。
- 宿主浏览器零参与：宿主浏览器进程增量 0；`~/.local/bin/orca` shim 与 Xvfb 为登记的有界例外（P10）。
- 外部拨入唯一路径：orca client 经 `--publish` 配对成功，且宿主无绕过 publish 的第二条入站路径（P11；`ss`/`nft` diff 复用 N3）。
**规格要点**：
- **路径排序（全景研究 §3）**：①**宿主显示 socket bind-mount**（Wayland 优先：`$XDG_RUNTIME_DIR/wayland-0` 或 WSLg `/mnt/wslg/`；X11 兜底 `/tmp/.X11-unix/X0` + XAUTHORITY）——与 R2/R4 同为声明式 bind，**零 `--publish`、零编码栈、100% 真实 headed、OAuth 原生窗口体验**（依据：netns 不隔离文件系统 AF_UNIX，`network_namespaces(7)`；Chromium `--ozone-platform=wayland`/Firefox 121+ 原生支持）——此路径不依赖 orca，可独立成立；②orca serve + `--publish`（本节主路径，P3）；③无图形会话宿主的兜底：cage+wayvnc 或 Xvfb+x11vnc+noVNC + 单端口 `--publish`；④可选强隔离变体 waypipe。排除项（含理由）见全景研究 §3：云 RBI（出口在厂商侧）、browserless（SSPL）、Kasm CE（license）、Guacamole/Sunshine/GNOME-RDP（重量或特权面）。headless 只作非交互自动化支线（Chrome 132 起 headless-shell 独立分发；`--headless=new` UA 与 headed 一致；Playwright 默认 headless 为 headless-shell 世系且注入自动化痕迹——源码实证）。
- 入站发布：`--publish [host-ip:]hostPort:netnsPort/TCP` → pasta `-t/-u`。orca serve 已绑 `0.0.0.0:6768`（serve 模式显式 `exposeNetworkByDefault`），`--pairing-address` 是纯通告语义，填宿主可达地址即可。拒绝：改 orca 配置 / 内核端口重定向（root+持久规则，违反 R5/R6）。该原语同时一般化解决"agent 起的 dev server 从宿主浏览器访问"。
- display 策略：会话默认**不复用宿主 DISPLAY**，让 orca 自起 Xvfb（渲染栈收敛进会话可定义集，是 P6 稳定性前提）；doctor（R12 落地时）新增检查：会话 PATH 含 Xvfb、会话内无 DISPLAY。
- 声明式重定向扩列（R4 机制）：`~/.config/orca/`；写集断言白名单含已知临时物（`$TMPDIR/orca-account-add-claude-*` 设计内自清理、启动期 `~/.local/bin/orca` dispatcher——P3 实测 kill 后是否残留再定去留）。
- 身份基线：UA identity 固定 `clean`；账号身份浏览用会话内页面或 Server (streamed) 渲染；client-hosted 页面网络身份虽同域，但 TLS/GPU/字体面来自 client 设备，不计入会话内指纹（文档注记）。
- 可选 env：`DO_NOT_TRACK=1` / `ORCA_TELEMETRY_DISABLED=1`（声明式，doctor 回显）。
- 已知缝隙（接受项，非伪装目标）：UTS hostname 未隔离（宿主名可经 mDNS 类探查）；SwiftShader 软件渲染与真 Chrome 可辨。R12 目标 = **同域 + 稳定**，不是反取证。
- 跨路径指纹注记：字体集若沙箱与宿主不一致，canvas/文本测量哈希漂移——宿主字体目录纳入声明式 bind（R4 同机制）[研究建议]。

---

## 二、非功能需求

| # | 需求 | 验收判据 |
|---|------|----------|
| N1 | fail-closed 是唯一失败模式 | 见 R8；泄漏测试矩阵（T5）全绿 |
| N2 | 会话启动开销可忽略 | `run -- true` 端到端 < 300ms（pasta 冷启 < 100ms 量级） |
| N3 | 零宿主状态突变 | run 前后 `/proc/mountinfo`、`ip route`、`nft list ruleset`、`lsns`、`~/.claude*` diff 为空（T5 断言）；有界例外：①挂载点缺失时预创建的空挂载点；②R12 场景 orca 启动期 `~/.local/bin/orca` dispatcher 与会话自起 Xvfb（P3 实测定界后登记，doctor 报告） |
| N4 | 可观测 | `--print-plan` 输出等价命令序列；`--verbose` 逐动作日志；`list`/`doctor` 机器可读输出 |
| N5 | 依赖面 | 运行时 = 1 个二进制 + 1 个 provider；`--print-plan` 是唯一透明性承诺 |
| N6 | 可测试 | 泄漏/状态不变式测试全部脚本化，可在 CI 容器内重放 |

## 三、目标环境矩阵（实测）

| 宿主 | 发行版/内核 | userns | AppArmor 限制 | usernet 包 | 备注 |
|------|------------|--------|----------------|-----------|------|
| 本机 | NixOS / WSL2 内核 6.6+ | ✅ | 无 | nixpkgs `passt` | 开发环境 |
| chenyizi-4090 | Ubuntu 22.04.2 / 5.19.0-45 | ✅ `unprivileged_userns_clone=1`，`max_user_namespaces≈2.06M`（实测） | 无（23.10+ 才引入） | jammy 无 `passt`；用 `slirp4netns`（universe）或上游静态 pasta | 首个验证宿主 |
| Ubuntu 24.04 类 | 24.04+ / 6.8 | ✅（条件） | ⚠️ `apparmor_restrict_unprivileged_userns` | 有 `passt` | **P3**：随包发布 apparmor profile，doctor 检测并提示 |

## 四、决策记录

### D1 无状态会话模型（替代具名 netns 单例）
会话 = 进程生命周期；不 `up`、不命名 netns、不落任何状态。**拒绝**：持久 netns 单例（具名状态=漂移温床，正是 R6 要消灭的）；`systemd-run --user` 变体（unit 名持久、终端关闭后 unit 存活，同为状态面）。并发多会话的代价是每会话一个 pasta（几 MB），可接受。

### D2 rootless pasta 路线（替代 root netns / cgroup 打标 / Docker）（2026-09-26 修订：cgroup 升格为 engine 变体）
userns+netns+usernet provider。**拒绝**：A) root netns+veth+策略路由——需要 root 与持久 nftables/路由规则（漂移面、sudoers 面）；C) Docker/容器——重、rootfs 复制、违反 R3 透明性。**B) cgroup/fwmark 修订**：DNS 漏可解（resolv.conf bind 指向隧道可达 resolver → 查询由会话自身 socket 发出，即被 mark 命中走隧道）；真正拒绝理由 = 宿主持久规则违反 R6 + 规则安装需 root 违反 R5 字面义 + 规则丢失静默 fail-open（doctor fail-loud 断言规则存在 + 表内 blackhole 可缓解不可消除）。降格为 `engine = "mark"` 声明式变体（保留 userns+mountns 做 locale/CC 重定向，去 netns/pasta），**localhost 双向零摩擦为其独有优势**；实现排序 T6 后，触发条件 = netns 方案 localhost 三件套（auto 转发/hostgw 别名/端口预占）实测硌手。

### D3 agent 入口 = 宿主管理 + 声明式定义（本项目不捆绑 release）
**选定**：passthrough——doctor 发现安装方式与版本，config 声明 `agent.command`/advisory `agent.version`，mismatch 仅警告。
**理由**：claude code 周更，捆绑=复制一个包管理器（下载、校验、更新、多平台构建）；OAuth/登录流与 shell 集成天然属于宿主安装；用户已在各宿主管理 cc。
**拒绝**：捆绑 release（维护成本、更新延迟、签名链）——留作后续可选 `agent.pinned` 模式，非 v1；nix profile 方式——只覆盖 NixOS 用户，违反 R11。

### D4 egress 契约 = L3 接口引用（2026-09-26 修订：SOCKS 形态部分解禁）
工具 v1 只消费 `if:<name>`。**修订**：嵌入式 SOCKS provider（TUN-in-netns + netstack-smoltcp 流还原 + tokio-socks pump，glue 约 300–500 行，setns 跨 netns 同 pasta 模式，DNS 经 53 拦截走 SOCKS 转发）经论证 rootless 可行且直接消费现有 mixed-port，升级为 SOCKS 形态的正解候选——pasta 在该形态下不参与；接口形态下 pasta 仍是对等物（D7 不变）。插入时机：T3 之后按隧道形态择一。**维持拒绝**：WG 跨 ns socket 技巧（协议专属，违反 R7）；env 代理（R1）。

### D5 实现形态 = thin Rust musl 静态二进制，provider 可插拔
核心动作是 syscall 编排（unshare/uid_map/mount）+ 子进程生命周期——shell 最脆的两件事；shell 的透明性由 `--print-plan` 补偿。usernet provider 抽象为 trait，pasta 首选、slirp4netns 回退（Ubuntu 22.04 无 passt 包的实测约束）。

### D8 网络隔离范围声明 net.scope = tree | self（ADR 0008）
R1 的进程树范围是一致性范围；平台风控面实为 cc 本体流量（身份范围）。`net.scope` 声明二选一：`tree` 默认（R1 原文）；`self`（cc 本体 netns + Bash 整段 RPC 转发宿主执行，cc 二进制例外，verify 语义收窄为 cc 本体出口）。详见 `docs/adr/0008-net-scope-declaration.md`；self 是否入 v1 待定。

## 五、实现清单（tracer-bullet 序，含阻塞边）

| # | 工单 | 交付 | 需求引用 | 阻塞于 | 验收 |
|---|------|------|----------|--------|------|
| T0 | 仓库骨架 | `flake.nix` devShell（rust+musl target、clippy、rustfmt、pasta、slirp4netns）、CI（build+clippy+test）、README | R11 | — | `nix develop` 内 `cargo build` 通过；CI 绿 |
| T1 | config + doctor | TOML 解析/校验/合并（全局+项目覆盖）、`doctor` 全项检查（egress 接口+路由、zoneinfo、locale、provider 二进制、userns sysctl、claude 安装探测、**cc 必达域名可达性**：api.anthropic.com、claude.ai/claude.com/platform.claude.com、registry.npmjs.org）、`--print-plan` | R6,R7,R9,R11 | T0 | 对 4090 实测输出正确 diff；故意破坏项逐条报错；**配置非法时 exit≠0 拒绝降级**（srt `refuseSettings` 范本：环境不合格拒绝启动，绝不静默回退） |
| T2 | 最小 rootless 会话（无网络定向） | userns+mountns、locale bind 覆盖（符号链接解析）、env 注入、exec、信号收割 | R2,R3,R5,R6 | T1 | 手动会话内 locale 全项一致；kill 后宿主状态 diff 为空 |
| T3 | 网络定向 | netns + pasta attach（egress pin、`--map-host-loopback`（沙箱→宿主服务，**默认即映射网关地址 → 宿主 loopback**）、resolv.conf、v6 off、**auto 端口转发默认开**（dev server 透传）+ 端口冲突 doctor 检测、宿主 loopback-only 服务（localhost MCP/IDE）可达性探针） | R1,R3,R7,R8,R10 | T1 | T5 网络子矩阵全绿；宿主浏览器直开会话内 dev server；会话内可经网关地址访问宿主 loopback-only 服务 |
| T4 | CC 配置隔离 | mountns 声明式路径重定向（默认 `~/.claude`、`~/.claude.json(.backup)`），unset `CLAUDE_CONFIG_DIR`，挂载点预创建与 doctor 报告、**写集探针**（`find ~ -newer` 断言 ⊆ 声明集）、首登流程文档、双开测试 | R4 | T2 | R4 验收三条全过；写集断言绿 |
| T5 | 泄漏与状态不变式验收矩阵 + `iso-cc verify` | 三部分：①**纯净度探针 P1–P15 + P3b**（../iso-cc-research/2026-09-26-related-projects-and-purity-probes.md §2d 基础 12 条 + 社区研究增补 ../iso-cc-research/2026-09-26-community-risk-signals.md：**P13** 出口 IP 区域归属==声明区（FAIL——官方唯一语义信号，GH anthropics/claude-code#2656 确定性 400）；**P14** IP 类型/欺诈分（WARN 留档，存活反例密集，禁止 FAIL）；**P15** 会话生命周期内出口恒定（社区最高频 IP 漂移信号）；zh 字体 WARN 记录；R12-P9 追加 OAuth 授权时刻出口未漂移；**P3b** getaddrinfo 路径解析一致性（`getent ahostsv4 whoami.akamai.net` == 隧道侧 resolver 出口，本机实测方法有效）+ nsswitch bind 静态断言——dig 类探针只测 resolv.conf 路径，测不出 nsswitch `resolve` 模块旁路）；②**R12 浏览器子矩阵 12 条**（../iso-cc-research/2026-09-26-orca-browser-identity.md §2.2；P3 阶段执行）；②b **工具面子矩阵 P16–P27**（../iso-cc-research/2026-09-26-cc-tool-surface.md：MCP 三传输/hooks 执行位置/IDE lockfile/Monitor WS/OTel collector/bwrap 嵌套出口/跨会话 UDS；版本漂移登记 2.1.263 基线）；③**产品化 = `iso-cc verify` 子命令**：与测试共用同一探针实现，会话内一键红绿（现成 canary 不存在，../iso-cc-research/2026-09-26-environment-canary-tools.md——覆盖 ≈1.5/15，保证域绑定的结构性缺口）；形态 = init-firewall.sh 正/反冒烟内嵌 + srt fail-loud；退出码 0=过/1=泄漏或环境不合格/2=依赖缺失（HTTP3 不可用显式 SKIP 不记 PASS）；**裁决语义钉死为「会话环境与声明一致」，非「账号安全」**——固定输出「不覆盖面」声明（行为/内容/账号/支付层，见 §六）；P5 拔线 opt-in；P8 写集断言用最小 cc 调用；P9 会话内跑真 `claude doctor`。断言只依赖会话内自起探针页，第三方服务留档非断言 | N1,N3,N6 | T2,T3,T4 | 4090 + 本机全绿，可 CI 重放；`verify` 退出码语义经故障注入验证 |
| T6 | slirp4netns 回退 provider | provider trait 第二实现，Ubuntu 22.04 全流程走通 | R11 | T3 | 4090（无 passt）全流程过 T5 |
| T7 | 发布 | musl 静态产物 + 安装/使用文档（含隧道侧出 TUN 的边界说明） | R11 | T1–T6 | 两台宿主从二进制冷启动过 T5 |
| P3 | 后期 | AppArmor profile（24.04）、`agent.pinned` 捆绑模式、socks 形态适配评估、systemd-run 变体、**`--publish` 入站发布原语 + R12 orca/浏览器身份同域**（验收与规格已按 orca 研究报告细化，含 `~/.config/orca` 重定向扩列、Xvfb/display 策略、身份基线）、**`host_exec` 逃生舱**（共享 fs unix-socket RPC，父进程宿主 netns 代执行；profile 开关默认关；verify 不覆盖面声明联动） | R5,R9,R12 | T7 | R12 验收六条全过（探针 P1–P11）；`~/.local/bin/orca` 残留行为实测定界 |

## 六、显式非目标

- 反取证级伪装、任何风控承诺。**环境一致性是必要非充分条件**（社区反证：硅谷稳定出口 20 天封、法国家宽同日封、英国漫游+KYC+时区全对 5 小时封，linux.do t/2952716）——`verify` 只裁决前者；行为/内容/账号/支付层信号完全出界。
- 管理隧道生命周期（R7）。
- 囚禁型沙箱（R3：不隔离文件系统/Docker socket/proc）。
- v1 内置 socks 透明重定向（D4）。

## 七、待解决（core，已识别、第二批实现）

| # | 问题 | 现状 | 第二批方案 | 探针 |
|---|------|------|-----------|------|
| Q1 | **nsswitch `resolve` 模块旁路**：Fedora 类发行版 `hosts: ... resolve [!UNAVAIL=return] dns` → getaddrinfo 经 D-Bus 直达 systemd-resolved，resolv.conf bind 被整体绕过；unix socket 走共享 fs，netns 不挡，两引擎均中 | 第一批不防御；P3b 探针（`getent ahostsv4` 路径）检测告警，受影响宿主上 verify 红 | mountns 第四条 bind：`/etc/nsswitch.conf` 强制 `hosts: files dns`（两引擎同防；静态断言入 doctor） | P3b（已列入 T5，检测语义） |
