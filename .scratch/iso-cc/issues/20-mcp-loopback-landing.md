# 20 — MCP 透明回环落地：P-MCP 探针 + 快照缺口 socat 兜底

**What to build:** 票 19 探究结论的产品化（薄）：pasta 原生镜像已是默认行为（零组件），落地 = 探针 + 缺口兜底 + 配置感知告警。

**Blocked by:** 无（05/09 已合入）　**Owner:** 待派

## Specification（依据 19 REPORT §8 票草 + PM 收窄）

1. **P-MCP 探针**（probe.rs）：会话内对声明的 MCP loopback 端口逐个 `curl 127.0.0.1:<p>`（探活 = connect 成功即可，不解析协议）；无声明条目时 SKIP。扫描源 = **profile 的 claude.json（ADR 0006 重定向后路径）**+ 项目 .mcp.json；只取 scheme=http/https|sse 且 host ∈ {127.0.0.1,localhost,[::1]} 的端口
2. **快照缺口兜底**：bootstrap 期（wait_ready 后、exec 前）对探活失败的声明端口起 socat（`TCP-LISTEN:<p>,bind=127.0.0.1,fork,reuseaddr TCP:<gw>:<p>`，marked 子进程，teardown 先于 pasta 收割——孤儿会拽住 netns，19 已证）；默认开启，config `net.mcp_fallback = true|false`
3. **冲突语义**：netns 内端口已被会话进程占用（EADDRINUSE）→ 跳过 + Warn（本地服务优先，19 已证）
4. doctor：声明端口 vs 探活结果汇总（engine=netns 限定；mark 引擎 SKIP）
5. 不做：读配置自动映射器主体（原生镜像已覆盖）、pasta `-T` 外科排除（被证伪）

## Acceptance

- [ ] 合成 MCP 配置 + 宿主服务：会话内 P-MCP 探针绿（原生镜像路径，零 socat）
- [ ] 快照缺口：宿主服务 attach 后启动 → 兜底 socat 补齐探针绿；`net.mcp_fallback=false` 时该端口探针红
- [ ] 冲突：会话自占端口 → 跳过 + Warn 输出
- [ ] mark 引擎下 P-MCP SKIP 取证
- [ ] clippy -D warnings 绿；nextest 全绿；探针套件不回归

## 轮子盘点

socat 现成（19 已证）；解析器自研 ~80 行（cc 配置无 SDK，ADR 0006 已拒 envvar 路线）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾
