# Changelog

> 版本方案：0.0.x = 草稿期（draft）。按 Cargo 语义 `^0.0.x` 仅精确匹配同 patch——**每次 0.0.x 发布都可能 breaking**，依赖方须精确钉版本。跳 0.1.0 = 承诺 minor 位兼容边界；跳 1.0.0 = 承诺配置格式 + CLI 面 + env 契约向后兼容。

## 0.0.1（2026-09-29）

初始草稿版。rootless 声明式沙箱会话：双引擎 + 三形态出口 + CC 配置隔离 + MCP 透明回环 + 四件套打包。

- 双引擎：`engine = "netns"`（userns+mountns+netns，pasta spawn 会话根）| `"mark"`（零 netns，uid 策略路由 + mihomo TUN，localhost 双向零摩擦）
- 出口三形态：`net.egress = "if:<iface>" | "socks5://<host>:<port>"`；fail-closed 贯穿（接口缺失/网关死亡/代理死亡均不回落直连）
- 隔离：`agent.cc_isolation`（默认 true）——`~/.claude*` 内置重定向对 → 持久 profile-state + P8 写集探针
- 拦截去注入化：L3 mountns bind（/bin/bash、/bin/sh → 会话 shim；存在性过滤）+ L2 PATH 承担全部拦截；会话 env 零 hook 点名变量
- `exec.bash = "sandbox" | "host"`：宿主侧执行经 exec.sock RPC（SCM_RIGHTS stdio 直通，常驻长生命周期已实测）
- MCP：stdio MCP 宿主侧 spawn（L1.5 直 exec 单载荷，源码级证实）；pasta 原生镜像覆盖宿主 loopback 服务（P-MCP 探针；快照边界：attach 后启动的宿主服务不可达——先起服务或重启 cc）
- 工具链：setup/gc/doctor/list；verify 探针（socks 形态下全绿 SG 出口）；PROXY 族 env scrub
- 打包：nfpm deb/rpm/apk（Depends 仅 passt）+ bundle tar.gz（libexec 三 helper）+ tag 触发 release workflow + R2 APT registry 自动入池
- BREAKING（相对草稿期内含语义）：`net.mcp_fallback` 键已删除（快照缺口不自动兜底）

## 历史注记

v0.1.0 / v0.2.0 曾短暂发布后按操作者裁决作废（版本方案重置为 0.0.x 草稿期）；相关 tag/release/池内 deb 已清理。
