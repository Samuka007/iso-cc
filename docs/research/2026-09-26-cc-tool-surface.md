# cc 全量工具面盘点：执行位置 × 网络面 × netns/locale/config 敏感性

日期：2026-09-26 ｜ 状态：研究笔记（info-only，不改需求基线 `docs/REQUIREMENTS.md`）
前置：`2026-09-26-related-projects-and-purity-probes.md`（P1–P12、2c 环境取证）、`2026-09-26-community-risk-signals.md`（P13–P15）、`docs/adr/0008-net-scope-declaration.md`（D8 tree/self）
版本锚点：**claude-code 2.1.263**（本机 nix store，Bun 单文件二进制，字符串取证）＋ 官方文档（code.claude.com/docs，跟踪 ~2.1.283）。差异逐项标注。

---

## TL;DR

1. **内建工具的网络面比直觉窄**：绝大多数工具（Read/Edit/Write/NotebookEdit/Task*/Cron*/plan 类）是 cc 进程内 Node fs/UI 操作，零网络。真正出网的模型面只有四类：模型 API、WebSearch（**服务端工具**，搜索在 Anthropic 后端执行，cc 只发 API 请求）、WebFetch（cc 进程客户端 fetch + api.anthropic.com 域名 preflight）、以及 claude.ai 平台工具（Artifact/RemoteTrigger/PushNotification/SendUserFile）。
2. **stdio MCP 全链路在 netns 内成立**：stdio server 是 cc 直接 spawn 的子进程（非 shell），stdio 管道不受 netns 影响；其出网（多数 server 需要）与安装流量（npx/uvx 拉包 → registry.npmjs.org）都走会话隧道。HTTP/SSE 同理走隧道。**唯一结构性缺口是 loopback**：本地 HTTP MCP server、IDE 扩展 WebSocket、自建 OTel collector 都以 `127.0.0.1:<port>` 为目标，cc 侧不可配置——map-host-loopback/会话内中继的必要性获三方佐证。
3. **IDE 集成方向已取证澄清**：VS Code 扩展是 **WS 服务端**（`ws://127.0.0.1:<random 10000–65535>`，token 落 `~/.claude/ide/<port>.lock`），cc 是**客户端**。cc 进 netns 后连宿主 loopback 断裂 → IDE 集成降级；无需担心反向入站。候选适配：会话内 `127.0.0.1:port → 网关地址:port` 中继（socat/内置），port 无需改写。
4. **Hooks 不走 `$SHELL`**：command hook 由硬编码 `/bin/sh -c` 执行 [二进制实测]，REPL `!` 前缀命令、skills 的 `` !`cmd` `` 动态注入、statusline 命令同理。**self 范围下这些面留在 netns、不透传**（与 D8 例外表一致，但必须写成显式声明而非默认假设）；tree 下无差异。官方 `CLAUDE_CODE_SHELL_PREFIX`（覆盖 Bash 调用、hook、statusline、stdio MCP 启动）是比 `SHELL` 更完备的拦截层候选。
5. **config 敏感面基本落入现有 R4 重定向集**（`~/.claude` + `~/.claude.json`），但存在三个**集合外写点**：`autoInstallIdeExtension` 写 `~/.vscode/extensions`、Chrome native messaging manifest 写浏览器配置目录、managed settings 读 `/etc/claude-code/managed-settings.json`（宿主系统路径，读方向）——写集探针 P8 会把它们抓出来，需登记有界例外或用 env 关闭。
6. **T5 增补 12 条**（P16–P27，§7）：MCP 三传输子矩阵、hooks/statusline 执行位置、IDE lockfile、OAuth loopback 闭合、Monitor WebSocket、平台流量、OTel collector、bwrap 嵌套沙箱、跨会话 UDS、写集例外回归。
7. 版本漂移显著（Glob/Grep 默认缺席、MCP `ws` 传输、skills claude.ai 同步等均 ≠ 2.1.263）→ L4 清单按 version pin 维护（§8）。

---

## 0. 方法与依据

- **二进制取证**：`/nix/store/nh4j5xkxxhl74bj487w34lyj98g50c66-claude-code-2.1.263/bin/.claude-wrapped`（215 MB Bun 编译单文件），`grep -a` 字符串级取证，标 `[实测]`。
- **官方文档**：code.claude.com/docs 各页（§9 全列），docs 描述最新版（~2.1.283），与 2.1.263 的差异标 `版本` 或 `[未验证]`。
- **五元组定义**：①执行位置（in-process / fork 子进程 / RPC 外部服务）②网络面（无 / 直连目的地 / 经第三方 API / 本地 loopback）③netns 敏感性（tree 行为；self 是否经 bash RPC 透传——**只有 Bash 工具的 `$SHELL -c` 走透传**，其余一切 fork 子进程随 cc 本体留在 netns）④locale/config 敏感性（读 TZ/locale；读写 `~/.claude*` 或其他路径）⑤验收缺口（是否需要新 T5 探针/netns 适配）。

---

## 1. 全量工具表 A：模型内建工具（tools reference）

> 工具名以官方文档现行名为准；2.1.263 二进制内旧名并存标注 [实测]。

