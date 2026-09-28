# 24 — grounding：L1.5 × stdio MCP 全链实测（@playwright/mcp 为样例负载）

**What to build:** 票 23/15 的验证缺口：host 模式下"cc（netns）→ L1.5 prefix → shim → exec.sock RPC → 宿主侧 spawn stdio MCP server → cc 持续调用其工具"全链**从未用真 MCP server 实测过**。本票 = 该链的逐跳 grounding + 缺口修复。

**Blocked by:** 无　**Owner:** lane-mcp-grounding（完成）
**Status:** done（2026-09-27，报告 /tmp/iso-cc-exp24/REPORT.md；E1 真负载全绿 parent 抽验）

## Answer（PM 落）

- **机制证实（源码级，字节偏移）**：stdio MCP spawn 走 L1.5「直 exec」形态——cc 以 CLAUDE_CODE_SHELL_PREFIX 本身为可执行文件、原命令+args 经 POSIX 单引号拼成单载荷（argv=[prefix, 单载荷]，shell:false）；与 execstub L1.5 契约**精确吻合**（E1 live argv 逐字验证）。附3 声明升级为机制证实
- **E1 全绿**：真 claude -p 在 netns 会话 + exec.bash=host，@playwright/mcp 常驻 stdio 四次工具调用（navigate/snapshot/screenshot/close）全成功；SCM_RIGHTS 跨 netns 直通 fd 级实证；E2 worker kill → 3s 透明 respawn；E3/E4 teardown/突死 ≤0.5s 零残留
- **G1（阻断级新 bug，转票 25）**：exec.bash=host 下 Bash 工具面 126 不通——Bash 工具的 prefix 机制与 MCP spawn 不同（prefix 拼进 -c 载荷 `'<prefix>' '<script>'`），execstub 对 `-l` 形态拒收 + 二跳缺 ISO_CC_EXEC_SOCK。附3『L1.5 覆盖 Bash 工具』在当前 shim 契约下不成立
- G2（pidns 行为定性，无需修）/ G3（reap 误记，并入 25 顺手修）/ G4（chromium WSL2 flags 脚注）

## 背景（诚实状态）

- 附3 取证声称 L1.5 "实测覆盖 stdio MCP"——行为观测，机制未知（spawn 是否经 prefix shell / 还是 spawn 命令串拼 prefix，两种机制拦截点不同）
- 票 15 的实现验证用的是合成 marker 探针（one-shot 命令形态）；**常驻 stdio 进程形态从未实测**
- SCM_RIGHTS fd 直通设计上支持长生命周期 stdio，但 pipe 半途死亡、cc 重连语义、host 侧收割时机均为空白

## 实验矩阵（/tmp 脚本，@playwright/mcp 真负载）

1. **spawn 机制取证**：CLAUDE_CODE_SHELL_PREFIX 注入下，cc（替身驱动等价链）spawn MCP server 时的 argv 形态与落点（netns 侧 or 宿主侧）——strace/进程树留证
2. **常驻生命周期**：MCP server 宿主侧 spawn 后：stdio 双向持续可用时长 = 会话期？cc 调工具（initialize / tools/list / tools/call 开关网页）多轮往返
3. **真实负载**：@playwright/mcp 真实起浏览器（headless 或 --cdp-endpoint 连宿主 Chrome，后者联动票 23）→ 会话内工具调用打开网页截图 → 成功即全链通
4. **失败语义**：MCP server 中途死亡（kill）→ cc 侧 pipe EOF 行为取证；会话结束时 worker 收割（13 双键）取证
5. **netns 对照组**：不注入 L1.5（engine=netns 默认 sandbox）→ MCP server 落 netns 内 → playwright 驱动浏览器的浏览器流量走会话出口（这是身份一致形态，两模式语义差异写清）

## Acceptance

- [ ] spawn 机制 argv/落点留证；机制结论（经 prefix shell / 拼接 / 不走）
- [ ] 真实 @playwright/mcp 常驻链全绿：多轮工具调用往返、会话期存活、teardown 收割
- [ ] 失败语义两例（worker 死 / 会话 teardown）行为留档
- [ ] 若链路有缺口（one-shot 假设不成立等）→ 缺口清单 + 修复票草；不阻塞报告
- [ ] clippy/nextest 不回归（如有代码修复）

## 边界

- /tmp 脚本 + 报告；真 MCP server 经 npx 拉 @playwright/mcp（公开源，零隐私）；不 git；不改 20/23 在途面


---

