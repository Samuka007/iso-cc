# 22 — 删除 MCP 快照缺口 socat 兜底（契约改为：服务先起，否则重启 cc）

**What to build:** 移除票 20 的 `net.mcp_fallback` socat 兜底机制（用户裁决 2026-09-27：给时序错位打补丁不优雅；正确契约 = **MCP server 先于 cc 启动，否则重启 cc 是合理操作**）。

**Blocked by:** 21（完成中）　**Owner:** 待派

## Specification

1. **删**：`net.mcp_fallback` config 轴、bootstrap 期 `mcp_gap_fallback`（探活+socat spawn 编排）、plan 中对应行、相关单测
2. **留**：P-MCP 探针本体（三态判定核保留，但语义收窄为两态：可达=绿 / 不可达=红 + 修复指引「先启动 MCP server 或重启 cc；快照边界见票 19 §3.2」）；`net.mcp_fallback=false` 轴删除后旧配置文件含该键 → deny_unknown_fields 报错（破坏性配置变更，CHANGELOG 注明）
3. doctor：mcp/loopback-ports 汇总保留，Warn 文案改「声明端口不可达（快照边界）——重启 cc 或先起服务」；不再有"兜底将补齐"分支
4. EADDRINUSE 跳过逻辑随兜底一起删除（无 socat 即无冲突面）；原生镜像覆盖的端口照旧绿

## Acceptance

- [ ] config 无 `net.mcp_fallback`；含该键的旧配置 → 明确报错（fail-loud 语义）
- [ ] 原生镜像端口 P-MCP 绿（不回归）；attach 后才起的宿主服务端口 → P-MCP 红 + 修复指引（取代原兜底绿）
- [ ] mark 引擎 P-MCP SKIP 不变；doctor 汇总无兜底分支
- [ ] clippy -D warnings 绿；nextest 全绿；其余探针不回归

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾
