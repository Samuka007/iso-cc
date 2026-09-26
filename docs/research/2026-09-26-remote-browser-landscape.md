# 远程浏览器全景：沙箱内浏览器进程 + 宿主呈现的全部手段

日期：2026-09-26 ｜ 状态：研究笔记（info-only，不改需求基线 `docs/REQUIREMENTS.md`）
问题：除 orca（stablyai/orca，已知基线，另有姊妹研究）外，还有什么手段实现 R12「浏览器进程落在会话虚拟环境（netns 同出口）内、UI 呈现给宿主用户」？
方法：全部论断溯源到一手来源（GitHub 仓库/官方文档/man page/源码文件，逐条标注）；仓库 license 与活跃度经 `gh api repos/<owner>/<repo>` 于 2026-09-26 实测；溯源不到的标 `[未验证]`；定性推断标 `[INFERENCE]`。

---

## TL;DR

- **机制上只有两条显示通道**：① **AF_UNIX socket 共享/代理**（netns 不隔离文件系统路径绑定的 unix socket，`network_namespaces(7)` 原文只认领"IP 协议栈 + abstract namespace"；把宿主 Wayland/X11 socket bind-mount 进沙箱 mountns 即得**零端口、原生 headed** 的浏览器——这是与 R2/R4 同构的"再绑一条"路径）；② **像素/指令流经 TCP**（VNC/RDP/WebSocket/WebRTC，全部需要 host→netns 入站端口发布，即 R12 点名的 `--publish` 原语，pasta 原生 `-t/-u` 转发且支持端口范围）。
- **v1 最小路径是①**：Wayland socket bind-mount（Firefox 121+ 默认原生 Wayland，Chromium `--ozone-platform=wayland` 官方支持）+ X11 socket 兜底；无 `--publish` 依赖、OAuth 交互 = 宿主原生窗口体验、指纹与本地浏览器同构。WSLg 本机即为 Wayland 会话，开箱即用。
- **不需要 `--publish` 的备胎**：waypipe（Wayland 协议代理，"ssh -X for Wayland"，沙箱→宿主出站方向，GPL-3+，活跃）；再退化到 `cage+wayvnc` / `Xvfb+x11vnc+noVNC` + 单个 `--publish` 端口（宿主无图形会话时的通用解）。
- **headless 面有硬证据**：老 headless UA 带 `HeadlessChrome`、Chrome 132 起被移出主二进制（`chrome-headless-shell` 独立分发）；`--headless=new` 用与 headed 相同的二进制与 UA。**Playwright 默认 headless 用的正是 headless-shell 世系**（源码 `getExecutableName` 实证），且注入 `--hide-scrollbars`/指针类型覆写等自动化痕迹；Puppeteer 默认即 `--headless=new` 但带 `--enable-automation`。⇒ 交互式 OAuth 不走 headless；headless 只作非交互自动化支线。
- **明确排除**：Cloudflare/Menlo 云 RBI（浏览器在厂商边缘，出口与 profile 都不在用户侧，方向相反）；browserless（SSPL）；Kasm Workspaces CE（非商业条款 + 5 并发上限——但其底层 KasmVNC 组件 GPL-2.0 可直接复用）；Guacamole（JVM/Tomcat 栈对单用户会话过重）；QEMU+SPICE（GB 级镜像，KVM 需 `/dev/kvm` 特权面）；neko/webtop 整镜像嵌套 rootless podman（改用其底层组件直跑进沙箱更符合 R11 单二进制目标）。

---

## 0. 问题形式化与裁决维度

iso-cc 会话 = userns + netns（pasta 唯一出口，R1/R8）+ mountns（声明式 bind：R2 locale、R4 配置路径）。R12 要把「以账号身份的浏览器」也放进这棵进程树。任何候选方案必须回答五个维度：

1. **浏览器进程落点**：是否真实落在会话进程树内（自动共享 sandbox netns ⇒ 出口同域 by construction）——"远程"只是显示意义上的。
2. **OAuth 交互登录**：用户要输入凭据、过 2FA（R12 验收首项：宿主浏览器零参与登录）。
3. **指纹面**：真实 headed（指纹与本地无差）vs headless（UA/字体/GPU/指针特征异常面）；`--headless=new` 的改善程度。
4. **入站端口发布**：是否需要 host→netns 端口转发（`--publish` = pasta `-t/--tcp-ports`、`-u/--udp-ports`），还是走显示通道（AF_UNIX / 出站连接）零端口。
5. **pasta-only rootless 可运行性**：全部组件能否在 userns 内无特权运行、不要求 /dev/dri、/dev/kvm、外层容器特权。

辅助维度：重量/延迟、profile 持久化位置（R4 声明式路径重定向）、维护活跃度、license（R11 musl 静态二进制 ⇒ 宿主依赖越少越好）。

**网络侧事实基础（passt.1，2026-09-26 实测 [passt.top/passt/plain/passt.1]）**：
- `-t/--tcp-ports`、`-u/--udp-ports spec`：把宿主端口转发**进** guest/namespace，spec 形如 `first[-last][:[toaddr/]tofirst[-tolast]]`，支持端口范围与目的重映射，可选监听地址 `address[%interface]` ⇒ `--publish` 原语 = 对这两个选项的声明式封装。pasta 下默认 `auto`（把命名空间内已监听端口按同号转发），passt 下默认 `none`。
- `--map-host-loopback addr` / 默认 gateway 映射：沙箱内访问网关地址即达宿主（源地址呈现在宿主侧为 127.0.0.1）⇒ 沙箱→宿主的 ssh/显示代理出站连接零配置可用。
- man page 只定义 TCP/UDP 端口转发（外加 qemu 的 vhost-user socket）；AF_UNIX 不在 pasta 管辖 ⇒ 显示通道与 netns 正交，见 §1.1。

---

## 1. 分类清单

### 1.1 显示通道：宿主显示 socket 直接 bind-mount 共享（零端口路径）

**机制**：把宿主的显示通道 socket 以 bind-mount 进沙箱 mountns——Wayland：`$XDG_RUNTIME_DIR/wayland-0`（WSLg 在 `/mnt/wslg/`）；X11：`/tmp/.X11-unix/X0` + `XAUTHORITY` cookie。浏览器作为该 socket 的普通 client 渲染，窗口直接出现在宿主屏幕；输入由宿主 compositor 原生处理。**网络出口仍走 pasta netns**——AF_UNIX 不受 netns 影响，这是 by construction 的正交性。