| 工具 | ①执行位置 | ②网络面 | ③netns 敏感性 (tree/self) | ④locale/config | ⑤验收缺口 |
|---|---|---|---|---|---|
| `Bash`（后台 `run_in_background` → `TaskOutput`/`TaskStop`；2.1.263 内旧名 `BashOutput`/`KillShell` 并存 [实测]） | fork：`$SHELL -c`（`process.env.SHELL‖COMSPEC` [实测]；`CLAUDE_CODE_SHELL` 可指定 bash/zsh 路径） | 由所跑命令决定（git/npm/curl…） | tree：netns 内。self：**唯一透传面**（整段 argv 经 exec.sock 宿主执行，L1=`SHELL` env） | 会话启动 source 用户 rc 生成 shell snapshot（写 `~/.claude/shell-snapshots/`）；env 块注入 `Platform`/日期/git 状态 | P1–P5 已覆盖 bash 内出网；self 保真度（复合命令/后台任务/交互 TUI）= D8 已列实测定界项 |
| `PowerShell` | fork（Windows） | 同上 | Linux 不适用 | — | 不适用（Linux 目标） |
| `Monitor` | fork（后台命令，权限规则同 Bash；cgroup kind 同 Bash）＋ **cc 进程内 WebSocket 客户端源**（连任意 URL） | 命令面同 Bash；ws 源 = 出站 ws | tree：均 netns。self：命令是否经 `$SHELL` 透传 [未验证]（docs 只说权限规则同 Bash）；ws 源留在 netns | deadline 5/30min；`DISABLE_TELEMETRY`/`NONESSENTIAL` 下**工具不可用**（feature-flag 依赖） | T5-P21（ws 出站走 egress）；self 透传语义实测 |
| `Read`/`Edit`/`Write`/`MultiEdit`（2.1.263 仍有 MultiEdit [实测]；现行文档已并入 Edit） | in-process（Node fs） | 无 | 无行为变化（读写共享 fs） | 无 | 无（写集由 P8 间接覆盖业务写入语义） |
| `NotebookEdit` | in-process（fs）；VS Code 场景经 IDE MCP 请求执行 cell | 无（或 loopback → IDE 面） | 同 IDE 行 | 无 | 并入 IDE 探针 |
| `Glob`/`Grep` | **Linux/WSL 默认缺席**（现行文档；2.1.263 无二者工具定义 [实测]）。搜索改走 Bash 的 `find`/`grep`，cc 用**内嵌 bfs/ugrep 二进制**顶替 [实测：两二进制内嵌，含 tavianator bfs 版权串]；显式启用时为 ripgrep 子进程 | 无 | bfs/ugrep/rg 在 netns 内 exec（self 下经 Bash 透传时用**宿主** find/grep——shim 不生效，行为差异小但存在） | Grep 尊重 .gitignore | L4 清单登记内嵌 bfs/ugrep；self 差异观测即可 |
| `LSP` | fork：插件提供的 language server 子进程（cgroup kind `lsp`） | 无（server 二进制用户自装） | tree：netns 内。self：留 netns（不透传） | 无 | 并入 P18 执行位置探针族 |
| `Agent`（旧 `Task`） | **in-process** subagent 循环（docs 无子进程表述；嵌套≤3 层、并发≤20；fork 模式同门） | 自身模型 API 请求 | 同 cc 本体（含其 Bash 递归走同规则） | 同 cc | 无新增 |
| Agent teams（`SendMessage`/`ListAgents` 的 team 侧、`TaskCreate` 族共享任务列表） | teammate：**in-process**（lead 进程内）或 **split-pane**（tmux/iTerm2 每 teammate 一个真实 claude 子进程）；通信=纯文件 mailbox `~/.claude/teams/<team>/inboxes/*.json`，任务列表 `~/.claude/tasks/<team>/` | 无本地网络；各实例自身 API 流量 | tree：split-pane 子进程进 netns。self：teammate 的 Bash 透传同 L1 | mailbox/任务文件落 `~/.claude`（重定向集内）→ 跨会话仅同 profile 可见 | 并入 P26（共享状态矩阵）；split-pane 需 tmux 在会话内可用（依赖面） |
| `TaskCreate`/`TaskGet`/`TaskList`/`TaskUpdate`（旧 `TodoWrite`，2.1.263 并存 [实测]） | in-process | 无 | 无 | 写 `~/.claude/todos`、任务列表文件 | 无 |
| `CronCreate`/`CronList`/`CronDelete` | in-process timer（每秒检查；任务仅在会话存活且 idle 时触发） | 无（触发的 prompt 走模型 API） | 无 | **按本地时区解释 cron**；持久化 `.claude/scheduled_tasks.json` | locale 断言已被 P6 覆盖（TZ 一致⇒调度一致） |
| `AskUserQuestion`/`EnterPlanMode`/`ExitPlanMode`/`EndConversation`/`ReportFindings`/`SubagentHandback`(≥2.1.271) | in-process UI/控制流 | 无 | 无 | 无 | 无 |
| `Skill`（旧 `SlashCommand`，2.1.263 并存 [实测]） | in-process（注入 prompt）；skill 内容里的 `` !`cmd` `` 动态注入 = **cc 执行 shell**（`disableSkillShellExecution` 可关） | 无网络本体 | tree：netns 内。self：**留 netns 不透传**（非 Bash 工具；具体 shell [未验证]） | 读 `.claude/skills/`、`~/.claude/skills/` | T5-P19（执行位置） |
| `EnterWorktree`/`ExitWorktree` | fork：git 子进程（worktree 增删） | 无（本地 git） | tree：netns。self：留 netns（git 非 Bash 工具调用） | 写 `.claude/worktrees/`、git 元数据 | 写集（git 元数据在项目内，正常） |
| `WebFetch` | **in-process** 客户端 fetch（非子进程）；HTML→Markdown；15min 缓存（`CLAUDE_CODE_WEBFETCH_CACHE_TTL_MS`）；UA `Claude-User…` | ①目标 URL（拒绝 localhost/无点域名；HTTP 强升 HTTPS）②**preflight：把 hostname 发 api.anthropic.com 做安全检查**（`skipWebFetchPreflight` 可关；不受 `NONESSENTIAL` 影响） | tree/self 同：cc 本体 → 隧道 | 无 locale 头 [实测 2c] | 出口==egress 由 P1 语义覆盖；preflight 属必达域名（doctor 集） |
| `WebSearch` | **RPC：Anthropic 服务端工具**（`web_search_20250305` [实测]；搜索在 Anthropic 后端执行，cc 只发 API 请求；≤8 次后端搜索/调用，session 上限 200） | api.anthropic.com | 同上，无自取页面 | query 语言随 prompt | 无（P13 出口归属已覆盖 API 面） |
| `Artifact` | in-process + 上传 claude.ai | claude.ai 平台 | 隧道 | credentials 在 profile | 无 |
| `RemoteTrigger`（Routines，`/schedule`） | RPC claude.ai（云端调度） | claude.ai | 隧道 | prompt 组装携带 `{userTimezone,…}` [实测 2c]——TZ 伪装错误改变调度语义 | P6 间接覆盖 |
| `PushNotification`/`SendUserFile` | RPC Anthropic infra | claude.ai | 隧道 | 无 | 无 |
| `ListMcpResourcesTool`/`ReadMcpResourceTool`/`ToolSearch`/`WaitForMcpServers` | in-process → 经各自 MCP server 传输 | 见 §2 | 随 server 传输 | 无 | 并入 MCP 子矩阵 |
| `Workflow`/advisor | in-process 编排 / API server tool | 模型 API | 同上 | 无 | 无 |
| `SendFeedback`（≥2.1.238，草稿本地 `~/.claude/feedback/drafts/`）、`/feedback`（上传 GCS + 可选 GitHub issue） | in-process 起草；上传时出网 | GCS/GitHub | 隧道 | 写 `~/.claude/feedback/` | 并入 P22 平台流量 |