# 附：REPORT.md 全文（2026-09-28 自 /tmp/iso-cc-exp24/ 迁入防丢；重实验产物仍在 /tmp，可复现）
# 工单 24 — L1.5 × stdio MCP 全链 grounding（@playwright/mcp 真负载）

状态：完成（阶段 1 + 阶段 2 全部实验落地；票面 Acceptance 前四条满足）
日期：2026-09-27
环境：NixOS 26.11 (WSL2, kernel 6.18.33.2-microsoft-standard-WSL2)；iso-cc = target/debug（票 24 时点源码）；claude-code 2.1.263（npm 官方 tarball + 本机 Nix 打装二进制）；@playwright/mcp@latest（npx 拉取）；宿主 chromium 152.0.7977.82 headless（`--remote-debugging-port=9222`）。

产物目录：`/tmp/iso-cc-exp24/`
- `cc-pkg/package/claude` —— @anthropic-ai/claude-code-linux-x64 2.1.263 官方二进制（215,662,064 B，sha256 26d02035…d5ba）。本机 Nix 装 `.claude-wrapped`（sha256 12f01c6d…4128）同版本、仅 ELF interpreter 被 Nix patch，字节不一致属预期。
- `cc-strings.txt` —— 二进制 strings 提取（44,276,467 B）。
- `ctx.py` —— 取证窗口提取器（regex → ±N 字节上下文）。
- `lab/` —— 阶段 2 实验场（config.toml 含密钥已 0600；e1/ 证据；observer*.sh）。
- 本报告。

---

## 阶段 1 —— cc 源码级机制取证（claude-code 2.1.263 打包产物）

方法：官方 npm `@anthropic-ai/claude-code-linux-x64@2.1.263` tarball（bun 编译 ELF，内嵌 minified JS bundle），对二进制做字节级 regex 取证；全部结论附字节偏移，可复现（`python3 ctx.py '<pattern>'`）。

### 1.1 结论总表

| 面 | 机制 | argv 契约（iso-cc shim 收到的形态） | 证据偏移 |
|---|---|---|---|
| **stdio MCP server spawn** | **L1.5 prefix 直接作为可执行文件 exec**（非拼串进 shell） | `[<prefix-bin>, "<shell-quoted 完整调用串>"]`（**恰一个载荷参数**） | 204764811；177442162；204734345/204988258 |
| （可选 cgroup 分支） | `/bin/sh -c '{ echo 0 > "$0"/cgroup.procs; } 2>/dev/null; exec "$@"' <cgdir> <prefix> <载荷串>` —— `exec "$@"` 保形 | 同上（prefix+单载荷串经 exec "$@" 透传） | 177445585（nRt） |
| Bash 工具 | prefix **字符串拼接**进 `-c` 载荷（Yhe），shell = `$SHELL` 解析路径 | `[<shell>, "-c", ["-l",] "'<prefix>' '<script>'"]` | 183248000–183252000；Yhe@182117300 |
| hooks / statusline | 同 Bash 工具：Yhe 拼串进 hook 的 shell 载荷 | 同上 | 186541046；statusline 经 wge→同一 exec 管线（bWt 两处） |
| REPL `!cmd` | 硬编码 `/bin/sh -c`（或 pwsh），**不走 prefix** | 与 L1.5 无关（留 netns 直跑，ADR 附3 一致） | 文案串@96817952 区段 |
| SHELL env 在 MCP spawn 中 | **不参与 spawn 机制**；仅作为继承 env 变量（SDK 基底白名单含 SHELL；CLI 模式全量继承 process.env） | — | 204734345（lo()）；179563138（h7e）；179561034（Ii） |

### 1.2 stdio MCP spawn 主路径（本票核心问题）

连接构建点（偏移 204764811 起，`t` = server 配置 `{command,args,...}`，`a` = env 访问器）：

```js
else if(t.type==="stdio"||!t.type){
  let D = a.CLAUDE_CODE_SHELL_PREFIX || t.command,
      y = a.CLAUDE_CODE_SHELL_PREFIX ? [jo([t.command,...t.args])] : t.args,
      {command:q,args:G,pending:pe,capped:ke} =
          due(e) ? {command:D,args:y,pending:!1,capped:!1}     // 被 hook 引用的 server：跳过 cgroup 包裹
                 : nRt("mcp",D,y);                             // 默认：cgroup 包裹决策
  ...
  ne={command:q,args:G,env:{...st,CLAUDE_PROJECT_DIR:sn(),CLAUDE_CODE_SESSION_ID:X(),CLAUDECODE:"1",...t.env,...},stderr:"pipe"},
  x=new Tt(ne)                                                 // BoundedStdioClientTransport
```

