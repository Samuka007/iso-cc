# iso-cc

在任意 Linux 发行版上，以 rootless、声明式、零持久状态的方式，为 Claude Code（或任意命令）拉起「网络出口定向 + locale 覆盖 + CC 配置隔离」的轻量会话。失败即断（fail-closed）：接口缺失、网关死亡、代理死亡均不回落直连。

- 需求基线：[docs/REQUIREMENTS.md](docs/REQUIREMENTS.md)
- 词汇表与不变式：[CONTEXT.md](CONTEXT.md)
- 依赖面：[docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)
- 决策记录：[docs/adr/](docs/adr/)
- 变更史：[CHANGELOG.md](CHANGELOG.md)

## 安装

**一键（bundle，零发行版依赖）**——装到 `$PREFIX/bin/iso-cc` + `$PREFIX/lib/iso-cc/libexec/`（默认 `PREFIX=/usr/local`，需 root；helper 经 exe-relative 解析，零配置）：

```bash
curl -fsSL https://raw.githubusercontent.com/Samuka007/iso-cc/main/packaging/install.sh -o iso-cc-install.sh
sudo sh iso-cc-install.sh install          # 最新 release；或显式版本：install 0.1.0
```

离线/自定义前缀：`PREFIX=~/.local sh iso-cc-install.sh install --from <bundle.tar.gz 或解包目录>`；卸载见[下文](#卸载)。

**手动（bundle tar.gz）**：

```bash
VER=0.1.0 ARCH=amd64   # arch: x86_64→amd64, aarch64→arm64
curl -fsSLO "https://github.com/Samuka007/iso-cc/releases/download/v$VER/iso-cc-bundle-$VER-linux-$ARCH.tar.gz"
curl -fsSLO "https://github.com/Samuka007/iso-cc/releases/download/v$VER/sha256sums.txt"
sha256sum -c --ignore-missing sha256sums.txt
tar xzf "iso-cc-bundle-$VER-linux-$ARCH.tar.gz"
sudo install -m0755 "iso-cc-bundle-$VER/iso-cc" /usr/local/bin/
sudo mkdir -p /usr/local/lib/iso-cc && sudo cp -r "iso-cc-bundle-$VER/libexec" /usr/local/lib/iso-cc/
```

> 只解包不安装也可以：`export ISO_CC_HELPER_DIR=$PWD/iso-cc-bundle-$VER/libexec` 指向 libexec 即可。

**发行版原生包（deb/rpm/apk）**：从 [Releases](https://github.com/Samuka007/iso-cc/releases) 下载 `iso-cc-<ver>-linux-amd64.{deb,rpm,apk}`；`Depends: passt`（slirp4netns→Suggests，tun2proxy→Recommends，版本下限见包元数据）。AMD64 亦有自建 APT registry（`apt.samuka007.com`，suite `stable`/component `main`），接入方式见 [docs/P1-r2-aptline.md](docs/P1-r2-aptline.md)。此形态 helper 走 PATH，建议装完跑一次 `iso-cc setup` 登记（doctor 漂移比对生效）。

**Nix**：

```bash
nix run github:Samuka007/iso-cc -- --version
```

## 快速上手

最小配置（`~/.config/iso-cc/config.toml` 全局，或项目根 `.iso-cc.toml` 覆盖；完整字段见 [examples/config.example.toml](examples/config.example.toml)）：

```toml
version = 1

[profile.sg]
egress = "if:wg0"          # 只引用，不管理（隧道侧负责接口存在且有路由）
net.engine = "netns"       # 默认引擎
locale.tz = "Asia/Singapore"
agent.command = "claude"
```

```bash
iso-cc run                            # 只打印等价执行计划（--print-plan 同）
iso-cc run -- claude "hi"             # 会话内执行命令
iso-cc verify --profile sg            # 会话内纯净度探针（红绿 + JSON）
iso-cc doctor                         # 配置 vs 宿主现实全量 diff（任何 FAIL → exit 1）
```

## 双引擎与 egress 形态

| `net.engine` | `egress` 形态 | 会话根 | 说明 |
|---|---|---|---|
| `netns`（默认） | `if:<iface>` | userns+mountns+netns，pasta spawn（tap0） | 出口跟随宿主接口事实；`net.gateway = "pasta" \| "slirp4netns"` 可选网关 |
| `netns` | `socks5://<host>:<port>` | 同上 + netns 内 tun2proxy worker（tun1） | DNS 全程隧道内经代理远端解析；宿主 mixed-port 经 `--map-host-loopback` 可达 |
| `mark` | `if:<tun-iface>` | 零 netns，整树 uid=4210 策略路由 | 仅隧道 TUN 设备；`socks5`/`net.*` netns 轴声明性拒绝；setup 输出 rootful 步骤待人工审计应用 |

IPv6 默认 `off`（fail-closed）；`net.scope = "tree" | "self"` 控制整树进 netns 还是仅 cc 本体。

## exec.bash 双轴与 MCP 行为

`exec.bash = "sandbox" | "host"`（默认 `host`）是独立于引擎/egress 的执行轴：`host` 时会话内 Bash 工具/hooks 经 exec.sock RPC 回宿主侧执行（SCM_RIGHTS stdio 直通，L1/L1.5/L2 拦截分层）；`sandbox` 全程会话内。

MCP：stdio MCP server 由宿主侧 spawn（L1.5 grounding）；netns 形态下 pasta 默认把宿主 loopback 服务镜像进会话，故 HTTP/SSE MCP 直连 `127.0.0.1` 可用。**快照边界**：attach 之后才启动的宿主服务不可达——先起服务再进会话，或重启 cc。

## 生命周期

- `iso-cc setup`：收敛 setup-manifested 资源并登记清单（幂等，重跑 diff = 0；mark 引擎在此输出 rootful 步骤）
- `iso-cc doctor`：配置 vs 宿主现实全量 diff（provider 漂移/接口/路由面/capability）
- `iso-cc list`：存活会话枚举
- `iso-cc gc`：默认 sweep 报告；`--prune` 清 stale 登记；`--all` 全量回收（有活跃会话拒绝；provider 二进制永不删除，只除名）

## 卸载

```bash
sudo sh iso-cc-install.sh uninstall      # 删 $PREFIX/bin/iso-cc + $PREFIX/lib/iso-cc/libexec
```

用户态残留（会话/profile-state）随后清理：`iso-cc gc --all`（有活跃会话会拒绝）。

## 开发

```bash
nix develop          # rust stable + musl target + clippy/rustfmt + pasta + slirp4netns
cargo build
nix build .#checks.x86_64-linux.clippy   # -D warnings
```

> 仓库刻意不提交 `result*`（nix GC 根）与 `target/`；flake.lock 锁定 rust-overlay 版本，工具链版本只随锁更新演进，`nix collect-garbage -d` 定期清理旧闭包。