---

## 2. 全量工具表 B：MCP 三传输与动态工具

| 面 | ①执行位置 | ②网络面 | ③netns 敏感性 | ④locale/config | ⑤验收缺口 |
|---|---|---|---|---|---|
| **stdio MCP** | fork：直接 `spawn(command, args)`（**不经 shell**）；`CLAUDE_PROJECT_DIR` 注入；env 默认继承（`CLAUDE_CODE_MCP_ALLOWLIST_ENV=1` 收敛为安全基线）；`StdioClientTransport` [实测] | server 进程自身出网（npx/uvx 拉包 → registry.npmjs.org；server API → 各自端点） | tree：全部在 netns → 走隧道 ✅。self：**不透传**（留 netns）→ 混合出口语义（bash=宿主网络，MCP=egress）需 doctor/verify 声明 | server 配置存 `~/.claude.json`/`.mcp.json`（重定向集内）；`${VAR}` 展开 | T5-P17：npx 拉包走 egress 断言；stdio spawn 落点 |
| **HTTP/SSE MCP**（remote） | cc 进程直连（`StreamableHTTPClientTransport`/`SSEClientTransport` [实测]；HTTP 优先、SSE 自动降级 ≥2.1.265，2.1.263 用 `--transport sse`） | 目标 https 端点 | cc 本体 → 隧道 ✅ | OAuth token 存储+刷新；`headersHelper` 命令执行面 | T5-P17 |
| **HTTP/SSE MCP**（**loopback 本地**，如 `http://127.0.0.1:PORT/mcp` [实测字符串]） | 同上，但目标是宿主 loopback | 127.0.0.1 | **缺口**：cc(netns)→宿主 loopback 不可达（cc 侧不可配置目标） | 同上 | T5-P16/P17：map-host-loopback/中继验证 |
| **ws MCP**（`type:"ws"`） | 现行文档支持 | wss:// | 隧道 | header-only 认证 | 2.1.263 无 `WebSocketClientTransport`/`type:"ws"` [实测]——版本漂移，L4 登记 |
| MCP OAuth | cc 本地 **loopback callback HTTP 监听**（自由端口；`MCP_OAUTH_CALLBACK_PORT` 可固定）+ 浏览器授权 | 宿主浏览器 → 会话内 listener | **反向缺口**：需 pasta auto 端口转发（T3 默认开） | token 落 `~/.claude` | T5-P17（OAuth 回环闭合；社区研究已列，此处补充机制锚点） |
| `mcp__*` 动态工具 | 经 server 传输 | 随 server | 同上 | `MAX_MCP_OUTPUT_TOKENS`/`MCP_TOOL_TIMEOUT` 等超时面 | 同上 |
| MCP discovery cache | in-process 缓存（`MCP_DISCOVERY_CACHE`） | 减少连接 | — | 缓存落 `~/.claude` | 无 |

---

## 3. 全量工具表 C：Hooks、扩展与集成面