- `jo`（偏移 177442162）= POSIX 单引号 shell-quote：

```js
function jo(e){return e.map((t)=>{let r=String(t);
  if(r==="")return"''";
  if(/^[A-Za-z0-9_./:=@+,-]+$/.test(r))return r;
  return"'"+r.replaceAll("'",`'"'"'`)+"'"}).join(" ")}
```

- 即 **prefix 被设置为 spawn 的 command 本身**，原始 `command+args` 被 quote 拼成**一个字符串**作为唯一参数。例：prefix=`/home/nixos/.local/state/iso-cc/sessions/<id>/bin/bash`、server=`npx -y @playwright/mcp@latest --cdp-endpoint http://127.0.0.1:9222`（全部 token 落在安全字符类内，quote 后原样）→

```
execve("<sess>/bin/bash", ["<sess>/bin/bash", "npx -y @playwright/mcp@latest --cdp-endpoint http://127.0.0.1:9222"], env)
```

- 传输层 `Tt extends uo`（SDK StdioClientTransport）最终 `spawn(this._serverParams.command, this._serverParams.args ?? [], {env:{...}, stdio:["pipe","pipe",stderr??"inherit"], shell:!1, windowsHide, cwd})`（偏移 204734345 / 204988258 两份拷贝）——**shell:false、argv 数组透传、无二次 shell 解析**。

**机制定性：MCP 面 = 「prefix 可执行 + 单载荷串」直接 exec，不是拼接进 shell 命令串。** iso-cc execstub 的 L1.5 形态判定（`args.len()==1` → 整串转宿主 `/bin/bash -c`）与该契约**精确吻合**；宿主侧 `/bin/bash -c <串>` 承担 cc 省掉的 shell 解析。

可选分支（`nRt`，偏移 177445585）：tool-memory cgroup 特性激活时（`U("mcp")` 有 cgroup 目录且 `BT(t)` 可映射），spawn 变形为

```
["/bin/sh","-c","{ echo 0 > \"$0\"/cgroup.procs; } 2>/dev/null; exec \"$@\"", <cgdir>, <prefix>, <载荷串>]
```

`exec "$@"` 丢弃 sh 自身、以 prefix 为 argv0 透传其余参数 → **契约保持不变**（仍为 prefix+单载荷串）。`due(e)`（偏移 182125178）= server 被 `mcp_tool` hook 引用时跳过 cgroup 包裹直连。默认部署（未开 cgroup）`U("mcp")==void 0` → 纯透传 `{command:D,args:[...y]}`。

### 1.3 Bash 工具 / hooks / statusline（对照面）

Bash 工具（偏移 183248000 区段，class `lin(e,t)`）：

- `buildExecCommand`：脚本串组装（snapshot source + `eval <user script>` + `pwd -P >| <cwdfile>` 以 `&&` 连接）后 `let Oe=a.CLAUDE_CODE_SHELL_PREFIX;if(Oe)Ie=Yhe(Oe,Ie)`。
- `Yhe`（偏移 182117300，真身定义；同 bundle 另有三处同名 minified 碰撞为 parentSessionId getter，已排除）：

```js
function Yhe(e,t){let r=e.lastIndexOf(" -");
  if(r>0){let o=e.substring(0,r),d=e.substring(r+1);
    return`${jo([o])} ${d} ${jo([t])}`}
  else return`${jo([e])} ${jo([t])}`}
```

- spawn：`getSpawnArgs` → `["-c", ...有snapshot时无"-l", commandString]`，shellPath = `$SHELL` 解析（ADR 附3 的 L1 主路径确认）。
- 前导守卫 `eso(e)`：prefix 存在时注入 shell 无关的 `{ shopt -u extglob || setopt …; } >/dev/null 2>&1 || true`。
- **注意**：prefix 值本身是 iso-cc 的 shim 路径（不含 " -"）时，`Yhe` 产出 `'<shim路径>' '<整段脚本>'` ——即 Bash 工具的 `-c` 载荷里**再次嵌套一次 shim 调用**（L1 与 L1.5 叠加）。这对 iso-cc 的含义见 §3 缺口 G1。
- hooks（偏移 186541046）：`let Gt=re!==void 0?re.shellPrefix:a.CLAUDE_CODE_SHELL_PREFIX,en=!Oe&&!Me&&Gt?Yhe(Gt,Ke):Ke` —— 同 Yhe 拼串。statusline（`bWt`→`wge(f,"StatusLine","statusLine",…)`）走同一 hook exec 管线 → 同为 Yhe 拼串形态。
- REPL `!cmd`：产品文案明示 "one-shot `/bin/sh -c` (or `pwsh`) subprocess"（偏移 96817952 区段文案串）——硬编码 /bin/sh，不经 prefix，也不经 `$SHELL`。

