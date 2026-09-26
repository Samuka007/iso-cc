# orca 运行时归属、浏览器 handoff 与会话内浏览器指纹面（R12 研究）

状态：研究完成（支撑 R12 验收细化，P3）。
一手来源锚定：`stablyai/orca` @ commit `d20cb69c48af2c7abb0651d02499025fe6899f5a`（2026-09-26，v1.4.197，MIT，78,596 stars，本机浅克隆核验）；在线文档 `onorca.dev/docs` 与仓库内 `docs/site/content/docs/` 逐句一致（已抽样比对）。Claude Code 侧取本机安装的 `claude-code 2.1.263`（nix store 路径见来源 S10）内嵌 JS 字符串证据。
本文不修改 REQUIREMENTS.md，只给修订建议。所有"风控/账号信号"类推断均显式标注；未验证处标 [未验证]。

---

## TL;DR

1. **`orca serve` 把整个 workbench 运行时放在 server 进程树**：repos/worktrees、PTY 终端、agent 进程、provider 账号凭据、会话状态、内嵌浏览器全部归属 server 运行时；client 只是 UI（官方文档原句 + 源码）。`iso-cc run -- orca serve` 使该进程树整体进入沙箱 netns——R12 的"出口同域"由构造保证。
2. **browser handoff 是两个正交决策**：①页面在哪台设备*渲染*（client-hosted 默认 / server streamed，用户可切）；②页面的*网络身份*归属谁。orca 对远程 workspace 的实现是：client 渲染的页面也强制经 loopback SOCKS 隧道把 HTTP(S)/WS/DNS/**loopback** 送回 server（`proxyBypassRules: '<-loopback>'`），即网络身份永远跟 workspace 的 host 走。渲染位置只影响 TLS 指纹栈与 GPU/字体面。
3. **`orca serve` 网络语义对 iso-cc 极友好**：WS 监听绑定 `0.0.0.0:6768`（serve 模式显式 `exposeNetworkByDefault`，源码 STA-2370），`--pairing-address` 仅改通告地址不改 bind。在 netns 内运行时无需任何 orca 感知；宿主侧用 pasta `-t` 端口发布即可让 Tailscale client 拨入——正是 R12 的 `--publish` 原语。
4. **OAuth 回环天然闭合**：claude code 的 OAuth callback 是 `http://localhost:<port>/callback`（PKCE S256，一手二进制证据）；cc 与浏览器同 netns 时 loopback 直达；即便用 client 渲染的浏览器，`<-loopback>` 规则也会把回环送进 server netns。
5. **两个必须在 R12 落实的宿主副作用**：orca 在 Linux 启动期会向 `~/.local/bin` 写一个临时 `orca` dispatcher（设计上"仅在启动期"），以及 `orca serve` 无 DISPLAY 时自动拉起 Xvfb——前者是 N3 零宿主状态的例外候选（T4 写集断言会抓到），后者要求会话内 PATH 有 `Xvfb`（doctor 新增检查项）。
6. **指纹面**：orca 默认 "clean" UA（剥离 Electron/Orca token，Chrome 形）；每个 browser-use profile 是独立 Chromium storage partition，落盘 `<userData>/Partitions/`；所有经路由的 guest 强制 `disable_non_proxied_udp`（WebRTC fail-closed）。TZ/LANG 未被 orca 覆写 → 会话注入直接传导到 `Intl`/`Accept-Language`。跨会话稳定性由"同一二进制 + 同一宿主字体/GPU 栈 + profile 持久化"保证；canvas/GPU（SwiftShader）与字体面与真 Chrome 有差异，属已知且稳定的差异，不在伪装目标内（§非目标）。

---

## Q1 orca 运行时归属与浏览器 handoff

### 1.1 orca 是什么、与 iso-cc 的关系边界

orca（Stably Inc.，MIT，"ADE for working with a fleet of parallel agents"）是 Electron 桌面 workbench + CLI（`orca`/Linux 上 `orca-ide`，因 GNOME 屏幕阅读器占用 `/usr/bin/orca`）。它管理 worktrees、终端、agent（claude/codex/…）、内嵌 Chromium 浏览器、移动端与远程配对。它是 **workbench，不是隔离层**：官方运行模式是 Local / SSH target / Remote Orca Server / per-workspace Cloud VM（`docs/site/content/docs/ways-to-run.mdx:12-19`），任何模式都不提 netns/沙箱；进程隔离与出口定向不在其功能面内。两者正交：orca 提供浏览器与账号的"运行位置"，iso-cc 提供"运行环境的网络/locale/配置域"。

### 1.2 remote-server / `orca serve` 模式的运行时归属

官方归属陈述（`docs/site/content/docs/remote-servers.mdx`，与 onorca.dev/docs/remote-servers 在线版一致）：

- "The server keeps the **projects, worktrees, terminals, tabs, provider accounts, and agent sessions**. Your laptop connects to that running Orca instance."（:8）
- "Install and authenticate Codex, Claude Code, OpenCode, `git`, and any provider CLIs on the **server computer**. A login on your laptop does not automatically carry over to the server."（:30）
- "Remote sessions use the server's **PATH, home directory, and credentials**—not the client's."（:218，故障排查节）
- "Terminals, agent processes, files, worktrees, and session state live on the server computer."（:101）

源码侧凭据落点（全部在 Electron `userData` 下，即 server 运行时私有状态）：

| 对象 | 位置 | 证据 |
|---|---|---|
| Claude 托管账号 | `<userData>/claude-accounts/`、`<userData>/claude-runtime-auth/` | `src/main/claude-accounts/managed-auth-path.ts:9`、`runtime-auth/runtime-auth-file-storage.ts:74` |
| Codex 托管账号 | `<userData>/codex-accounts/`、`<userData>/codex-runtime-home/` | `src/main/codex-accounts/runtime-home-service-paths.ts:68,86`、`codex-managed-home-path.ts:18` |
| 浏览器 profile（cookies/localStorage/cache） | `<userData>/Partitions/<name>/`（Electron `session.fromPartition('persist:…')` 标准落盘） | `src/main/browser/browser-cookie-chromium-prepare.ts:40-47`（由 `session.getStoragePath()` 推导，注释明言"Chromium partition name maps to a directory"） |
| E2EE keypair（mobile 配对） | `<userData>/<E2EE_KEYPAIR_FILENAME>`（0600） | `src/main/runtime/e2ee-keypair.ts:25-46` |
| 实例互斥 | userData 内 singleton lock；退出码 3 = "another process already owns this userData profile" | `docs/reference/headless-linux-server.md:987-989` |

`userData` 在 Linux 的默认值 = Electron 语义 `<appData>/<name>` = `$XDG_CONFIG_HOME/orca`（`package.json` name=`"orca"`，无 `setName` 覆写 packaged 路径的代码；dev 模式才改为 `orca-dev`，`src/main/startup/configure-process.ts:221`）。官方 headless 文档出现的 `~/.config/orca-rollback-$stamp`（`headless-linux-server.md:864`）与此吻合。**[INFERENCE]** packaged 精确目录为 `~/.config/orca`——建议 P3 实机以 `orca status --json` 或 `lsof` 落锤一次。

**`orca account add --agent claude` 的精确语义**（`src/cli/specs/account.ts:14-21`、`src/cli/handlers/account.ts:86-241`）：在**当前终端**跑 `claude auth login --claudeai`（`CLAUDE_CONFIG_DIR` 指向 `mkdtemp('orca-account-add-claude-*')`），完成后调 runtime RPC `accounts.addClaudeFromConfigDir` 把凭据导入 `<userData>/claude-accounts`，临时目录即弃。注释原文："the desktop GUI flow drives this via a browser Orca can't reach on a headless host"——即 headless 下 OAuth URL 打印在 server 终端、由人粘贴 code；GUI 下登录浏览器是 orca 自己的内嵌浏览器。两个变体对 R12 的含义见 §Q1.7 与修订建议。

### 1.3 browser handoff：渲染位置与流量归属正交

远程 workspace 的浏览器页有两种托管（`docs/site/content/docs/browser/overview.mdx:29-33`）：

> "For a workspace on a paired Remote Orca Server, **new browser pages render on this desktop by default while HTTP(S), WebSocket, DNS, and loopback traffic still go through the remote host**. Input, selection, and popups stay native to this device **without changing the page's network identity**. The host indicator in the browser toolbar shows where traffic is going."

- 设置面：`Settings → Browser → Remote server workspaces → This device | Server (streamed)`，只影响新开的页；本地渲染失败可 **Reopen on server**（"signed-in or form state may differ"——因为两个位置用不同的 storage partition）。
- **流量归属的实现**（一手源码）：client 侧为每个 route partition 设 `session.setProxy({mode:'fixed_servers', proxyRules:'socks5://127.0.0.1:<port>', proxyBypassRules:'<-loopback>'})` 并在应用后用 `resolveProxy` 探针验证（失败抛 `browser_route_partition_proxy_verification_failed`）——`src/main/browser/browser-route-session-policy.ts:35-49`。SOCKS listener 只绑 loopback（`browser-client-network-route-address.ts:3`，`remote-browser-socks-server.ts:60`，对 `0.0.0.0` 目标拒绝 :279），经配对 WebSocket（`paired-runtime-browser-network-route.ts`）或 SSH execution route（`local-ssh-browser-route.ts`）把连接 dial 到 server 侧。**`'<-loopback>'` 是点睛之笔**：它反转 Chromium 默认的 loopback bypass，使页面对 `localhost:<dev-port>` 的访问也进隧道、到达 **server 的** 127.0.0.1——OAuth callback 由此在 server netns 内闭合。
- 结论（证据链闭合）：**browser handoff 的网络归属 = workspace 所在 host，与渲染位置无关**；渲染位置只决定 TLS/HTTP2 指纹栈（client 的 Chromium vs server 的 Chromium）、GPU/字体面与输入延迟。R12 的"账号身份浏览"应落在 server-hosted（会话内）页面；client-hosted 页面可作为"看板"，因其网络身份仍同域。

### 1.4 `orca serve` 端口/配对机制与 netns 拨入条件

- 命令面：`orca serve [--port <port>] [--pairing-address <host>] [--mobile-pairing] [--no-pairing] [--project-root <path>] [--recipe-json] [--json]`（`src/cli/specs/serve.ts:8-32`；`--json` 输出单行 ready 契约）。前台运行，Ctrl-C 退出；`--port 0` = bind 时随机（`config/docker/headless-serve-shutdown/run-signal-case.sh:53`）。
- **bind 与通告分离**（`docs/reference/headless-linux-server.md:121-124` 原句）："`--pairing-address` is only the address advertised to clients. **It does not change the listener bind address.**" ready 块输出 `Bound endpoint: ws://0.0.0.0:6768` + `Advertised endpoint: ws://100.64.1.20:6768`（:133-140）。
- **bind 语义源码**：desktop 默认 loopback，配对时才懒扩；"`Only 'orca serve' (explicit remote opt-in) and E2E set this`"（`exposeNetworkByDefault`，`src/main/runtime/runtime-rpc/runtime-rpc-pairing-types.ts:19-27`）。默认端口 `DEFAULT_WS_PORT = 6768`。
- **通告合法性**（`src/main/runtime/pairing-endpoint.ts:20-25,42`）：无 `--pairing-address` 时通告回落 `127.0.0.1`（"default pairing must remain local-only"）；通配（`*`/`0.0.0.0`/`::`）拒绝作为通告地址；`http(s)://` 归一化为 `ws(s)://`；`--pairing-address` 带 host 无 port 时与实际 bound port 组合。
- 配对凭据：一次性 pairing code 生成 per-client 可吊销 token（remote-servers.mdx:113-118）；mobile 走 E2EE（ECDH，`e2ee-keypair.ts:1-3`）。
- **netns 拨入条件（iso-cc 视角）**：`orca serve` 在 netns 内 bind `0.0.0.0:6768` → 该 listener 只在 netns 内可见。外部 client 经宿主 Tailscale 拨入需要：①宿主侧 host→netns 端口发布（pasta `--map-host-loopback` 无关，此处需 `-t <hostPort>:6768`，即 R12 `--publish` 原语；WS over TCP，无 UDP 需求）；②`--pairing-address` 填宿主侧 client 可达地址（如宿主 Tailscale IP:port 或反代 URL）。除此之外无额外条件：配对协商全走这条 WS。
- Linux 运行前提（`headless-linux-server.md`）：glibc ≥ 2.31；`orca serve` 无 `DISPLAY` 时**自动拉起 Xvfb（:99）**，`DISPLAY` 存在且可用则复用、锁文件指向死进程则拒启（:12-15）；Electron 共享库清单（:27-46）；软件渲染建议 `LIBGL_ALWAYS_SOFTWARE=1`（:105）。

### 1.5 orca 浏览器运行时的 profile 持久化与环境元数据来源

- **内嵌浏览器** = Electron guest webContents（真实 Chromium 引擎），per-worktree 一个 pane、tabs 按 worktree 作用域（overview.mdx:7-25）。serve/headless 下自动化浏览器 provider 启动序：**先装好的 orca Electron，后 `ORCA_BROWSER_EXECUTABLE` 指定的外部 Chromium**（CDP 驱动），后者 profile 目录 `<statePath>/browser-chromium`（`src/main/orcad/orcad-browser-provider.ts:80-190`、`external-chromium-browser-session.ts:80-87`）。
- **Browser-use profiles**（profiles.mdx）：`Settings → Browser → Profiles`，可种子 cookies 与 viewport；每 profile 独立 storage partition（"cookies, local storage, cache. Profiles don't leak into each other"，:29-31）；agent 驱动的浏览器命令继承当前 profile（:27）。可从 Chrome/Edge/cookie 文件导入 cookies，**Google 域 cookie 明确排除**（:21）。
- **UA**：进程级二选一——"cleaned"（默认，`src/shared/browser-user-agent-mode.ts:14` `appliedMode: 'clean'`）剥离 `Electron/<ver>` 与 app token，保 Chrome 形（`browser-process-user-agent.ts:21-28,38-52`，在 `app.ready` 前设 `app.userAgentFallback`）；"native" 保留 Electron 原始 UA。**Google 登录域例外**：导航到 Google auth host 时整个 WebContents 换成平台一致的 Firefox UA 并 `stripClientHints`（剥 `sec-ch-ua*`，`browser-google-auth-ua.ts:38-64`、`browser-manager-auth-user-agent.test.ts:87-138`）。文档明言 cleaned "does not make the embedded browser identical to Chrome"（profiles.mdx:17）。
- **locale/TZ**：orca 不设 `--lang`、不覆写 locale/TZ（`src/main/browser/` 无 `getLocale/Accept-Language/timezone` 调用点）。即 Chromium/Electron 默认：Linux 上 app locale 由 `LC_ALL → LC_MESSAGES → LANG` 环境解析（Electron 官方文档 `app.getLocale()`，来源 S9），`Accept-Language` 与 JS `Intl` 由该 locale 派生；时区来自进程环境（TZ / /etc/localtime）。**iso-cc 的 R2 注入因此天然传导到浏览器**——这是"零额外动作"面。
- **遥测**（若关心会话内外发连接）：匿名 PostHog（US），无内容/无 IP/无主机名，`DO_NOT_TRACK=1` 或 `ORCA_TELEMETRY_DISABLED=1` 关闭（telemetry.mdx）。会话内可声明式注入这两个 env。

### 1.6 claude code 的 OAuth 登录流（一手：claude-code 2.1.263）

对本机安装二进制（nix store `claude-code-2.1.263`）抽串核验：

- callback：`redirect_uri = http://localhost:${port}/callback`（配置 `MANUAL_REDIRECT_URL` 时替换）——**本地 loopback HTTP 回调**；
- PKCE：`code_challenge` + `code_challenge_method=S256`；grant `authorization_code`；
- authorize 端点：`https://platform.claude.com/oauth/authorize`（console）与 `https://claude.com/cai/oauth/authorize`（claude.ai 订阅线，orca 的 `--claudeai` 即此线）；
- 打开浏览器：Linux 走 `xdg-open <url>`，无 DISPLAY 时报 `no_display`；`process.env.BROWSER` 参与解析（远程场景有 caps.browser 注入）；URL 打不开时终端呈现"open this URL"手工粘贴路径。

R12 含义：`/login` 的浏览器只要与 cc **同 netns**，callback 到 `localhost:<port>` 直达 cc 的本地 listener；orca 终端里跑 `/login` 时，URL 经 orca terminal link routing 可一键进 orca 内嵌浏览器（`docs/site/content/docs/terminal.mdx:25`：本地 web 链接提供 **Orca Browser / System Browser** 选项；`browser/overview.mdx:37-44` link routing 默认可配）——浏览器在会话内、出口在 egress、回环同 netns，三点闭合。
**登录事件与后续 API 流量的 IP 一致性是否为账号侧信号：[未验证]**——Anthropic 未公开账号风控建模，本文不做任何风控承诺（与 REQUIREMENTS §六非目标一致）。

---

## Q2 指纹面清单与一致性方法学

### 2.1 指纹面 × 决定因素 × iso-cc（rootless netns + pasta + TZ/LANG 注入 + profile 持久化）下的状态

| # | 指纹面 | 由什么决定 | iso-cc 会话内状态 | 所需动作 |
|---|---|---|---|---|
| 1 | 出口 IP（v4/v6/QUIC） | netns 路由 + pasta egress pin | ✅ 与 cc API 同 egress（同 netns 唯一出口） | 无（R1/R8 已保证） |
| 2 | TLS JA3/JA4、HTTP/2 SETTINGS | Chromium 网络栈（BoringSSL）+ 二进制版本 | ✅ 会话内恒定（同一 orca/Electron 二进制）；server-hosted 页 = 会话内栈 | 无；探针留档即可 |
| 3 | UA / OS platform | 二进制 + identity 模式（clean/native）+ Google-auth 覆写 | ✅ 稳定；默认 clean（Chrome 形，无 Electron token） | 文档固定 clean；Google 登录场景知悉 Firefox UA 覆写 |
| 4 | Client Hints（`sec-ch-ua*`） | Chromium 默认发送 | ✅ 稳定（Google-auth host 例外剥离） | 探针留档 |
| 5 | Accept-Language | app locale ← `LC_ALL/LC_MESSAGES/LANG` env | ✅ 随 R2 注入传导 | 无 |
| 6 | TZ（JS Date/Intl） | 进程 TZ env / /etc/localtime | ✅ 随 R2 bind+env 传导 | 无 |
| 7 | WebRTC/mDNS 本地地址 | 接口枚举 + ICE 策略 + UTS hostname | ⚠️ orca 对 routed guest 强制 `disable_non_proxied_udp`（fail-closed，`browser-route-webrtc-policy.ts:8-13`，应用失败即关 guest）；候选面被压缩进 netns；**UTS 未隔离 → hostname 为宿主名**（现有 R12 规格已列 T5 探针） | 探针验证；hostname 泄漏属已知缝隙 |
| 8 | canvas/WebGL renderer | GPU 栈：headless 建议 `LIBGL_ALWAYS_SOFTWARE=1` → SwiftShader | ✅ 跨会话稳定（同宿主同软件栈），但与真 Chrome（硬件 GPU）**不同且可辨** | 接受为已知差异（§非目标：非伪装）；探针留档保稳定 |
| 9 | fonts | fontconfig / 宿主字体集（mountns 未隔离 /usr/share/fonts，R3 放行） | ✅ 同宿主跨会话稳定；与个人桌面机不同 | 无；探针留档 |
| 10 | profile 持久化（cookies/设备标识/TLS ticket） | `<userData>/Partitions/<profile>/` | ⚠️ 默认落宿主 `~/.config/orca`——**必须纳入 R4 声明式重定向** | 把 `~/.config/orca/` 加入 redirect 列表 |
| 11 | 账号凭据落点 | `<userData>/claude-accounts|claude-runtime-auth/`、cc 侧 `~/.claude` | ✅ 随 #10 与 R4 `~/.claude` 重定向同域持久 | 同 #10 |
| 12 | 屏幕几何/设备像素比 | Xvfb `-screen 1280x1024x24`（orca 自启）或 `--screen` 显式 | ✅ 固定值即可稳定 | 会话固定 Xvfb 参数（orca 默认已定） |
| 13 | orca 遥测外联 | PostHog（匿名，无 IP 采集，但出联本身存在） | ✅ 经 egress 出域 | 可选注入 `DO_NOT_TRACK=1`/`ORCA_TELEMETRY_DISABLED=1` |

天然一致（无需动作）：1、2、5、6、9、12。需要动作：10（重定向）、13（可选 env）。已知缝隙/接受项：7 的 UTS hostname、8 的软件渲染可辨性。**没有任何一项需要 orca 感知沙箱**。

### 2.2 R12 验收候选探针（全部可在会话内/宿主侧执行，T5 R12 子矩阵）

前提探针页（会话内自起，兼测 loopback）：

```bash
# 会话内：本地探针页（输出 TZ/Intl/UA/lang/屏幕/canvas/WebGL/candidates）
cat > /tmp/fp-probe/index.html <<'EOF'
<pre id=out></pre>
<script>
const c=document.createElement('canvas').getContext('webgl');
const dbg=c&&c.getExtension('WEBGL_debug_renderer_info');
const out={
  tz:Intl.DateTimeFormat().resolvedOptions().timeZone,
  tzOffset:new Date().getTimezoneOffset(),
  locale:Intl.DateTimeFormat().resolvedOptions().locale,
  languages:navigator.languages, ua:navigator.userAgent,
  screen:[screen.width,screen.height,devicePixelRatio],
  gl:dbg?c.getParameter(dbg.UNMASKED_RENDERER_WEBGL):'n/a'
};
const pcs=new RTCPeerConnection();const cand=[];
pcs.onicecandidate=e=>e.candidate&&cand.push(e.candidate.candidate);
pcs.createDataChannel('x');pcs.createOffer().then(o=>pcs.setLocalDescription(o));
setTimeout(()=>{out.ice=cand;document.getElementById('out').textContent=JSON.stringify(out,null,1)},3000);
</script>
EOF
(cd /tmp/fp-probe && python3 -m http.server 8099) &
```

| # | 探针 | 命令（会话内除注明外） | 断言 |
|---|---|---|---|
| P1 | 浏览器出口 = egress | `orca goto https://ifconfig.me --worktree active --json && orca snapshot --worktree active --json` | 页面 IP == `curl -4 ifconfig.me` == cc API egress；`curl -6` 失败/同出口（R1 复用） |
| P2 | Accept-Language 头 | `curl -s https://<echo 服务>/headers` 不可用（那是 curl 的头）；改为浏览器面：探针页加 `fetch('https://httpbin.org/headers').then(r=>r.json()).then(j=>…)` 或对比 P5 的 `navigator.languages` | 头与 `navigator.languages` 同源且 == 声明 locale |
| P3 | TZ/Intl | `orca goto http://127.0.0.1:8099 && orca snapshot` | `tz` == 声明 TZ；`locale`/`languages` == 声明 locale；宿主同页(宿主浏览器)不同 |
| P4 | UA/CH | 同 P3（`ua` 字段）+ 任一 `sec-ch-ua` echo 页 | 含 `Chrome/<n>`；**不含** `Electron/`、`Orca`（clean 模式） |
| P5 | WebRTC/mDNS | 同 P3（`ice` 字段） | 无 `srflx`/`prflx` 候选（UDP 关闭）；候选仅 `host`/relay 或空；`.local` mDNS 名不含宿主 hostname 明文 |
| P6 | canvas/WebGL 稳定 | 两次会话各跑一次 P3 | `gl` 与 canvas hash 逐字节一致（预期 SwiftShader 类输出） |
| P7 | 字体稳定 | 两次会话 `fc-list \| sha256sum`（会话内） | 哈希一致 |
| P8 | profile 持久化 | 会话 A：`orca goto <固定 origin> `+ 种 cookie/localStorage（或 orca fill 登录一测试站）；kill；会话 B：重复 goto | 值存活；落盘位于 `$ISO_CC_PROFILE/orca-config/Partitions/<profile>/`（见修订建议）；`find ~ -newer <marker>`（T4 写集探针）⊆ 声明重定向集 ∪ 已知临时物 |
| P9 | OAuth 回环闭合 | 会话内 orca 终端跑 `claude` → `/login`，URL 用 Orca Browser 打开并完成 | 重定向后的 `~/.claude/.credentials.json` 出现；宿主 `~/.claude` 无变化；期间 `orca network --limit 50` 可见浏览器外联均经隧道 |
| P10 | 宿主浏览器零参与 | P9 全程宿主侧 `pgrep -c 'chrome\|chromium\|firefox'` 前后快照 + 无新窗口 | 增量为 0；`~/.local/bin/orca` shim 存在性变化被记录为有界例外（见修订） |
| P11 | 外部拨入 | 宿主：`--publish 127.0.0.1:<hp>:6768`（或 wg0 侧）+ 会话内 `orca serve --port 6768 --pairing-address <宿主可达地址> --json`；另一台机器 orca client 配对 | ready JSON `boundEndpoint=ws://0.0.0.0:6768`；外部 client `orca status --json` 经配对返回会话内 runtime id；宿主 `ss -ltnp` 只见 iso-cc pasta 转发，不见第二条进 netns 的路 |
| P12 | TLS 栈留档（可选） | 浏览器开 `https://tls.browserleaks.com/json`（第三方，结果留档不作断言依赖） | JA3/JA4 跨会话一致 |

注：P2 依赖公网 echo 服务（httpbin.org 等），离线环境可用会话内起 TLS echo；P12 为第三方服务，仅留档。所有探针不依赖 orca 之外的浏览器。

---

## 对 docs/REQUIREMENTS.md R12 的修订建议（逐条、可执行）

不改需求本体语义；以下为验收与规格要点的替换/增补文本建议。

**A. 验收逐条修订**

1. 原「登录浏览器出口 IP = egress」→ 细化为三面断言（P1）：会话内 `curl -4 ifconfig.me` == orca 浏览器探针页出口 == cc API 出口；`-6`/QUIC 不直连（引用 R1/R8 既有矩阵）。
2. 原「浏览器 TZ/Accept-Language 与声明一致」→ 绑定探针（P2/P3）：`Intl.DateTimeFormat().resolvedOptions().timeZone`、`navigator.languages`、Accept-Language 头三方一致且等于声明值。
3. 原「跨会话浏览器指纹稳定（profile 持久化）」→ 绑定探针（P4/P6/P7/P8）：UA/canvas/WebGL/fonts 两次会话逐字节一致；cookie/localStorage 跨会话存活且落盘在 profile 重定向集内；T4 写集断言扩展覆盖 `~/.config/orca`。
4. 原「宿主浏览器零参与登录」→ 绑定探针（P10）：宿主浏览器进程数增量 0；`~/.local/bin/orca` shim 与 Xvfb 为**显式登记的有界例外**（N3 表追加）。
5. 原「orca client (Tailscale) 可连入会话内 orca serve」→ 绑定探针（P11），并补断言：宿主侧不存在绕过 `--publish` 的第二条入站路径（`ss`/`nft` diff 为空，N3 复用）。
6. **新增验收（f）OAuth 回环**（P9）：会话内完成 `/login` 后凭据落在重定向 profile；callback 走 `localhost`；浏览器外联全经 egress。

**B. 规格要点修订（how + 理由）**

1. **入站发布**：明确 `--publish [host-ip:]hostPort:netnsPort/TCP` 映射到 pasta `-t/-u`；理由：serve 已绑 `0.0.0.0`（STA-2370），无需或不应改 orca 的 bind；`--pairing-address` 语义为纯通告（引 headless 文档原句），填宿主侧可达地址。拒绝方案：改 orca 配置/内核 netns 端口重定向（root + 持久规则，违反 R5/R6）。
2. **display 策略**：会话默认**不复用宿主 `DISPLAY`**（若共享 X socket，浏览器渲染进程仍在 netns 但 GPU/剪贴板面与宿主耦合，且宿主 X 的存在会诱导 orca 跳过自起 Xvfb）；默认让 orca 自起 Xvfb，doctor 新增检查：`Xvfb` 在会话 PATH、会话内无 DISPLAY。理由：把渲染栈收敛进会话可定义集（P6 稳定性前提）。
3. **声明式重定向扩列（R4 同机制）**：`~/.config/orca/`（userData：Partitions、claude-accounts、claude-runtime-auth、codex-accounts 等；claude-plugin 型 skills 落 `~/.claude/plugins`，已被 R4 覆盖；orca 自有 skills 存储走 userData 侧 RPC `skills.install`——精确集合以 T4 写集探针实测定界，不预先穷举）。已知临时物（写集断言白名单）：`$TMPDIR/orca-account-add-claude-*`（设计内自清理）、启动期 `~/.local/bin/orca` dispatcher（源码 `src/main/cli/linux-bare-orca-dispatcher.ts`，自述幂等且不覆盖用户同名文件——**P3 需实测会话 kill 后是否残留**，若残留则登记例外或由 iso-cc kill 路径清理）。
4. **CLAUDE_CONFIG_DIR 交互**：R4 的 unset 仅清除 ambient 值；orca account add / 多账号切换自设临时 `CLAUDE_CONFIG_DIR`，与 R4 不冲突，落点进重定向集。多账号出口同域由 netns 构造保证。
5. **浏览器身份与渲染模式基线**：identity 固定 `clean`（默认）；远程 workspace 页面如使用 orca desktop client 连入，**账号身份浏览选 Server (streamed)** 或直接在会话内 orca UI 操作；client-hosted 页面网络身份虽仍同域（socks 隧道 + `<-loopback>`），但其 TLS/GPU/字体面来自 client 设备，不算"会话内指纹"。写进 R12 文档注记，不做强制。
6. **env 注入可选项**：`DO_NOT_TRACK=1` / `ORCA_TELEMETRY_DISABLED=1` 进 profile 可选 env（声明式，doctor 回显）。
7. **探针落地**：§2.2 P1–P12 编入 T5 R12 子矩阵（N6：脚本化、CI 可重放）；第三方服务（P2 echo、P12 browserleaks）标记为"留档非断言"，断言只依赖会话内自起探针页。
8. **非目标重申**：不承诺 UA/canvas 与真 Chrome 不可区分（orca cleaned UA 与 SwiftShader 即非 Chrome 原生）；R12 目标是**同域与稳定**，不是反取证伪装。

---

## 来源

**S1. stablyai/orca 源码**（浅克隆 @ `d20cb69c48af2c7abb0651d02499025fe6899f5a`，2026-09-26；package.json v1.4.197）
- `docs/site/content/docs/remote-servers.mdx`（:8、:30、:32-38、:97、:101、:113-121、:129-135、:151-156、:218、:220-228）
- `docs/site/content/docs/ways-to-run.mdx`（:12-19、:43-61）
- `docs/site/content/docs/browser/overview.mdx`（:7-25、:29-33、:37-44、:56）
- `docs/site/content/docs/browser/profiles.mdx`（:13-31）
- `docs/site/content/docs/terminal.mdx`（:23-29 link actions）
- `docs/site/content/docs/agents/claude-code.mdx`（:7-11、:21-23）
- `docs/site/content/docs/telemetry.mdx`（全篇）
- `docs/site/content/docs/cli/reference.mdx`（:68-76、:163-199）
- `docs/reference/headless-linux-server.md`（:12-15、:27-46、:104-140、:181-230、:305-335、:864、:984-989）
- `src/cli/specs/serve.ts`（:8-32）；`src/cli/specs/account.ts`（:11-21）
- `src/cli/handlers/account.ts`（:86-163、:197-241）
- `src/main/runtime/runtime-rpc/runtime-rpc-pairing-types.ts`（:19-27）
- `src/main/runtime/pairing-endpoint.ts`（:18-31、:42-46）
- `src/main/runtime/e2ee-keypair.ts`（:1-46）
- `src/main/claude-accounts/managed-auth-path.ts`（:9）；`claude-accounts/runtime-auth/runtime-auth-file-storage.ts`（:74）
- `src/main/codex-accounts/runtime-home-service-paths.ts`（:68、:86）；`codex-managed-home-path.ts`（:18）
- `src/main/browser/browser-process-user-agent.ts`（:12-52）；`browser-google-auth-ua.ts`（:38-64）；`browser-identity-mode-store.ts`；`src/shared/browser-user-agent-mode.ts`（:14）
- `src/main/browser/browser-route-session-policy.ts`（:35-49）；`browser-client-network-route-address.ts`（:3）；`remote-browser-socks-server.ts`（:60、:279）；`local-ssh-browser-route.ts`（:1-37）；`paired-runtime-browser-network-route.test.ts`；`browser-route-webrtc-policy.ts`（:8-13）；`browser-cookie-chromium-prepare.ts`（:37-47）
- `src/main/orcad/orcad-bind-address.ts`（全篇）；`orcad-browser-provider.ts`（:80-190）；`external-chromium-browser-session.ts`（:80-87）
- `src/main/startup/configure-process.ts`（:206-222）
- `src/main/cli/linux-bare-orca-dispatcher.ts`（:20-52）；`cli-install-location.ts`（:211-226）
- `config/docker/headless-pairing/run-appimage-case.sh`（:31-35）；`headless-serve-shutdown/run-signal-case.sh`（:52-55）

**S2. onorca.dev 在线文档**（与 S1 仓库 docs/site 逐句一致性已抽样比对）
- https://www.onorca.dev/docs/remote-servers （"Remote sessions use the server's PATH, home directory, and credentials—not the client's."）
- https://www.onorca.dev/docs/ways-to-run · `/docs/browser/overview` · `/docs/browser/profiles` · `/docs/cli/reference` · `/docs/telemetry`

**S3. GitHub 仓库元数据**：`gh api repos/stablyai/orca` → stars 78,596、license MIT、pushed 2026-09-26、描述原文。

**S4. Electron 官方文档**：`app.getLocale()` / locale 解析（Linux `LC_ALL → LC_MESSAGES → LANG`）：https://www.electronjs.org/docs/latest/api/app

**S5. pasta/passt 手册**（端口转发原语依据）：https://passt.top/passt/about （`passt.1` `-t/-u` 端口转发）

**S6. Chromium 代理语法**（`<-loopback>` bypass 反转）：https://chromium.googlesource.com/chromium/src/+/HEAD/docs/proxy.md

**S7. Electron `session`/partition 落盘语义**（`Partitions/` 目录）：https://www.electronjs.org/docs/latest/api/session （`session.fromPartition`）

**S8. Tailscale 100.64.0.0/10 CGNAT 寻址**（pairing-address 语境）：https://tailscale.com/kb/1015/100.x-addresses

**S9. Chromium Linux locale env 解析**（`ui/base/l10n`，POSIX 分层）：同 S4 引文与 Chromium 源码树 docs。

**S10. Claude Code 二进制字符串证据**（本机 `/nix/store/nh4j5xkxxhl74bj487w34lyj98g50c66-claude-code-2.1.263/bin/.claude-wrapped`，2.1.263）：
- `redirect_uri http://localhost:${port}/callback`（两处）、`code_challenge`/`S256`、`authorization_code`
- `CONSOLE_AUTHORIZE_URL:"https://platform.claude.com/oauth/authorize"`、`CLAUDE_AI_AUTHORIZE_URL:"https://claude.com/cai/oauth/authorize"`
- `xdg-open` 调用与 `no_display` 失败路径、`process.env.BROWSER` 解析、"open this URL" 手工流程

**S11. docs/REQUIREMENTS.md**（本仓库，R1–R12、N1–N6、T5、非目标节）