| 面 | ①执行位置 | ②网络面 | ③netns 敏感性 | ④locale/config | ⑤验收缺口 |
|---|---|---|---|---|---|
| **Hooks（command 型）** | fork：**硬编码 `/bin/sh -c`**（Windows pwsh）[实测]；35+ 事件（SessionStart…SessionEnd，含 FileChanged/CwdChanged/WorktreeCreate）；`CLAUDE_PROJECT_DIR` 注入；cgroup kind `hooks`；HTTP hook=cc 出站 POST（`allowedHttpHookUrls` 门控）；MCP/prompt/agent hook 型无 shell | command：用户任意；HTTP：任意 URL | tree：netns 内直跑。self：**不透传**（`/bin/sh` 不读 `$SHELL`）→ D8 例外表应显式声明「hooks 留沙箱」；如需覆盖，`CLAUDE_CODE_SHELL_PREFIX` 官方变量作用于 hook 命令 | hook 配置来自 `~/.claude/settings.json` 等（重定向集）；`disableAllHooks` 可全关 | **T5-P18**：hook 内标记文件+出口 IP，tree/self 各自断言 |
| Skills `` !`cmd` `` 动态注入 / REPL `!` 前缀命令 | cc 执行 shell（REPL：one-shot `/bin/sh -c` 绕过模型 [实测字符串]） | 用户任意 | tree：netns。self：留 netns | 读 skills/commands 文件 | T5-P19 |
| **statusLine**/`fileSuggestion`/`subagentStatusLine` | fork：settings 命令，stdin JSON/stdout；300ms debounce；workspace trust 门控（未信任不执行）；`CLAUDE_CODE_SHELL_PREFIX` 作用域包含 statusline | 用户任意 | tree：netns。self：不透传 | settings 内；无沙箱 | T5-P19（副作用文件法） |
| **IDE 集成（VS Code/Cursor 等扩展）** | 扩展（宿主 VS Code 进程）起 **ws 服务端** `ws://127.0.0.1:<random 10000–65535>`（未加密 ws，token 认证）；token 写 `~/.claude/ide/<port>.lock`（0600/0700，`CLAUDE_CONFIG_DIR` 重定位）；**cc 为 WS 客户端**（`X-Claude-Code-Ide-Authorization` header [实测]），经扩展提供的 **IDE MCP server**（`mcp__ide__*`：getDiagnostics/openDiff/执行 notebook cell [实测]） | **本地 loopback**（127.0.0.1） | **结构性缺口**：cc(netns)→宿主 loopback 断裂；cc 侧不可配置目标 IP。候选适配：会话内 `127.0.0.1:<port> → 网关地址:<port>` 中继（socat/内置；port 不变故 lockfile 无需改写）→ 实现期裁决；不可用则 doctor 报告降级 | lockfile/IDE MCP 配置在 `~/.claude`（重定向集）；**`autoInstallIdeExtension`：会话内跑 `claude` 会自动装扩展 → 写 `~/.vscode/extensions`（重定向集之外！）**；`CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL=1` 可关 | **T5-P20**：lockfile 存在性 + ws 经中继可达性；**P27**：写集例外登记回归 |
| IDE 集成（JetBrains 扩展） | 同族机制（lockfile+本地连接）[未验证细节——jetbrains 页未取证] | loopback | 同上 | 同上 | 同上（先取证再列） |
| **Chrome 集成**（Claude in Chrome，`/chrome`） | 浏览器扩展 + **native messaging**（host manifest 写 `~/.config/google-chrome/NativeMessagingHosts/com.anthropic.claude_code_browser_extension.json` 等，Windows 走注册表）；工具以 `claude-in-chrome` MCP server 暴露；云端 `bridge.claudeusercontent.com` | loopback/native messaging + 云端 | **WSL 不支持**（官方）；Linux 桌面场景：浏览器在宿主、cc 在 netns 的跨界 [未验证可达性] | manifest 写浏览器配置目录（**重定向集之外**） | P27 登记 + doctor 检测 |
| **Plugins** | 安装：marketplace **git clone**（GitHub shorthand 先 SSH 后 HTTPS；用宿主 git credentials）/ 本地目录 / https marketplace.json；源类型 `github`/`git-subdir`/`npm`（**不跑 install scripts**）/`archive`（https zip，禁 loopback 目标）/`command`（**跑 shell 命令**，需确认）；落 `~/.claude/plugins/`；插件组件=commands/skills/agents/**hooks/MCP/LSP/monitors**（hooks/MCP/LSP 即 §1–3 各执行面）；2.1.263 二进制含 `downloads.claude.ai/claude-code-releases/plugins/…` [实测，官方托管分发，文档未提] | git/npm/https 下载 | tree：安装流量在 netns → 隧道 ✅。self：安装时 cc 本体出站 → 隧道 | `enabledPlugins`/marketplace 状态在 settings；官方 marketplace `claude-plugins-official` 首启自动添加 | T5-P22（安装流量走 egress） |
| **跨会话消息**（`SendMessage`/`ListAgents` 的 session 侧） | cc **bind 本地监听**：per-session **Unix domain socket** `/tmp/cc-socks-<uid>/`（Windows named pipe）；`CLAUDE_CODE_MESSAGING_SOCKET/_TOKEN`；同机投递**不过 Anthropic** | 本地 socket；跨机/云端经 Remote Control（Anthropic 中继） | R3 放行 `/tmp` → 同宿主跨会话可达（同 uid）；fs 不同的容器/会话之间天然不可达（官方明示） | `crossSessionInbound` accept/hold/refuse；注册表在磁盘文件 | **T5-P26**：同宿主跨会话 × 同/异 profile 矩阵 |
| **Remote Control**（`claude --remote-control`/`/rc`） | cc 出站轮询+流式连接（**仅出站 HTTPS api.anthropic.com:443，无入站端口**[官方明示]）；transcript 连接期**存储于 Anthropic 服务器**；同机执行不变 | api.anthropic.com | 隧道 ✅（出口归属风控面→P13） | `ANTHROPIC_BASE_URL` 非官方则禁用；`NONESSENTIAL`/`DISABLE_GROWTHBOOK` 下禁用 | P13 语义已覆盖；并入 P23 |
| Worktrees/后台任务/`/tasks` | fork git / 进程组管理 | 本地 | 同 Bash | 状态在 `~/.claude` | D8 已列后台任务保真度 |
| Channels（MCP server 反向推送进会话） | 经 MCP 传输 | 随 server | 随传输 | — | 并入 P17 |
| computer-use | cc 驱动本机输入/截图 | 本地 | netns 内（ Wayland/X11 依赖宿主 display——R12 域） | — | P3/R12 域，不单列 |
| `claude-cli://` deep links | 桌面 URL scheme | 本地 | — | — | 不适用（无头宿主） |

---

## 4. 全量工具表 D：平台面（遥测/更新/诊断/登录）

| 面 | ①执行位置 | ②网络面（目的地） | ③netns | ④locale/config | ⑤验收 |
|---|---|---|---|---|---|
| 模型 API + feature flags + WebFetch preflight + 遥测事件 | in-process | `api.anthropic.com`（第三方 provider 时换端点，**preflight 仍打它**[2c]） | 隧道 | flags 缓存 `~/.claude`；`DISABLE_TELEMETRY` 连带关 flag 评估 [2c] | doctor 必达域名集（已有） |
| 运营遥测/错误上报 | in-process | Datadog logs intake `http-intake.logs.us5.datadoghq.com/api/v2/logs`、`browser-intake-us5.datadoghq.com` [实测]；Sentry [实测]；Statsig/GrowthBook flags；文档只称 "third-party logging/error tracking"（厂商名仅二进制可见） | 隧道 | `DISABLE_TELEMETRY`/`DISABLE_ERROR_REPORTING`/`DO_NOT_TRACK`/`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`（注意 `0`/`false` 也是开[官方]）；Bedrock/Vertex 默认全关 | T5-P22（全开关时零非必要流量断言） |
| **OTel**（`monitoring-usage`） | in-process exporter 主动推送 | `OTEL_EXPORTER_OTLP_ENDPOINT`（grpc 4317 / http 4318；**示例即 `localhost:4317`**） | **loopback 缺口**（自建 collector 常在宿主 localhost）＋ remote collector 走隧道 | `CLAUDE_CODE_ENABLE_TELEMETRY=1` 开启；`OTEL_*` 不传给任何子进程 [官方]；trace 时 bash 子进程注入 `TRACEPARENT` | **T5-P24**：collector 可达性（loopback 中继 vs remote egress） |
| 自更新（native 安装形态） | in-process 下载 + fork installer | `downloads.claude.ai/claude-code-releases/<channel>` [实测]；npm 形态 → registry.npmjs.org | 隧道 | `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` 关闭 | T5-P22 |
| **OAuth 登录**（claude.ai/Console） | in-process + 本地 loopback callback 监听（自由端口）+ 宿主浏览器 | claude.ai/platform.claude.com + 127.0.0.1 | **反向缺口**：宿主浏览器→会话 listener 需 auto 端口转发 | credentials 落 `~/.claude/.credentials.json`（重定向集） | T5-P17（OAuth 回环闭合，含 `claude mcp login`） |
| `claude doctor` | in-process 只读诊断：settings 校验、安装健康度、**扫描 `~/.zshrc ~/.bashrc ~/.config/fish/config.fish`** [2c]；`/status` 显示必达域名连通性 | `/status` 有 API 探测 | netns 内读到的是沙箱视图（预期） | **读宿主 HOME 的 rc 文件**（R3 HOME 共享的佐证）；managed settings `/etc/claude-code/managed-settings.json` 为宿主路径，mountns 未隔离 → 会话内读到宿主 managed 配置（读方向，doctor 应报告） | doctor 增检 managed-settings 来源 + IDE lockfile 存在性 |
| Checkpointing/rewind | in-process（`~/.claude` 下状态 [实测 checkpoint×67]） | 无 | 无 | 重定向集内 | 无 |
| 文件监视（agents/skills/settings 热加载、`FileChanged` hook） | in-process watcher | 无 | 无 | 监视 `~/.claude/agents` 等 | 无 |
| Shell snapshots | 会话启动 fork `$SHELL` 捕获 rc → 写 `~/.claude/shell-snapshots/` | 无 | self：捕获经 wrapper 落宿主 shell env（与 D8 env 语义一致） | 重定向集内 | D8 已列 |
| cgroup 内存上限（`CLAUDE_CODE_TOOL_MEMORY_LIMIT`） | fork `sh -c 'echo $PPID > "$1"/cgroup.procs'` [实测] | 无 | iso-cc userns 下 cgroup 委派通常不可用 → **官方语义：失败则无 cap 继续跑** | env 开关 | **T5-P26 内**：降级不断言失败（命令仍执行） |
| `CLAUDE_CODE_PROCESS_WRAPPER`/settings `processWrapper` | 官方进程包装钩子（corporate-launcher 场景） | — | **适配候选**：比 `SHELL_PREFIX` 更底层的统一包装点（评估成本后裁决） | settings | 评估项，无探针 |
| Bash **sandbox 模式**（`/sandbox`） | fork：Linux = **bubblewrap**（userns）+ **socat relay → sandbox proxy**（proxy 在沙箱外，默认无预放行域名，逐域名批准；`sandbox.bwrapPath/socatPath/ripgrep` settings；seccomp 选配用于 UDS 屏蔽；`sandbox-exec` 串仅 macOS [实测]） | proxy 强制（HTTP/SOCKS 端口型；实验性 `tlsTerminate`） | **嵌套**：bwrap userns-in-userns（Linux 支持；Ubuntu 24.04 AppArmor 限制→P3 已列）；proxy 链：命令→sandbox proxy(会话 netns 内)→pasta→egress | `sandbox.excludedCommands`、`allowUnsandboxedCommands`、`autoAllowBashIfSandboxed` | **T5-P25**：双层出口一致断言 |

---

## 5. 网络位置矩阵（面 × 位置）

图例：●=有 ○=无。行按矩阵主序排列。

| 工具面 | 进程内直做 | fork 子进程 | 出站 api.anthropic.com | 出站第三方/任意公网 | loopback 出站 (cc→127.0.0.1) | 本地监听 (cc bind) | 宿主 RPC (self 透传) |
|---|---|---|---|---|---|---|---|
| Read/Edit/Write/MultiEdit/NotebookEdit | ● | ○ | ○ | ○ | ○ | ○ | ○ |
| Task*/TodoWrite/Cron*/plan 类/Skill 注入本体 | ● | ○ | ○ | ○ | ○ | ○ | ○ |
| Glob/Grep/rg/bfs/ugrep/LSP/git(worktree) | ○ | ● | ○ | ○ | ○ | ○ | ○ |
| Bash（含后台任务） | ○ | ● | ○ | ●(用户命令) | ○ | ○ | **●** |
| Monitor（命令） | ○ | ● | ○ | ●(用户命令) | ○ | ○ | [未验证] |
| Monitor（ws 源） | ● | ○ | ○ | ●(任意 ws) | ○ | ○ | ○ |
| WebFetch | ● | ○ | ●(preflight) | ●(目标站) | ○(拒绝 localhost) | ○ | ○ |
| WebSearch | ● | ○ | ● | ○(服务端执行) | ○ | ○ | ○ |
| Agent/subagents(in-process) | ● | ○ | ● | ○ | ○ | ○ | ○ |
| Teams split-pane | ○ | ●(子 claude) | ● | ○ | ○ | ○ | 经其 Bash● |
| MCP stdio | ○ | ● | ○ | ●(server 出网) | ○ | ○ | ○ |
| MCP HTTP/SSE remote | ● | ○ | ○ | ● | ○ | ○ | ○ |
| MCP HTTP/SSE **loopback** | ● | ○ | ○ | ○ | **●** | ○ | ○ |
| MCP OAuth / claude.ai OAuth | ● | ○ | ● | ○ | ○ | **●**(callback) | ○ |
| Hooks(command) / REPL `!` / skill `!cmd` / statusline | ○ | ● | ○ | ●(用户命令) | ○ | ○ | ○(不透传) |
| Hooks(HTTP 型) | ● | ○ | ○ | ● | ○ | ○ | ○ |
| IDE 扩展 | ● | ○ | ○ | ○ | **●** | ○(cc 侧) | ○ |
| Chrome native messaging | ●(manifest) | ● | ●(bridge) | — | [未验证] | ○ | ○ |
| 跨会话消息 | ● | ○ | ○(同机) | ●(跨机经 RC) | ○ | **●**(UDS) | ○ |
| Remote Control / Artifact / Routines / Push / SendUserFile | ● | ○ | ● | ○ | ○ | ○ | ○ |
| 遥测(Datadog/Sentry/Statsig) | ● | ○ | ● | ● | ○ | ○ | ○ |
| OTel | ● | ○ | ○ | ●(collector) | ●(localhost 常见) | ○ | ○ |
| 自更新/插件安装 | ●/○ | ●(git/npm/installer) | ○ | ● | ○ | ○ | ○ |
| Bash sandbox(bwrap+proxy) | ○ | ● | ○ | ●(经 proxy) | ○ | ○ | 经 Bash● |