### 1.4 MCP 子进程 env（SHELL/prefix 的下游可见性）

`ne.env` 基底 = `Ve = h7e() ? {...lo(),...cme()} : Ii()`：

- `h7e()`（179563138）：`CLAUDE_CODE_MCP_ALLOWLIST_ENV` 已设（含空串）→ true；否则 `CLAUDE_CODE_ENTRYPOINT==="local-agent"`（SDK 场景）→ true。
- 交互 CLI（ENTRYPOINT="cli"）→ `Ii()`（179561034）= **process.env 全量**（外加 agent-proxy 项）。此时 MCP server 子进程继承 `CLAUDE_CODE_SHELL_PREFIX`、`SHELL=<shim>`、`ISO_CC_EXEC_SOCK`、L2 前置的 PATH。
- allowlist 模式 → 仅 `{HOME,LOGNAME,PATH,SHELL,TERM,USER}`（`lo()`，204734345 区段）+ agent-proxy 项——`SHELL` 仍在白名单内，但 `ISO_CC_EXEC_SOCK`/prefix 不在。
- 显式排除：`{CLAUDE_CODE_CHILD_SESSION, CLAUDE_CODE_CHROME_MCP_ORG_DENIED}` 从基底剔除。

### 1.5 对 iso-cc shim argv 契约的裁定

**契约成立。** 三种 cc 实际注入形态全部落在 execstub::parse_script 的识别表内：

1. MCP：`[shim, "<串>"]` → L1.5 单载荷分支 ✓（转宿主 `/bin/bash -c "<串>"`，语义 = cc 侧本应发生的 shell 解析移到宿主执行）。
2. Bash 工具/hooks/statusline：`[shim, "-c", "'<shim>' '<脚本>'"]` → L1/L2 `-c` 分支 ✓（载荷串转宿主；嵌套 shim 见缺口 G1）。
3. 常驻 stdio：MCP 传输的三根 pipe（cc netns 侧创建）随 SCM_RIGHTS 过墙，shim 阻塞在响应读 → **shim 进程存活期 = MCP server 存活期**，fd 级双向直通，无泵线程——常驻形态在现有设计内成立（§2 实测验证）。

---

## 阶段 2 —— iso-cc 全链实测（netns 引擎 + exec.bash=host + L1.5/L1/L2 注入）

实验场：`/tmp/iso-cc-exp24/lab/`
- `config.toml`：profile `mcp` = `{egress=if:eth0, net.scope=tree, exec.bash=host, agent.command=claude}` + env 透传（ANTHROPIC_BASE_URL/AUTH_TOKEN 取自宿主 settings.json，0600）；profile `mcpctl` = 同轴但无 exec.bash=host（对照组）。
- `mcp.json`：`{"mcpServers":{"playwright":{"command":"npx","args":["-y","@playwright/mcp@latest","--cdp-endpoint","http://127.0.0.1:9222"]}}}`。
- 负载：真 `claude -p --output-format stream-json --verbose --mcp-config mcp.json --allowedTools mcp__playwright__*`（工具白名单枚举）。
- 宿主 chromium 152 headless CDP :9222（票 23 联动形态；--disable-gpu 抗 WSL2 dzn 崩溃）。
- `observer2.sh`：0.4s 轮询进程树 + 关键进程深拍（argv/fd/ns/env 标记/CDP 可达性）。

### E1 —— 主链全绿：多轮 MCP 往返 + 常驻 + 落点取证（PASS）

会话 `mcp-2481111217`（`iso-cc run --profile mcp --config config.toml -- claude -p … --output-format stream-json --mcp-config mcp.json --allowedTools …`，退出码 0，58s，5 turns）。

**多轮工具往返（stream-json，e1/cc.stream.json）**：initialize/tools/list 由 cc 连接期完成（`mcp_servers: [{playwright: connected}]`）；随后 4 次 tools/call 全部成功：

1. `browser_navigate https://example.com` → Page Title: Example Domain
2. `browser_snapshot` → YAML 可达性快照
3. `browser_take_screenshot filename=exp24-e1.png` → 截图落盘（playwright 输出目录）
4. `browser_close` → 正常关闭