**依据**：
- `network_namespaces(7)`（man-pages 6.19）：netns 隔离"network devices, IPv4/IPv6 stacks, routing, firewall, /proc/net, port numbers…In addition, network namespaces isolate the UNIX domain **abstract** socket namespace"——文件系统路径绑定的 unix socket 不在隔离集合内。[来源：https://man7.org/linux/man-pages/man7/network_namespaces.7.html]
- 浏览器原生 Wayland 支持为一等公民：Firefox 121（2023-12）起 Linux 上**默认**原生 Wayland（`MOZ_ENABLE_WAYLAND` 降级为关闭开关）[来源：https://www.firefox.com/firefox/121.0/releasenotes/ 、https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/121]；Chromium 官方 Ozone 文档支持 `--ozone-platform=wayland` / `chrome://flags/#ozone-platform-hint`，Wayland 后端中 browser process 为唯一 compositor client、GPU process 直接注册 buffer [来源：https://chromium.googlesource.com/chromium/src.git/+/HEAD/docs/ozone_overview.md 、https://chromium.googlesource.com/chromium/src/+/main/ui/ozone/platform/wayland/README.md]。
- 该做法的成熟集成先例：x11docker（MIT，6319★，v7.8.0，最近 push 2026-07-05 [实测]）在容器外宿主侧管理 X server/Wayland 并把 socket 提供给容器，且 wiki 专门提供"without x11docker"的手工 how-to 目录 [来源：https://github.com/mviereck/x11docker]。

| 维度 | 判定 |
|---|---|
| 浏览器落点 | 会话进程树内，Wayland/X11 client 直连宿主显示 |
| OAuth 交互 | 完整：宿主原生窗口、原生输入法/辅助功能；2FA 硬件密钥经宿主浏览器通道等同本地 |
| 指纹 | **最优**：100% 真实 headed，GPU/字体/窗口面与用户本地浏览器同构 |
| 入站端口 | **零**（显示走 AF_UNIX；OAuth callback 打沙箱内 localhost，同 netns 天然成立） |
| rootless/pasta 可运行 | 是：纯 mountns bind（R2/R4 同机制），无任何设备/特权要求 |
| 重量 | 浏览器自身；无编码/VNC 栈 |
| 条件 | 宿主须有图形会话（Wayland 或 X11）。X11 socket 共享无输入隔离（x11docker 文档长期警示的 X security 面）；Wayland 下同用户仍可枚举部分全局状态——两者都在"同用户信任边界"内，不在 R12 出口/指纹威胁面上 [INFERENCE] |

### 1.2 显示转发协议代理（仍零 `--publish`）

#### waypipe
- **机制**：Wayland 协议代理——"forwards Wayland messages and updates to file descriptors"，让应用转发"similar to `ssh -X` feasible"；buffer 经网络传输，支持 dmabuf（vulkan/gbm 零拷贝）、lz4/zstd 压缩；`--xwls` 模式配合 xwayland-satellite 兼顾 X11 client。README 明示传输走 ssh（也可裸 unix socket 转发）。
- **落点**：浏览器（Wayland client）在沙箱内连接 waypipe client；宿主侧 waypipe server 附着用户 compositor。连接方向沙箱→宿主（ssh 到宿主，经 pasta 默认 gateway 映射可达）⇒ 零入站端口。
- **安全语义**：waypipe 对 Wayland 协议"没有完整视图"，压缩/视频编码库独立于浏览器进程——比直接共享 compositor socket 多一层间接 [来源：README security 节]。
- **活跃/license**：GPL-3+（`waypipe-c` MIT），已重写为 Rust（edition 2021, ≥1.77），最后提交 2026-08-25 "Bump version" [实测 GitLab]。
- **来源**：https://gitlab.freedesktop.org/mstoeckl/waypipe （README 原文引用行见 4/6/14/31/55-87/124-128 行）。
- **条件**：宿主须有 Wayland 会话；两端各一个 waypipe 进程。

#### ssh -X / -Y（X11 forwarding）
- **机制**：X11 连接经 ssh 通道转发到宿主 X server；`-X` 为 untrusted，受 X11 SECURITY extension 约束；`-Y` trusted 绕过 [来源：https://man.openbsd.org/ssh，-X/-Y 定义原文]。
- **落点**：浏览器在沙箱内为 X11 client（需要沙箱内有 X11 库 + host 侧 XWayland）。
- **OAuth/指纹**：完整交互、真实 headed；但无 GPU 直通/省电路径，软件渲染；X11 协议同步往返密集，现代网页重绘明显慢于像素流方案 [INFERENCE，公认定性]。
- **端口**：零（沙箱出站 ssh）。
- **判定**：可用性兜底，不推荐作 v1 主路径（性能与 X11 安全面均劣于 §1.1）。

#### Xpra
- **机制**：Xvfb 内起 X server + 自有编码协议把窗口"seamless"重投到宿主 client；有 HTML5 client（须 `--bind-tcp` 监听 ⇒ 入站端口）与 ssh attach 两种接法；x264/vp9/webp 编码。
- **活跃/license**：GPL-2.0+，2987★，pushed 2026-09-26 [实测]。
- **来源**：https://xpra.org 、https://github.com/Xpra-org/xpra 。
- **判定**：比 waypipe 重（X11 世代 + 编码栈），HTML5 模式需 `--publish`；在"沙箱内 X11 应用 → 宿主 Wayland"组合里是备选。

#### x11docker（方法学目录）
- 本项目不依赖 docker，但 x11docker 的 README/wiki 系统化了"容器 GUI 的每一种 X/Wayland 接法"（socket 共享、Xvfb+VNC、Xephyr 嵌套等），作为 §1.1/§2 各路径的方法学出处引用 [来源：https://github.com/mviereck/x11docker ；MIT；v7.8.0；最近 push 2026-07-05（低速维护）[实测]]。

#### X2Go / nx-libs（NX 系）
- nx-libs（ArcticaProject 维护，3.6.x 稳态维护 + CVE 修补，3.7.x 在做 Xserver 重基）是 X11 世代的压缩显示转发；X11-only、栈老 [来源：https://github.com/ArcticaProject/nx-libs]。**判定：备查，不进推荐。**

### 1.3 compositor/像素串流（全部需要 `--publish` 入站）

| 方案 | 机制 | 组件与落点 | 入站端口 | 指纹/OAuth | 重量 | 活跃度/license [实测 2026-09-26] | 来源 |
|---|---|---|---|---|---|---|---|
| **Xvfb + x11vnc + noVNC/websockify** | 沙箱内 Xvfb 起虚拟屏，浏览器 headed 跑在其上；x11vnc 抓屏；websockify 把 VNC 转 WebSocket + mini web server 托管 vnc.html | 全部普通用户态组件；浏览器在沙箱 | VNC 5900 或 web 6080（单 TCP） | headed 真浏览器，指纹同构；OAuth 完整 | 低（各发行版均有包） | x11vnc GPL-2.0（2025-05 push，稳态）；noVNC MPL-2.0（2026-09 push）；websockify 为 noVNC 姊妹项目（README 原文） | https://github.com/novnc/noVNC 、https://github.com/novnc/websockify 、https://github.com/LibVNC/x11vnc |
| **KasmVNC** | 打破 RFB 的现代 VNC：**内建 web/WebSocket 服务器**（无需 websockify），WebCodecs H.264/H.265/AV1 + 动态 jpeg/webp，剪贴板、多用户、IME、YAML 配置 | 沙箱内配 Xvfb/Xorg 起 kasmvncserver；浏览器在沙箱 | 单 web 端口（YAML `websocket_port`，auto） | headed；OAuth 完整 | 低-中 | GPL-2.0；5276★；pushed 2026-09-23；Kasm Workspaces 的底层引擎 | https://github.com/kasmtech/KasmVNC |
| **wayvnc + cage** | cage（wlroots kiosk compositor，单应用全屏）内以原生 Wayland 跑浏览器；wayvnc 附着 compositor 出 VNC | 全部用户态、pixman 软渲染、无 DRM 设备要求；浏览器在沙箱 | VNC 5900（单 TCP） | headed；OAuth 完整 | **最低**（三个小程序） | wayvnc ISC、1825★、pushed 2026-09-25，"run without a physical display"；cage MIT、2070★，支持嵌套 virtual output | https://github.com/any1/wayvnc 、https://github.com/cage-kiosk/cage |
| **Weston RDP backend** | Weston `--backend=rdp-backend.so` 无头运行，FreeRDP 协议出桌面；pixman 软渲染、multi-seat、TLS 必需 | 沙箱内 weston + 浏览器（原生 Wayland 或 XWayland） | RDP 3389（单 TCP） | headed；OAuth 完整；客户端 xfreerdp | 低 | rdp-backend 维护回归中（backend-rdp 最近提交 2026-09-08/07-15/06-01 [实测 GitLab]）；Weston/MIT、FreeRDP/Apache-2.0 | https://man.archlinux.org/man/weston-rdp.7.en 、https://gitlab.freedesktop.org/wayland/weston |
| **Selkies**（原 selkies-gStreamer） | "GPU/CPU-accelerated HTML5 remote desktop"，**默认 WebSocket 传输、WebRTC 为 opt-in**；LinuxServer webtop 现基于其 baseimage | 沙箱内 Xvfb/桌面 + selkies 发送端；浏览器在沙箱 | HTTP(S) 8080 级单端口；启用 WebRTC 后另需 ICE/UDP 面 | headed；OAuth 完整 | 中（GStreamer 系依赖） | MPL-2.0；2214★；pushed 2026-09-26；Google 工程师起源 | https://github.com/selkies-project/selkies 、https://docs.selkies.io |
| **Sunshine + Moonlight** | GameStream 系低延迟抓屏编码（NVENC/VAAPI/软件编码）；捕获后端 KMS/DRM、Wayland(wlroots)、X11、XDG portal | 浏览器在沙箱；捕获/编码也在沙箱 | **TCP 47984/47989/47990/48010 + UDP 47998/47999/48000/48002/48010** | headed；OAuth 完整；游戏级延迟 | 高；KMS 捕获需 /dev/dri、输入注入需 uinput ⇒ 设备特权面超出 R3 最小侵入 | GPL-3.0；41.5k★；pushed 2026-09-26 | https://github.com/LizardByte/Sunshine 、https://docs.lizardbyte.dev/projects/sunshine/v0.22.2/about/advanced_usage.html 、https://www.moonlight-stream.org |
| **Apache Guacamole** | clientless 网关：浏览器 HTML5/JS + WebSocket（JSR-356，HTTP tunnel 兜底）→ Java servlet 容器(Tomcat) → guacd(C, :4822) 按 libguac 插件翻译 VNC/RDP/SSH 为自研画布指令 | 浏览器可在沙箱（经 VNC 后端） | 后端 VNC 入沙箱 + 前端 web 入宿主，多段 | headed；OAuth 完整 | **高**（JVM+Tomcat+可选 DB），对单用户会话过度设计 | Apache-2.0；TLP 级维护 | https://guacamole.apache.org/doc/gug/guacamole-architecture.html |
| **GNOME RemoteDesktop（46+ headless remote login）** | GDM + systemd + Mutter/PipeWire 的无头 RDP 登录（TCP 3389），FreeRDP3 | 浏览器在沙箱内整个 GNOME 会话中 | 3389 | headed；OAuth 完整 | **高**（完整 GNOME 栈依赖） | GNOME 46 起官方特性 | https://release.gnome.org/46/ 、https://github.com/GNOME/gnome-remote-desktop |

### 1.4 容器化远程浏览器平台：整镜像嵌套 vs 组件复用

| 平台 | 镜像内部结构（一手证据） | 端口面 | license/活跃 [实测] | 嵌套判定 |
|---|---|---|---|---|
| **m1k1o/neko** | 容器内 **Xorg + dummy 视频驱动（dummy_drv.so）+ 自研 neko 输入驱动（neko_drv.so）**（根 Dockerfile.tmpl L5/14/15）+ 浏览器 + GStreamer WebRTC 发送端 + 房间服务；per-browser 镜像 firefox/chromium/tor-browser/… | TCP 8080 + **UDP 52000-52100（NEKO_WEBRTC_EPR）**；NAT 后必须设 `NEKO_WEBRTC_NAT1TO1` 指宿主地址（compose 原文）——pasta 场景正需要该等价物 | Apache-2.0；22.4k★；pushed 2026-09-24 | 组件复用优于整镜像嵌套 |
| **Kasm Workspaces** | KasmVNC + 容器编排 + 管理面 | 由 KasmVNC 承载 | **CE：非商业专用 + 5 并发会话上限**（license 页原文）；底层 KasmVNC 组件 GPL-2.0 可自由复用 | **整体排除**；仅复用 KasmVNC |
| **linuxserver/docker-webtop** | 现基于 **docker-baseimage-selkies**（README 原文），HTTPS 3001 强制（WebCodecs 需 secure context）、3000 为反代口；PUID/PGID；部分 tag 为 Wayland 桌面 | 3000/3001 | GPL-3.0；4.4k★；pushed 2026-09-25 | 组件复用（Selkies/KasmVNC） |
| **jlesage/docker-firefox** | baseimage-gui（jlesage/docker-baseimage-gui，MIT）= Xvfb + x11vnc + noVNC 打包；web 5800 / VNC 5900（端口表原文）；USER_ID/GROUP_ID、VNC_LOCALHOST_ONLY | 5800/5900 | MIT；2.5k★；pushed 2026-09-22 | 组件复用（§1.3 第一行即是它的解包） |

**嵌套可行性结论**：外层 iso-cc 已是 userns 沙箱，再嵌 rootless podman 意味着 userns 套 userns + 容器内 overlay/fuse-overlayfs 存储栈 + 第二套运行时依赖——技术上可行（kernel 允许 userns 深度嵌套；podman 官方 rootless 模式存在，[来源：https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md]），但与 R11"单静态二进制 + 最少宿主依赖"直接冲突，且上表平台没有一个必须容器化（全是普通用户态程序组合）。**判定：不嵌套；把浏览器 + （Xvfb|cage|KasmVNC|Selkies 发送端）作为沙箱进程树内的普通进程运行。**

### 1.5 CDP / headless 自动化驱动（非交互支线）

**CDP 基础设施**：Chromium 以 `--remote-debugging-port`（HTTP `/json/version` 发现 + WebSocket）或 `--remote-debugging-pipe` 暴露 DevTools 协议；`Page.startScreencast`（帧流）与 `Input.dispatchMouseEvent/insertText`（输入注入）协议方法经 2026-09-26 实测存在于官方协议文档 [来源：https://chromedevtools.github.io/devtools-protocol/tot/Page/ 、https://chromedevtools.github.io/devtools-protocol/tot/Input/]。理论上可拼装"CDP 查看器"实现远程交互，但 UX = 在另一个浏览器里操作一个画布，无原生浏览器 chrome、无密码管理器/扩展/系统输入法面——**交互式 OAuth 体验显著降级 [INFERENCE]**。

**headless 世代（官方证据）**：
- 老 headless 是独立实现（"didn't share any of the Chrome browser code in //chrome"）；Chrome 132.0.6793.0 起**只以独立二进制 `chrome-headless-shell` 提供** [来源：https://developer.chrome.com/docs/automation-and-testing/headless]。
- Chrome 112 引入 `--headless=new`：同一浏览器二进制；UA 与常规 headed 一致（无 `HeadlessChrome` 标记）[来源：https://developer.chrome.com/blog/removing-headless-old-from-chrome]。老 headless UA 显式带 `HeadlessChrome/...`，是最易检出的指纹面（同来源 + 社区检测器如 scrapfly automation-detector 引此为首要向量）。

**驱动器默认行为（源码实证 2026-09-26）**：
- **Playwright**（`packages/playwright-core/src/server/chromium/chromium.ts`）：headless 时追加 `--headless` + `--hide-scrollbars --mute-audio --blink-settings=primaryHoverType=2,...primaryPointerType=4,...`（指针/悬停类型覆写=可探自动化痕迹）；`getExecutableName`：**默认 headless 使用 `chromium-headless-shell` 二进制**（老 headless 世系），`channel: 'chromium'` 或 headed 才用完整 chromium；默认参数**不含** `--enable-automation`，全文件无 `navigator.webdriver` 注入 [来源：同文件 L355-423 [实测]]。
- **Puppeteer**（`packages/puppeteer-core/src/node/ChromeLauncher.ts`）：默认参数**含 `--enable-automation`**（L241）；`headless: true` 默认 → `--headless=new`，`'shell'` → 老 `--headless`（L278-280）[来源：同文件 [实测]]。
- **browserless**：`SPDX-License-Identifier: SSPL-1.0 OR Browserless Commercial License`（LICENSE 原文）——SSPL 对分发型工具不利 ⇒ 排除 [来源：https://github.com/browserless/browserless LICENSE；13.7k★；pushed 2026-09-25 [实测]]。

**判定**：headless/new + CDP 是**非交互自动化支线**（e.g. 自动抓取、自动表单）；OAuth 交互首登走 §1.1/§1.3 的 headed 路径，cookie/profile 落 R4 重定向目录，后续自动流程复用该 profile。若必须 headless：选 `--headless=new` 完整二进制（Puppeteer 默认即此；Playwright 须显式 `channel: 'chromium'`），并知悉指针覆写/automation 标志等残余面。

### 1.6 企业 RBI（方向相反，仅方法学参照）

| 方案 | 架构（一手） | 为何不可用作 iso-cc 出口 | 方法学参照价值 |
|---|---|---|---|
| **Cloudflare Browser Isolation** | 无头 Chromium 容器跑在 Cloudflare 边缘（300+ PoP），内容在云内执行；渲染回传用 **Network Vector Rendering**（截获 Skia 绘制指令，令本地浏览器执行矢量指令而非收像素）；clientless 模式以 `<team>.cloudflareaccess.com/browser/<URL>` 进入；会话结束容器销毁 | 浏览器进程不在用户进程树、**出口 IP = Cloudflare**、**profile 一次性即毁**——与 R12"同 egress + profile 跨会话稳定"两条验收直接矛盾 | ①NVR"绘制指令流"是像素流之外的第二条串流路线（对应 §1.3 与 Guacamole 指令协议的同类思想）；②一次性浏览器 session = 反面教材：R12 恰好要 profile 持久 |
| **Menlo Security** | Isolation Core™ 在云内执行全部活动内容，Adaptive Clientless Rendering™ 回传"安全视觉/HTML 元素" | 同上：出口与执行都在厂商云 | ACR = DOM/视觉重建路线参照；与像素流（VNC/Selkies）和指令流（NVR/Guacamole）并列第三条路线 |

来源：https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/ 、https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/canvas-remoting/ 、https://blog.cloudflare.com/browser-isolation-private-network/ 、https://www.menlosecurity.com/product/remote-browser-isolation 。

### 1.7 其他类别

- **QEMU + SPICE（VM 路径）**：KVM 加速需 `/dev/kvm` 读写权（root/kvm 组/ACL）；**TCG 纯软件模拟完全用户态无特权** [来源：https://www.qemu.org/docs/master/system/introduction.html]；SPICE 协议 + remote-viewer 客户端 [来源：https://www.spice-space.org]。判定：rootless 不阻塞，但 GB 级镜像 + TCG 性能 + 整套 VM 生命周期管理对 R12 的"登录一个浏览器"过度；仅在出现"浏览器 0-day 逃逸防线"级别新需求时重评。
- **PipeWire screencast 自研（零端口视频）**：沙箱内 cage/wlroots compositor + xdg-desktop-portal-wlr（实现 `org.freedesktop.impl.portal.ScreenCast`，README 原文 [来源：https://github.com/emersion/xdg-desktop-portal-wlr，MIT，pushed 2026-08-13 [实测]]）产生 PipeWire 视频流；PipeWire 走 AF_UNIX，与 netns 正交（§1.1 同依据），可 bind-mount 出沙箱由宿主 `gst-launch pipewiresrc` 消费——**视频面零端口**。缺口：wlr portal 只有 screencast 无输入回注；沙箱内还需 DBus session。**无现成"跨沙箱 PipeWire 浏览器查看器"项目 [未验证]**；列为 P3+ 实验向，非依赖路径。
- **GTK Broadway**（HTML5 显示后端）只服务 GTK 应用栈，浏览器（Chromium/Firefox 自绘 UI）不适用 [未验证——未找到官方"不适用清单"，按架构判定]。

---

## 2. 关键裁决点矩阵

判定符号：✅ 良好 / ⚠️ 有条件 / ❌ 不可行。入站端口列：`--publish` 需求的形状（对应 pasta `-t/-u` spec）。

| 方案 | OAuth 交互登录 | 指纹面 | `--publish` 需求 | pasta-only + rootless userns 可运行 | profile 持久化 |
|---|---|---|---|---|---|
| 显示 socket bind-mount | ✅ 宿主原生窗口 | ✅ 真实 headed | **无需** | ✅ 纯 mountns bind | 沙箱 fs（R4 同机制） |
| waypipe | ✅ 宿主原生窗口 | ✅ 真实 headed | **无需**（出站 ssh） | ✅ 两端用户态 | 沙箱 fs |
| ssh -X | ✅（性能差） | ⚠️ headed 但软件渲染 | **无需** | ✅ 需沙箱 X11 库 | 沙箱 fs |
| Xpra（ssh attach） | ✅ | ⚠️ headed + 编码 | 可零端口（ssh）；HTML5 模式需单 TCP | ✅ | 沙箱 fs |
| Xvfb+x11vnc+noVNC | ✅ | ✅ headed | 单 TCP（5900/6080） | ✅ 全组件无特权 | 沙箱 fs |
| KasmVNC | ✅ | ✅ headed | 单 TCP（web/ws） | ✅ | 沙箱 fs |
| cage + wayvnc | ✅ | ✅ headed | 单 TCP（5900） | ✅ pixman 软渲染 | 沙箱 fs |
| Weston RDP | ✅ | ✅ headed | 单 TCP（3389） | ✅ | 沙箱 fs |
| Selkies | ✅ | ✅ headed | 默认单 TCP；WebRTC 后加 UDP ICE 面 | ⚠️ GStreamer 依赖集较大 | 沙箱 fs |
| Sunshine+Moonlight | ✅ | ✅ headed | **多端口**（4×TCP + 5×UDP） | ⚠️ 捕获/输入涉 /dev/dri、uinput | 沙箱 fs |
| Guacamole | ✅ | ✅ headed | 多段（网关侧另需部署面） | ⚠️ JVM/Tomcat 宿主依赖重 | 沙箱 fs |
| GNOME RemoteDesktop | ✅ | ✅ headed | 单 TCP（3389） | ❌ 需完整 GNOME 栈 | 沙箱 fs |
| neko（组件复用） | ✅ | ✅ headed（容器内真浏览器） | TCP 1 + **UDP 范围**（EPR）+ NAT1TO1 | ⚠️ WebRTC/NAT 配置面 | 沙箱 fs |
| neko/webtop 整镜像嵌套 | ✅ | ✅ headed | 同上 | ❌ 违反 R11（嵌套运行时栈） | 容器卷 |
| Playwright/Puppeteer + CDP（headless=new） | ❌ 交互体验崩坏（DevTools/画布） | ⚠️ UA 正确但自动化痕迹（指针覆写/enable-automation/headless-shell 默认） | 单 TCP（9222）或 pipe | ✅ | 沙箱 fs |
| browserless | ❌（自动化定位） | ⚠️ 同 headless | 单 TCP | ⚠️ SSPL license | 服务端管理 |
| Cloudflare/Menlo RBI | ✅（厂商产品化） | ❌ 厂商边缘指纹，非同域 | —（方向相反） | ❌ **出口/执行在厂商云** | 厂商侧一次性 |
| QEMU+SPICE | ✅ | ✅ headed | 单 TCP（SPICE） | ⚠️ TCG 可行但重；KVM 需 /dev/kvm | VM 磁盘 |
| PipeWire screencast（自研） | ⚠️ 输入回注缺口 | ✅ headed | **视频零端口**（AF_UNIX） | ⚠️ 需 DBus + portal；[未验证] 无现成实现 | 沙箱 fs |

---

## 3. 对 iso-cc R12 的推荐路径排序

先决注记：`--publish` 原语（pasta `-t/-u` 封装）本就是 P3 待实现项（REQUIREMENTS §五 P3 行）；§1.1 路径连它都不需要。所有路径中浏览器 profile 均落 `~/.local/state/iso-cc/<profile>/...`（R4 声明式重定向同机制扩展），满足"跨会话稳定"。

### v1 最小路径（P3 到位时第一批实现）

1. **宿主显示 socket bind-mount（Wayland 优先）**：mountns 声明式 bind 清单加一条 `$XDG_RUNTIME_DIR/wayland-0`（或 WSLg `/mnt/wslg/wayland-0`）+ `XDG_RUNTIME_DIR` 指向；浏览器 `--ozone-platform=wayland`（Firefox 默认）。零新网络原语、零编码栈、指纹最优、OAuth UX=原生。doctor 项：检测 Wayland session / socket 存在 / 浏览器 Wayland 可用。
2. **兜底 A：X11 socket bind-mount**（`/tmp/.X11-unix/X0` + XAUTHORITY）：宿主为 X11 会话时的同构路径；接受同用户 X11 无隔离边界（非 R12 威胁面）。
3. **兜底 B：cage + wayvnc + `--publish 5900`**：宿主无图形会话（SSH-only 服务器）或不信任 socket 共享时；三个小程序、pixman 软渲染、单 TCP 端口。等价替换：Xvfb+x11vnc+noVNC + `--publish 6080`（获得浏览器即客户端 + 音频剪贴板可后补 KasmVNC）。
4. **可选显示代理变体：waypipe**：希望"沙箱进程连 compositor socket 都拿不到"时的强隔离版 §1.1（沙箱→宿主 ssh 出站，零 `--publish`）。

### P3 完整路径（体验/能力升级）

- **KasmVNC 替换 noVNC**（单端口 web、剪贴板、动态分辨率、多用户权限面），或 **Selkies**（默认 WebSocket 单端口；要 60fps+ 时开 WebRTC：`--publish` UDP ICE 范围 + 发布宿主侧 candidate 地址——即 neko `NEKO_WEBRTC_NAT1TO1` 的等价问题）。
- **orca serve 基线同域**：`orca serve` 属会话进程树即满足 by-construction；外部 client 拨入靠 `--publish`（R12 已点名）。
- **非交互自动化支线**：CDP + `--headless=new` 完整二进制（Puppeteer 默认；Playwright 显式 `channel: 'chromium'`）+ `--publish 9222`；首登 cookies 由 headed 路径写入 R4 profile。

### 排除清单（含理由）

- **Cloudflare BI / Menlo**：出口与执行在厂商云，违反 R12 同域 by construction。
- **browserless（SSPL）/ Kasm Workspaces CE（非商业 + 5 并发）**：license 与分发目标冲突；取其开源底层（KasmVNC GPL-2.0）。
- **Guacamole**：JVM/Tomcat/guacd 三层栈对单用户登录场景过重，违反 R11 精神。
- **QEMU+SPICE**：重量与 KVM 特权面；TCG 可行但无必要。
- **Sunshine**：游戏级体验但 9 端口面 + /dev/dri、uinput 设备要求，超出 R3 最小侵入；仅在"远程桌面级富交互"P3+ 需求时重评。
- **GNOME RemoteDesktop / webtop / neko 整镜像**：栈重或嵌套运行时违反 R11；组件（KasmVNC/Selkies/Xvfb 系）已按 §1.4 复用。

### 指纹注记（跨路径）

真实 headed 路径（§1.1/§1.2/§1.3 全部）指纹等价于用户本地浏览器；R2 的 TZ/LANG 覆盖保证 Intl/Accept-Language 一致。残余差异面：**字体集**（沙箱 fontconfig 若与宿主不同，canvas/文本测量哈希漂移——建议 R4 同机制声明式 bind 宿主字体目录 [INFERENCE]）与 **hostname/WebRTC 本地地址**（R12 已列为 T5 探针项）。headless 支线残余面见 §1.5。

---

## 4. 来源列表

**内核/协议基础**
- network_namespaces(7)：https://man7.org/linux/man-pages/man7/network_namespaces.7.html
- passt(1)（-t/-u spec、map-gw/map-host-loopback）：https://passt.top/passt/plain/passt.1
- ssh(1) -X/-Y：https://man.openbsd.org/ssh
- unix(7)（abstract namespace 对照）：https://man7.org/linux/man-pages/man7/unix.7.html

**显示通道共享/代理**
- x11docker：https://github.com/mviereck/x11docker
- waypipe：https://gitlab.freedesktop.org/mstoeckl/waypipe
- Xpra：https://xpra.org 、https://github.com/Xpra-org/xpra
- ArcticaProject/nx-libs（X2Go）：https://github.com/ArcticaProject/nx-libs

**浏览器原生 Wayland**
- Firefox 121 release notes：https://www.firefox.com/firefox/121.0/releasenotes/ 、https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/121
- Chromium Ozone：https://chromium.googlesource.com/chromium/src.git/+/HEAD/docs/ozone_overview.md 、https://chromium.googlesource.com/chromium/src/+/main/ui/ozone/platform/wayland/README.md

**VNC/RDP/串流组件**
- noVNC：https://github.com/novnc/noVNC ｜ websockify：https://github.com/novnc/websockify ｜ x11vnc：https://github.com/LibVNC/x11vnc
- KasmVNC：https://github.com/kasmtech/KasmVNC
- wayvnc：https://github.com/any1/wayvnc ｜ cage：https://github.com/cage-kiosk/cage
- Weston RDP backend：https://man.archlinux.org/man/weston-rdp.7.en 、https://gitlab.freedesktop.org/wayland/weston
- Selkies：https://github.com/selkies-project/selkies 、https://docs.selkies.io
- Sunshine 端口表：https://docs.lizardbyte.dev/projects/sunshine/v0.22.2/about/advanced_usage.html ｜ https://github.com/LizardByte/Sunshine ｜ Moonlight：https://www.moonlight-stream.org
- Guacamole 架构：https://guacamole.apache.org/doc/gug/guacamole-architecture.html
- gnome-remote-desktop：https://github.com/GNOME/gnome-remote-desktop 、https://release.gnome.org/46/

**容器平台**
- neko：https://github.com/m1k1o/neko （compose 端口/`Dockerfile.tmpl` dummy 驱动为 repo 原文）
- Kasm Workspaces license：https://www.kasmweb.com/docs/develop/license.html
- linuxserver/docker-webtop：https://github.com/linuxserver/docker-webtop
- jlesage/docker-firefox：https://github.com/jlesage/docker-firefox ｜ baseimage-gui：https://github.com/jlesage/docker-baseimage-gui
- podman rootless tutorial：https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md

**CDP/headless**
- Chrome Headless mode 官方文档：https://developer.chrome.com/docs/automation-and-testing/headless
- 移除老 headless（Chrome 112 引入 new、132 起 shell 独立）：https://developer.chrome.com/blog/removing-headless-old-from-chrome
- chrome-headless-shell：https://developer.chrome.com/docs/automation-and-testing/headless-chrome-shell
- DevTools Protocol（Page.startScreencast / Input.dispatchMouseEvent，2026-09-26 存在性实测）：https://chromedevtools.github.io/devtools-protocol/tot/Page/ 、https://chromedevtools.github.io/devtools-protocol/tot/Input/
- Playwright 源码（defaultArgs/getExecutableName）：https://github.com/microsoft/playwright/blob/main/packages/playwright-core/src/server/chromium/chromium.ts
- Puppeteer 源码（DEFAULT_ARGS）：https://github.com/puppeteer/puppeteer/blob/main/packages/puppeteer-core/src/node/ChromeLauncher.ts
- browserless LICENSE（SSPL-1.0 OR Commercial）：https://github.com/browserless/browserless

**企业 RBI（方法学参照）**
- Cloudflare Browser Isolation：https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/ 、https://developers.cloudflare.com/cloudflare-one/remote-browser-isolation/canvas-remoting/ 、https://blog.cloudflare.com/browser-isolation-private-network/
- Menlo Security RBI：https://www.menlosecurity.com/product/remote-browser-isolation

**VM/PipeWire**
- QEMU（KVM /dev/kvm vs TCG 用户态）：https://www.qemu.org/docs/master/system/introduction.html ｜ SPICE：https://www.spice-space.org
- xdg-desktop-portal-wlr（ScreenCast portal）：https://github.com/emersion/xdg-desktop-portal-wlr

**仓库元数据（license/活跃度/stars）**：`gh api repos/<owner>/<repo>`，2026-09-26 [实测]，结果内嵌于 §1 各表。