---

## 6. netns 侧适配清单（按优先级）

1. **A1 loopback 中继原语**（P0，服务四个面）：cc→宿主 loopback 的目标硬编码 127.0.0.1，pasta `--map-host-loopback` 只把**网关地址**映射到宿主 loopback，cc 不会用网关地址。候选方案：会话内起 `127.0.0.1:<port> → <网关地址>:<port>` 的中继（socat 已是 cc 自己的依赖，形态同构；或 iso-cc 内置 multi-call relay）。受益面：IDE 扩展 WS、loopback MCP server、OTel localhost collector、其他宿主 loopback-only 服务。**实现期裁决项**；不可用则 doctor 显式报告「IDE/loopback MCP 降级」。
2. **A2 反向发布（已有 T3 auto 端口转发）**：OAuth 登录/MCP OAuth callback 是宿主浏览器→会话 listener，依赖 pasta 入向端口转发默认开 + doctor 端口冲突检测（T3 已覆盖，此处补用途登记）。
3. **A3 self 拦截完备性声明**（D8 例外表增补）：不透传面 = hooks（`/bin/sh -c`）、statusline/fileSuggestion、REPL `!`、skill `` !`cmd` ``、stdio MCP server、LSP server、rg/bfs/ugrep、git helper、sandbox bwrap。语义：这些面的出网走 egress A，与透传的 Bash（宿主网络）**混合出口**——verify「不覆盖面」声明 + doctor 回显。可选统一层：`CLAUDE_CODE_SHELL_PREFIX`（官方变量，覆盖 Bash 调用、hook 命令、statusline、stdio MCP 启动）或 `CLAUDE_CODE_PROCESS_WRAPPER`，进 P3 评估。
4. **A4 写集例外登记**（P8 白名单扩列）：① `autoInstallIdeExtension` → `~/.vscode/extensions/`（或 profile 预置 `CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL=1`）；② Chrome NM manifest → `~/.config/{google-chrome,BraveSoftware,...}/NativeMessagingHosts/`（仅启用 chrome 集成时）；③ 现有有界例外（挂载点/orca）沿用登记格式。
5. **A5 doctor 增检**：`/etc/claude-code/managed-settings.json` 存在性（宿主配置会渗入会话，读方向）；`~/.claude/ide/*.lock` 存在性（IDE 集成意图）；`/tmp/cc-socks-<uid>`（跨会话消息面）；tmux 缺失（split-pane teams 不可用）。
6. **A6 共享状态语义表**：`/tmp`（UDS 消息）× `~/.claude`（profile 重定向）两轴正交——同宿主跨会话消息可达（R3 放行 /tmp），但 teams/tasks/mailbox 仅同 profile 互见。写入 R3/R10 验收措辞。
7. **A7 嵌套沙箱兼容**：cc sandbox 模式的 bwrap 在 iso-cc userns 内嵌套创建 userns（内核支持即过；24.04 AppArmor → P3 profile 覆盖 bwrap）；sandbox proxy 链路与 egress 叠加断言（P25）。
8. **A8 可选 env 预置**（profile 可选 env，声明式）：`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`、`DO_NOT_TRACK`、`CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL`、`disableSkillShellExecution`（settings 键）。
9. **A9 必达域名集增补**（doctor egress 校验）：在 `api.anthropic.com`+OAuth 三件套+registry.npmjs.org 基础上，按启用的面追加：`downloads.claude.ai`（自更新/官方插件分发）、`bridge.claudeusercontent.com`（chrome 集成）、用户配置的 MCP/OTel 端点（ doctor 从 config 读取而非硬编码）。