最终回答 = "Example Domain"。**会话期存活**：MCP server（node playwright-mcp）自 spawn 起持续存活至 claude 退出（observer tree.log 全程在册），stdio 链路 58s 内 6+ 次 JSON-RPC 往返无一失败。

**逐跳进程树 / fd / netns 归属（e1/deep-spawn-shim + deep-mcp-alive 快照，13:29:49）**：

```
[netns 4026532579 = 会话 netns]
  claude (.claude-wrapped)
    └─ shim pid 3847715  argv = [<sess>/bin/bash, "npx -y @playwright/mcp@latest --cdp-endpoint http://127.0.0.1:9222"]
         exe = target/debug/iso-cc（multi-call，argv0=bash）
         fd0 → socket:[53174268]  fd1 → socket:[53174271]  fd2 → socket:[53213185]   ← cc 传输三根 stdio
         fd3 → socket:[53210553]                                                   ← exec.sock 连接
         env: SHELL=<shim> CLAUDE_CODE_SHELL_PREFIX=<shim> ISO_CC_EXEC_SOCK=<exec.sock> ISO_CC_SESSION=mcp-2481111217
[netns 4026531833 = 宿主 netns]
  iso-cc 主进程 (pid 3847641)
    └─ worker pid 3847768  argv = [npm exec, @playwright/mcp@latest, --cdp-endpoint, http://127.0.0.1:9222]
         exe = node（bash -c 已 exec 成 npx→npm 同 pid）；netns = 宿主；ISO_CC_SESSION 标记在册
         fd0 → socket:[53174268]  fd1 → socket:[53174271]  fd2 → socket:[53213185]   ← 与 shim fd0/1/2 同 inode
        └─ node playwright-mcp pid 3847801（真 MCP server，宿主侧）
             fd0/1/2 → 同上三 inode；env ISO_CC_SESSION 在册；cwd = /tmp/iso-cc-exp24/lab（共享 fs 同路径零映射）
```

- **argv 契约留证**：shim 的 argv 恰为 `[prefix路径, 单载荷串]`，与 §1.2 源码结论逐字吻合（quote 原样：`/`、`@`、`:`、`.` 均在安全字符类）。
- **落点裁定**：MCP server（npm exec + node playwright-mcp）落在**宿主 netns**（4026531833），父链 = iso-cc 主进程（exec server fork）；shim 留会话 netns。→ `--cdp-endpoint http://127.0.0.1:9222` 直达宿主 chromium（ss: LISTEN 127.0.0.1:9222 chromium pid 3847205）——**host 落点身份语义：MCP server 及其浏览器流量的出口 = 宿主出口**（与 E4 对照组互补）。
- **SCM_RIGHTS fd 直通留证**：shim 与宿主 worker/node 三根 stdio 的 socket inode **逐一相同**（bun 以 AF_UNIX socketpair 充当 stdio pipe），跨 netns 双向零拷贝；exec.sock 连接（shim fd3）为唯一控制流。

注：本次运行全程无断链；前两次试跑中 chromium（WSL2 dzn GPU 路径）曾自行崩溃致 CDP 拒连，加 `--disable-webgpu --disable-software-rasterizer --disable-dev-shm-usage` 后稳定——环境脚注，非 iso-cc 缺陷。

### E1' —— teardown 收割（会话正常结束）

cc 退出 → shim 被 cc 侧回收 → exec.sock EOF → watcher 中断转发：`[iso-cc] exec-rpc: 对端断开 → 中断转发 worker pid=3847768（SIGKILL）`；npm exec 被杀 → node playwright-mcp 因 stdio EOF 自行退出（良态 stdio MCP server 语义），reap_adopted 扫描时该孤儿已处退出/僵尸竞态窗口，记录 `L2 登记: 收养孤儿 pid=3847801 无 ISO_CC_SESSION 标记——不杀`。**存活期标记核验**：deep-mcp-alive 快照实证 node 进程 `/proc/<pid>/environ` 含 `ISO_CC_SESSION=mcp-2481111217`（"无标记"是扫描时点进程已亡的读失败伪象，非 env 剥离——对照 probe 实验双重证实：`ISO_CC_SESSION=probe-12345 npx …` 链上 npm exec 与 node 子进程标记均 =1）。会话后无残留进程。竞态窗口的登记语义见缺口 G3。

### E1b —— 复现性对照（PASS）

第二跑（会话 `mcp-508758097` 之外的独立运行，41–53s）三次顺序导航 `example.com → iana.org → rfc-editor.org` 全部成功，三个标题全部正确回读。**主链结果可复现**。

