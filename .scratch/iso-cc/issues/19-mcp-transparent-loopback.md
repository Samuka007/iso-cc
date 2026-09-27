# 19 — 可行性探究：MCP 透明回环（读取 cc MCP 配置 → 自动端口映射）

**What to build（探究后落地）:** 读取当前 claude code 的 MCP 配置，找指向 `127.0.0.1`/`localhost` 的 HTTP/SSE MCP 条目，在 netns 内自动建端口映射，使 cc 对 `127.0.0.1:<port>` 的连接透明到达宿主服务——cc 配置零改动（用户提案 2026-09-27）。

**Blocked by:** 无　**Owner:** lane-mcp-loop-probe（探究完成）
**Status:** 探究 done（2026-09-27，REPORT 174 行，parent 抽验锚点属实）

## Answer（PM 落）

- **结论：可行，且比提案更省——passt 2026_07_16 默认已原生镜像**：attach 时刻宿主全部 TCP 监听端口自动镜像为 netns 内 `[::]:PORT` 双栈监听，cc 拨 `127.0.0.1:<同端口>` **零组件**透明到达宿主（loopback-only 的 mihomo 7891 也通，204×4 实测）。原型 NATIVE-MIRROR-ROUNDTRIP OK
- 机制择型：**c) pasta 原生 ≫ a) socat per-port（作缺口兜底）＞ b) nft DNAT**（三开关缺一即超时黑洞，难诊——F1/F2 对照实测）
- **关键边界（落地票输入）**：镜像集 = attach 时刻快照（attach 后启动的服务不补，实测 20s）→ 缺口补丁 = socat per-port（已证可用）；治理 flag 是 `-T/-U` 非 `-t/-u`，且显式 `-T SPEC` 会关闭全部 loopback 捕获（外科手术式排除被证伪）；冲突处置 = 会话服务避让 + Warn（EADDRINUSE 多次实测）；生命周期 = session-scoped 无需 manifest（b 规则随 netns 消亡已证；a 孤儿转发进程会拽住 netns 需先收割）
- mark 引擎无关（无 netns 天然直达）；本机文件态 http/sse loopback MCP = 0 条（脱敏清单在 REPORT §1；真实环境才有数据）
- 落地票（20）：探针 + 缺口 socat 编排 + 解析器，估 2 人日，REPORT §8 有文件集；**扫描源必须是 profile 的 claude.json（ADR 0006 重定向后）**

## 提案原文（用户）

"如果 netns 要考虑 mcp 的话，直接写一个读取当前 claude code 的 mcp 配置，找那些需要指向 127.0.0.1 的 http mcp，做个从外部 ns 到内部 ns 的端口映射，不就好了"

## 要探究的问题

1. **配置源与解析**：cc 的 MCP 配置分布在哪些文件（`~/.claude.json` mcpServers、项目 `.mcp.json`、managed settings）？http/sse 型条目的 URL 形态枚举（含 `localhost`、`[::1]`、非默认端口）；stdio 型不适用本机制（走 L1.5 shell prefix，ADR 0008 附3）
2. **方向确认**：MCP server 在宿主（外部）→ cc 在 netns（内部）；cc 连自己的 `127.0.0.1:PORT`（netns loopback）→ 需要在 **netns 内**监听 `127.0.0.1:PORT` 并转发到宿主侧同一端口（经 pasta 网关地址，map-host-loopback 已证 issue 10）。票 15 的 exec.bash=host 不适用（MCP 由 cc 直 spawn，非 bash）
3. **机制候选**（零自研优先）：
   - a) netns 内 socat per-port（`TCP-LISTEN:PORT,bind=127.0.0.1,fork TCP:<gw>:PORT`，marked 子进程归 13 收割）
   - b) netns 内 nftables DNAT（`127.0.0.1:PORT → gw:PORT`，需 route_localnet=1——netns 作用域无害；零常驻进程）
   - c) 其他一手来源发现的更优形态
4. **端口冲突语义**：netns 内 PORT 已被会话内进程占用（会话自己的服务）→ 跳过该映射并 Warn（本地服务优先，不该转发到宿主）
5. **生命周期**：映射集合 = 会话资产；bootstrap 期建、teardown 收；清单/manifest 是否需要登记（推测不需要——session-scoped，探针可证）
6. **与 mark 引擎的关系**：mark 无 netns → 127.0.0.1 天然直达，本机制仅在 engine=netns 下有意义（探针引擎无关）

## Acceptance（探究报告，非实现）

- [ ] 配置源实测：本机真实 cc 配置文件里 http 型 MCP 条目清单（脱敏：只记端口与形态，不记 URL 路径/token）
- [ ] 机制择型：a/b/c 对比（进程面/规则面/边界语义）+ 推荐
- [ ] 原型证据：/tmp 一次性脚本，netns 内 `curl 127.0.0.1:<port>` 打通宿主服务（round-trip 输出）
- [ ] 冲突语义与生命周期结论；落地方案票草（工作量估计、文件集）

## 边界

- 探究 = /tmp 脚本 + 报告，不改仓库代码；隐私：MCP URL 里的 token/路径不入票面与报告（只记 host:port 形态）；不 git