---

## 7. T5 增补建议（编号续接 P1–P15）

| # | 探针（会话内/宿主侧） | 断言 | 覆盖面 |
|---|---|---|---|
| P16 | 正向：会话内起 loopback HTTP server → 宿主 `curl 127.0.0.1:<port>`（经 auto 转发）。反向：宿主起 loopback server → 会话内 `curl 127.0.0.1:<port>`（经 A1 中继/map-host-loopback） | 双向按声明可达；不可达时 doctor 降级声明一致 | map-host-loopback、IDE/loopback MCP 前提 |
| P17 | MCP 三传输子矩阵：①stdio：`claude mcp add` 一个 `npx -y` server → 首连时宿主 egress 侧观测 registry.npmjs.org 请求；②remote HTTP：指向 echo server → 出口==egress；③OAuth：`MCP_OAUTH_CALLBACK_PORT` 固定端口 → 宿主浏览器完成授权 | ①拉包走隧道 ②出口==egress ③回环闭合 | stdio/HTTP/OAuth callback |
| P18 | hooks 执行位置：SessionStart hook 写标记文件 + `curl ifconfig.co`；对照 Bash 工具内同命令 | tree：标记在 netns 视角、出口==egress。self：标记文件共享 fs 同路径；**hook 出口==egress（不透传），Bash 出口==宿主**——与 D8 例外表逐字一致 | hooks、self 混合出口语义 |
| P19 | statusline 副作用文件 + REPL `!` 与 skill `` !`cmd` `` 内 `curl ifconfig.co` | self 下三者出口==egress（留沙箱）；tree 同 P18 | statusline/`!`/skill 注入 |
| P20 | IDE lockfile：宿主 VS Code 装扩展后，会话内读 `~/.claude/ide/*.lock`（端口/token）→ 经 A1 中继连 `ws://127.0.0.1:<port>` | token 握手成功（`/ide` 可用）或 doctor 降级声明 | IDE 集成、A1 |
| P21 | Monitor ws 源连 `<egress echo ws>` | 连接自会话出站且出口==egress | Monitor WebSocket |
| P22 | 平台流量：`DISABLE_TELEMETRY`+`DO_NOT_TRACK`+`NONESSENTIAL` 全设 → 会话期宿主 egress 上无非必要域名；未设时观测 datadoghq/downloads.claude.ai 走隧道 | 出口==egress；全关时零非必要流量 | 遥测/自更新/feedback |
| P23 | Remote Control / Artifact 可用性冒烟（有 claude.ai 账号时） | 出口==egress 且出口区域==声明区（P13 语义延伸到平台面） | 平台面出口恒定 |
| P24 | OTel：`OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`（宿主起 collector）→ 会话内收到导出（经 A1）；remote endpoint 版本走 egress | 导出到达且出口符合声明 | OTel collector |
| P25 | bash sandbox 冒烟：会话内 sandbox bash `curl ifconfig.co` → 出口==egress（proxy→pasta 双层）；bwrap userns 嵌套成功 | 双层出口一致；无 AppArmor 拒绝 | sandbox 嵌套 |
| P26 | 跨会话矩阵：同宿主两会话（同/异 profile）× `SendMessage` + cgroup cap 降级观测（userns 下命令仍执行） | 同 profile 可达/异 profile 按声明不可达；cap 失败不致命 | UDS/teams/cgroup |
| P27 | 写集例外回归：启用 `autoInstallIdeExtension`/chrome 集成后重复 P8 | diff ⊆ 声明集 ∪ 登记例外 | P8 扩展 |