### E2 —— 失败语义①：MCP server 中途死亡 → cc 侧 pipe EOF 行为（PASS，透明重连）

方法：killer 脚本在 MCP server node 进程出现 18s 后对其 SIGKILL（会话 `mcp-580519084`，13:40:08 击杀 pid 3862885；此时导航 1/2 已成功）。

**观测（e2/cc.stream.json + e2/tree.log）**：

1. 击杀 → 第一链 stdio EOF：npm exec（worker）随子进程死亡退出，cc 传输读到 stdout EOF/进程退出 → transport onclose → server 标记断连。**模型层零错误透出**。
2. 下一次 tools/call（导航 3，rfc-editor.org）时 cc **自动重连**：重新经同一 L1.5 通道 spawn 第二链——tree.log 实证第二链 node pid 3863906（父 = 第二 worker 3863885，13:40:11 出现，即击杀后 3s 内），落点仍为宿主侧。
3. 导航 3 成功返回 "RFC Editor"；最终四个标题（此跑三次）全部正确。会话 teardown 时收割的正是第二链：`对端断开 → 中断转发 worker pid=3863885（SIGKILL）`。
4. 会话后无残留进程（pgrep playwright-mcp/npm exec = 空）。

**结论**：常驻 stdio MCP server 被 kill 后，cc 2.1.263 的行为 = **pipe EOF → 断连标记 → 下次调用透明 respawn**（同 L1.5/exec.sock 通道、同宿主落点），对模型对话流不可见。iso-cc 侧无需任何额外处理即保持语义（每条链独立走 SCM_RIGHTS 直通与收割）。

### E3/E4 —— 失败语义②：会话 teardown / cc 突死 → 三层收割全链留档（PASS）

E3（cc 被 SIGKILL，会话 `mcp-2479444552`）与 E4（带逐 0.5s census 的重演，会话 `mcp-3524863340`）实测死亡序列：

```
T+0     SIGKILL claude（仅此一个 pid；pre-kill census: claude/shim/worker/node 四者俱在）
T+0~0.5 shim、node 消失，exec.sock 消失                    ← 见下「杀手是谁」
T+~0.5  [iso-cc] exec-rpc: 对端断开 → 中断转发 worker pid=3876262（SIGKILL）   ← watcher 路径
T+~1    Session::wait 收尾：sess_dir + exec.sock 资产删除（mcp-2479444552 已验证不存在）
退出码  iso-cc 退出 137 = 会话根状态忠实传播（128+SIGKILL）
事后    pgrep playwright-mcp/npm exec = 空（零残留）
```

**杀手是谁（实测排除法 + 对照实验）**：非 Bun PDEATHSIG（本机 bun 与 bun×node:child_process 双探针：父 SIGKILL 后子进程均存活）；非 reap_adopted（其在 watcher 之后才运行）。**定性 = pidns 内核语义**：pasta spawn 模式把整棵会话树放进 PID namespace（对照实验：netns 会话内 `nohup sleep 500 &` 的后台进程在根命令退出后即刻被杀，`bgpid=2` 证实 pidns 存在）；cc（bootstrap exec 的同一 pid）= pidns 的 1 号进程，1 号死亡 → 内核 SIGKILL 全体成员 → shim 即刻死亡 → exec.sock EOF → watcher 接管。与 ADR 附「stub 随 cc 退出」的设计预期一致；ExecServer::shutdown 的登记簿「在途 worker SIGKILL」路径对 cc 面降级为防御纵深（非 pidns 常驻的替身驱动才可达），mark/替身场景仍有意义。

### E4 对照组 —— netns 落点（matrix#5，PASS）

会话 `mcpctl-3784608276`（同负载，无 exec.bash=host → 无 L1.5/L1/L2 注入）：

- **落点**：MCP server 链全部在会话 netns 内——node playwright-mcp `net:[4026532578]`（宿主 = 4026531833），父链 `claude(3887562) → npm exec(3887640) → node(3887680)`，全程零 shim、零 exec.sock（tree.log 无 `bin/bash npx` 行）。transport 直接 spawn `npx …`（§1.2：prefix 未设时 `D=t.command, y=t.args` 原样 argv）。
- **身份语义裁定**：MCP server 自身流量（npm registry 等）走会话出口（pasta → eth0）= 身份一致形态；本次 navigate 经 `--cdp-endpoint 127.0.0.1:9222` 仍成功——pasta 默认 host-loopback 映射（`--map-host-loopback` 默认行为，pasta.rs:16 与票 19/20 交付面）把 netns 侧 127.0.0.1 拨号翻译到宿主 loopback，故浏览器本体仍 = 宿主 chromium = 宿主身份。**完全身份一致（浏览器也在 netns、流量走会话出口）需 playwright 自带 chromium 在 NixOS 的 FHS 适配**，联动票 23 落地票，不在本票范围。
- 两模式对照一句话：**host 落点（E1）= spawn 面出沙箱、执行与流量全宿主；netns 落点（E4）= spawn 面在沙箱、MCP server 流量走会话出口、经 loopback 映射可驱动宿主浏览器**。二者皆为合法形态，由 exec.bash 轴显式声明选择。

---

## 缺口清单（草稿）

### G1（阻断级，live 实锤）：exec.bash=host 下 cc Bash 工具面不通

E5 实测（会话内 `claude -p` 驱动 Bash 工具执行 `echo probe-$((6*7))`）：**Exit 126**，tool_result 原文：`[iso-cc exec stub]` "`-c` 后参数数异常（2）：仅接受单段脚本"。两层失配，须同修：

- **G1a argv 形态**：cc 2.1.263 Bash 工具真实 spawn = `<shim> -c [-l] <payload>`（`getSpawnArgs`：snapshot 缺席时夹 `-l`；snapshot 创建本身依赖 `$SHELL` 探测 → 在 host 轴被 shim 级联破坏，故实际恒见 `-l`）。execstub::parse_script 只容忍 `-c` **前**的 flag（`-l -c`），对 `-c` 后的 `-l`（`-c -l payload`）按「-c 后参数数 = 2」拒收。证据：E5 stream-json + execstub.rs:36-44。
- **G1b 载荷二次进 shim**：过 G1a 后，payload = `'<shim路径>' '<script>'`（Yhe 产物，§1.3）被转发为宿主 `/bin/bash -c "'<shim>' '<script>'"` → 宿主侧再次 exec shim（L1.5 单载荷形态）→ 需要 `ISO_CC_EXEC_SOCK`，而 `worker_env_vars`（execrpc.rs:118-134）不含它 → 126 `ISO_CC_EXEC_SOCK 未设置`。hooks/statusline 走同一 Yhe 管线，同受累。
- **修复方向**（票 25 草）：① parse_script 容忍 `-c` 与 script 之间的 flag 段（`-l`）；② `worker_env_vars` 注入 `ISO_CC_EXEC_SOCK`（sock 路径已在 ExecChannel 上）→ 第二跳 shim 经同一通道再转发，一次额外 RPC 跳，终态语义不变（脚本由宿主 `/bin/bash -c` 执行）。不采纳：server 侧解析 Yhe 产物剥壳（引号逻辑复制进 server，脆弱）。

### G2（行为定性，无需修）：shutdown 登记簿路径对 cc 面不可达

pidns 语义（E3/E4）使 shim 必然先于 shutdown 死亡，watcher 路径恒先接管；登记簿「在途 worker SIGKILL」仅对替身驱动/非 pidns 常驻场景可达。登记为行为文档即可（防御纵深保留）。

### G3（化妆级，可顺手修）：reap_adopted 竞态窗口误记

worker 被 SIGKILL 后，良态 stdio MCP server 因 stdin EOF 自退；reap_adopted 扫描若落在退出/僵尸窗口，`/proc/<pid>/environ` 读失败 → 误记 `无 ISO_CC_SESSION 标记——不杀`（本票 probe 已证标记实际在册）。修复：读失败（ESRCH/ENOENT）与「确无标记」分支分记；或扫描前先非阻塞收尸。

### G4（环境脚注，非缺陷）

- WSL2 chromium headless 走 dzn/MESA 路径会静默崩溃（两次实测），需 `--disable-gpu --disable-webgpu --disable-software-rasterizer --disable-dev-shm-usage`；E1 首跑的 CDP ECONNREFUSED 即此因。
- 长驻进程（chromium）若由短命 shell 命令后台化，父命令结束后会被清理方收割——实验负载应挂到会话级服务管理。

---

## 修复票草（25 — execstub Bash 工具形态修复 + 收割日志分记）

**What to build:** 票 24 E5 实锤的 G1a/G1b：exec.bash=host 下 Bash 工具（含同管线 hooks/statusline）126 不通。修复后该面的宿主执行声明（ADR 0008 附 3 L1.5 覆盖声称）首次全链成立。

**Blocked by:** 无　**Owner:** 待派

