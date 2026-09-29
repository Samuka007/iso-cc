# Changelog

## 0.2.0（2026-09-29）

- 拦截去注入化：L3 mountns bind（/bin/bash、/usr/bin/bash、/bin/sh → 会话 shim）+ L2 承担全部拦截；会话 env 零 hook 点名变量（BREAKING：CLAUDE_CODE_SHELL_PREFIX 不再注入）
- Bash 工具 host 轴阻断级修复：`-l` flag 容忍 + 二跳 sock 注入（stdio MCP 常驻链同票实测全绿）
- exe-relative 解析层：manifest > ISO_CC_HELPER_DIR > <exe_dir>/../lib/iso-cc/libexec > PATH——install.sh 安装布局零配置解析
- packaging/install.sh：install/uninstall、sha256 校验、PREFIX 覆盖、--from 离线模式
- PROXY 族 env scrub（防出口配置泄漏经继承代理绕过隧道）
- MCP 快照缺口兜底删除（契约：服务先起或重启 cc；BREAKING：net.mcp_fallback 键删除）
- README 全面重写；依赖降级（Depends 仅 passt；slirp4netns→Suggests，tun2proxy→Recommends）

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

### 安装（票 27）
- `packaging/install.sh`（POSIX sh，装/卸）：GitHub Release 拉 bundle + `sha256sums.txt` 强校验；`--from <dir|tar.gz>` 离线模式；`PREFIX` 可覆盖（默认 /usr/local）；幂等覆盖安装，装完 `iso-cc --version` 自证
- exe-relative libexec 解析层：`pinned_bin` 优先级链插入第三层——manifest 钉路径 > `ISO_CC_HELPER_DIR` > `<exe_dir>/../lib/iso-cc/libexec`（current_exe 解析符号链接）> PATH；bundle 经 install.sh 装到 PREFIX 后零配置命中（无需 `ISO_CC_HELPER_DIR`、无 PATH 依赖）；dev/nix 构建该层自然不存在，回落 PATH 无害

### BREAKING（相对无 CHANGELOG 前的内部版本）
- `net.mcp_fallback` 键已删除（快照缺口不再自动兜底），旧配置拒绝解析
- 依赖降级：deb/rpm/apk `Depends` 仅 passt（slirp4netns→Suggests，tun2proxy→Recommends）