---

## 8. 版本漂移与 L4 清单增补（按 version pin 维护）

- **2.1.263 实测≠现行文档**：MCP `ws` 传输无 [实测]；skills claude.ai 同步 ≥2.1.273 无；WebFetch/SSE auto-fallback ≥2.1.265 无；Task tools 默认集、Glob/Grep 默认缺席语义以现行文档描述为准（2.1.263 无 Glob/Grep 工具定义 [实测]，bfs/ugrep 内嵌 [实测]）；`SubagentHandback`/per-command sandbox domains 等 ≥2.1.271 无。
- **L4（per-agent 重定向清单）按版本登记**：内嵌 bfs/ugrep（二进制内嵌，无路径）；PATH 依赖 bwrap/socat/ripgrep（nix 包装已注入；`sandbox.bwrapPath/socatPath/ripgrep` settings 可显式钉死——iso-cc doctor 可建议钉死以稳定 L4）；`/bin/sh`（hooks/REPL，来自宿主 glibc 路径，稳定）。
- 工具名演化：`Task`→`Agent`、`BashOutput/KillShell`→`TaskOutput/TaskStop`、`SlashCommand`→`Skill`、`TodoWrite`→`Task*`（2.1.263 新旧并存 [实测]）——permission/hook matcher 与探针按 pin 版本使用对应名。