**Change:**
1. `execstub::parse_script`：`-c` 后允许 flag 段（现知 `-l`），script = 其后唯一剩余参数；保持 fail-loud（仍拒绝多脚本参数）。单测补 cc 2.1.263 真实形态回放：`["-c","-l",payload]`、`["-c",payload]`、`[payload]`。
2. `execrpc::worker_env_vars`：注入 `ISO_CC_EXEC_SOCK`（ExecChannel 已有 `sock_path`；WorkerSpec 冻结时随带）→ Yhe 载荷第二跳 shim 经同通道转发。注意：第二跳为 L1.5 单载荷形态，须与修复①叠加。
3. `Session::reap_adopted`：`proc_marker` 读失败分支与「确无标记」分支分记日志（文案区分 race/真缺失）。

**Acceptance:**
- [ ] 复现脚本（本票 lab E5）：`iso-cc run --profile <host轴> -- claude -p "Bash echo probe"` → tool_result 含 `probe-42`、exit 0
- [ ] MCP 链回归：本票 E1 脚本仍全绿（L1.5 MCP 面不回归）
- [ ] nextest/clippy 零回归

**边界:** 不改 20/23 在途面；不动 pidns 语义（G2 仅文档）。

---

## 验收对照

| 票面 Acceptance | 状态 | 证据 |
|---|---|---|
| ① spawn 机制 argv/落点留证；机制结论 | ✅ | §1（源码偏移级：MCP = prefix 直 exec + 单载荷串 @204764811；Bash/hooks = Yhe 拼串；REPL = /bin/sh 硬编码；SHELL 不参与 MCP spawn 载体）+ E1（live argv 逐字吻合、落点宿主）+ E4（对照组落点 netns）|
| ② 真 @playwright/mcp 常驻链全绿 | ✅ | E1：initialize/tools/list + 4 次 tools/call 全成功、会话期 58s 存活、teardown 收割；E1b 三导航复现；SCM_RIGHTS 同 inode 留证 |
| ③ 失败语义两例留档 | ✅ | worker 死 → E2（pipe EOF → cc 透明 respawn）；会话 teardown/突死 → E1'/E3/E4（pidns 内核语义 → watcher 收割 → 资产清理 → 退出码传播 → 零残留）|
| ④ 缺口清单 + 修复票草 | ✅ | G1（阻断，双层失配 live 实锤）+ G2/G3/G4 + 票 25 草；不阻塞报告 |
| ⑤ clippy/nextest 不回归（如有代码修复） | N/A | 本票零仓库改动（纯 /tmp 实验 + 本报告），无代码修复 |

## 禁区核对

仓库零改动（仅读）；.scratch/docs 只读；未 git；未碰 mihomo-sg-tmp。宿主面变更：`~/.config/iso-cc/config.toml` 追加 mcp/mcpctl/mcpctl2 profile（沿用票 15/23 宿主配置惯例）、`iso-cc setup` 背书对、实验会话 state 资产（已随 teardown 回收）。

## 复现索引

| 步骤 | 产物 |
|---|---|
| 阶段 1 取证 | `/tmp/iso-cc-exp24/ctx.py`（regex 窗口提取器）、`cc-pkg/package/claude`（官方二进制）、`cc-strings.txt` |
| 标记对照 probe | `/tmp/iso-cc-exp24/marker-probe.sh`（npm exec/node 标记继承核验） |
| E1/E1b/E2/E3/E4 主链 | `lab/`：`config.toml`、`mcp.json`、`observer3.sh`、`e{1,2,3,4}/{cc.stream.json,cc.err,tree.log,deep-*/snapshot.txt,killer.log}` |
| E5 Bash 工具探针（票 25 回放素材） | `lab/e5/cc.stream.json` |
| Bun/PDEATHSIG 排除 | `/tmp/iso-cc-exp24/pds.js`、`pds2.js` |
| pidns 对照实验 | `lab/ctl.err` + 「NETNS BG KILLED AT ROOT EXIT」输出（本报告 E3/E4 节） |

## 结论一句话

**stdio MCP 全链（cc netns → L1.5 prefix → multi-call shim → exec.sock RPC → 宿主 spawn → 真浏览器负载 → teardown 收割）机制与实测双向闭环：机制 = prefix 直 exec + 单载荷串（非拼串、SHELL 无关）；常驻 = SCM_RIGHTS fd 直通天然支持；失败语义双向可观测且无泄漏；唯一阻断缺口在 Bash 工具面（G1），已交票 25 草。**
