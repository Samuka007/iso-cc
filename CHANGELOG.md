# Changelog

## 0.1.0（2026-09-27）

首个可用里程碑：rootless 声明式沙箱会话，双引擎 + 三形态出口 + 全绿 verify（SG 出口形态）。

### 引擎与网络
- `engine = "netns"`（默认）：userns+mountns+netns，pasta spawn 为会话根（R8 结构性 fail-closed）；`net.egress = "if:<iface>" | "socks5://<host>:<port>"`（后者 netns 内 tun2proxy 组合）
- `engine = "mark"`：零 netns，uid 策略路由（setup-manifested 路由面 + unreachable 兜底），localhost 双向零摩擦
- `net.gateway = "pasta" | "slirp4netns"`、`net.dns`、`net.publish`（规划中）、失败即断（fail-closed）贯穿：接口缺失/网关死亡/代理死亡均不回落直连

### 隔离与身份
- `agent.cc_isolation`（默认 true）：`~/.claude*` 内置重定向对 → 持久 profile-state；P8 写集探针（实测写集 ⊆ 声明集 ∪ 白名单）
- locale：TZ env + tzdb 内嵌 TZif + CLAUDE_CONFIG_DIR unset
- MCP：stdio MCP 经 L1.5 宿主侧 spawn（票 24 grounding）；pasta 原生镜像覆盖宿主 loopback 服务（P-MCP 探针；**快照边界**：attach 后启动的宿主服务不可达——先起服务或重启 cc）

### exec.bash 轴（票 15/25）
- `exec.bash = "sandbox" | "host"`：宿主侧执行经 exec.sock RPC（SCM_RIGHTS stdio 直通）；L1/L1.5/L2 拦截分层；host 轴 Bash 工具/hooks 形态已修复（票 25）

### 工具链
- setup/gc/doctor：两级资源模型（session-scoped / setup-manifested / residue=0），清单双向校验
- verify：P1/P2/P3b/P6/P8/P13/P15 + P-MCP；socks 形态下全绿（SG 出口）

### BREAKING（相对无 CHANGELOG 前的内部版本）
- `net.mcp_fallback` 键已删除（快照缺口不再自动兜底），旧配置拒绝解析
- 依赖降级：deb/rpm/apk `Depends` 仅 passt（slirp4netns→Suggests，tun2proxy→Recommends）