---

## 9. 来源

**官方文档（code.claude.com/docs/en/，2026-09-26 取证）**
- 工具面：`tools`（tools-reference）、`mcp`、`hooks`、`sub-agents`、`agent-teams`、`cross-session-messaging`、`remote-control`、`scheduled-tasks`、`worktrees`（索引级）
- 扩展/集成：`vs-code`（ide-integrations 重定向目标；§IDE MCP server/lockfile 一节为本缺口核心依据）、`chrome`、`plugins/overview`、`plugins/install`、`plugins/marketplace-reference`、`skills`
- 平台面：`monitoring-usage`（OTel）、`data-usage`（遥测/preflight/本地留存）、`statusline`、`env-vars`（`CLAUDE_CODE_SHELL`/`_SHELL_PREFIX`/`_SUBPROCESS_ENV_SCRUB`/`MCP_OAUTH_CALLBACK_PORT` 等）、`settings`、`managed-settings`（Linux `/etc/claude-code/managed-settings.json`）、`network-config`、`debug-your-config`、`troubleshoot-install`（承 2c 引用）、`sandboxing`（bwrap+socat+proxy）、`corporate-launcher`（processWrapper，索引级）、`llms.txt`（面清单交叉核对）

**二进制取证 [实测 2026-09-26]**
- `/nix/store/nh4j5xkxxhl74bj487w34lyj98g50c66-claude-code-2.1.263/bin/.claude-wrapped`，`grep -a` 字符串级：`web_search_20250305`、`StdioClientTransport/SSEClientTransport/StreamableHTTPClientTransport`（无 WS 传输）、`X-Claude-Code-Ide-Authorization`、`ws://127.0.0.1:${f.port}`、`mcp__ide__`/`getDiagnostics`/`openDiff`、`/bin/sh -c`（hooks/REPL）、`process.env.SHELL||process.env.COMSPEC`、bfs/ugrep 内嵌（Tavian Barnes 版权串）、`sandbox-exec`/`bwrap`/`socat - PROXY:`/`socat - UNIX-CONNECT:`、`sh -c 'echo $PPID > cgroup.procs'`、`http-intake.logs.us5.datadoghq.com/api/v2/logs`、`downloads.claude.ai/claude-code-releases/<channel>/plugins/…`、工具名新旧并存（`BashOutput`/`KillShell`/`TaskOutput`/`MultiEdit`/`SlashCommand`/`TodoWrite`）、CLAUDE_CODE_* env 597 个。

**本仓库前置**
- `docs/research/2026-09-26-related-projects-and-purity-probes.md` §2b/2c/2d（locale 双视线、必达域名表、P1–P12）
- `docs/research/2026-09-26-community-risk-signals.md`（P13–P15、OAuth 回环闭合）
- `docs/adr/0008-net-scope-declaration.md`（self 转发机制、L1–L4 拦截分层、例外表）
- `docs/REQUIREMENTS.md` T3/T5（auto 端口转发、loopback-only 可达性探针既有条目）
