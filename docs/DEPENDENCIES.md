# iso-cc 依赖面清单

状态：v1（2026-09-26）。本机实证 = NixOS WSL2 内核 6.6，`unshare -Urn`/`-Urm` + make-rprivate + bind 实测通过（见 §5）。

## 0. 核心判词

- **T2（最小会话：userns+mountns+locale 覆盖+env 注入+exec）的运行时外部依赖 = 0 个二进制**——只有内核特性（§1）。locale 覆盖用我们自己的 mountns bind，不需要 bwrap。
- **T3（网络定向）+1**：pasta（或 slirp4netns 回退），会话里唯一的外部二进制。
- iso-cc 本体 = musl 静态链接，运行时零库依赖。

## 1. 内核特性（不是包）

| 特性 | 需求 | NixOS（本机实测） | Ubuntu 22.04（4090 实测） | Debian 12 | Arch | Ubuntu 24.04 |
|---|---|---|---|---|---|---|
| CONFIG_USER_NS + 非特权可用 | 必须 | ✅（无限制 sysctl，`unshare -U` 通过） | ✅ `kernel.unprivileged_userns_clone=1` | ✅ 默认 | ✅ 默认 | ⚠️ AppArmor 限制（P3：随包 profile） |
| `user.max_user_namespaces` > 0 | 必须 | ✅ 71895 | ✅ ≈2.06M | ✅ 默认 63k？doctor 校验 | ✅ | ✅ |
| CONFIG_NET_NS / mountns / bind mount | 必须 | ✅（`unshare -Urn`/`-Urm` 通过） | ✅ | ✅ | ✅ | ✅ |
| landlock（pasta 自sandbox 可选用） | 可选 | 内核 ≥5.13 即有 | 5.19 ✅ | 6.1 ✅ | ✅ | ✅ |

无 Debian 系 `kernel.unprivileged_userns_clone` sysctl 的内核（如 NixOS 主线）默认即允许非特权 userns，无需任何 sysctl 调整。

## 2. 运行时二进制依赖

### run 路径（会话本体）

| 依赖 | 用途 | 首选 | 回退 | 备注 |
|---|---|---|---|---|
| pasta | netns 的用户态网关（tap + NAT，出口钉 `-i <if>`；选型论证见 ADR 0007） | 发行版包（Debian 12+/Ubuntu 23.04+/NixOS `passt`）；**上游静态构建** [passt.top/builds/latest/x86_64/](https://passt.top/builds/latest/x86_64/) 单文件可直接放 jammy | slirp4netns（jammy universe 1.0.1-2；deb 依赖 libslirp/glib/seccomp/libcap，较重） | provider trait 抽象（D5）；attach 模式 `pasta [PID]` 配合我们的无状态 netns，无需命名 netns |
| claude | 宿主管理（D3），PATH 透传 | — | — | 非本项目依赖 |

### verify 探针路径（T5，全部可 SKIP 降级，不进 run 路径）

`curl`（HTTP3 支持与否决定 P4 跑或 SKIP）、`dig`/`drill`/`resolvectl`（P3）、`jq`、`node`（Intl 视线，cc 宿主安装自带）、coreutils `find/stat`、宿主侧 `ip`。doctor 对缺失项报 SKIP 清单而非失败。

## 3. 明确的非依赖（设计的"薄"证明）

不需要：`bubblewrap`、util-linux `unshare`/`mount`（直接 syscall）、`ip netns`（无命名 netns）、`iptables`/`nftables`（pasta 用户态 NAT）、`docker`/`podman`、`sudo`/root、`newuidmap`/`newgidmap`（只映射自身 uid，内核直写 `/proc/self/uid_map`）、`slirp4netns`（若用 pasta）、AppArmor profile（Ubuntu 22.04 及多数发行版；24.04 类 P3 处理）。

## 4. 磁盘写入面（全部用户态、全部在声明路径内）

| 路径 | 内容 | 时机 |
|---|---|---|
| `~/.local/state/iso-cc/<profile>/` | per-profile 存储（claude 重定向目标、resolv.conf、后续 orca 扩列） | run 时按需创建 |
| config TOML | 用户/项目自管（`~/.config/iso-cc/`、`.iso-cc.toml`），工具只读 | — |

挂载点缺失时的预创建（`~/.claude` 等）是唯一例外，doctor 报告（N3 有界例外①）。

## 5. 本机实证记录（2026-09-26，NixOS WSL2 / 6.6）

```
$ unshare -Urn sh -c 'echo USERNS_NET_OK'        → USERNS_NET_OK
$ unshare -Urm sh -c 'mount --make-rprivate / &&
   mount --bind /etc/hostname /etc/hostname'      → MOUNT_BIND_OK
$ pasta --version                                 → 未安装（T0 devShell 补 nixpkgs#passt）
```

## 6. 构建期依赖（不进运行时）

`flake.nix` devShell：rust stable + `x86_64-unknown-linux-musl` target、clippy、rustfmt、`passt`、`slirp4netns`（T6 测试用）、`curl`（HTTP3 构建可选）。CI：nix build + clippy + test；GitHub Actions ubuntu-latest 允许非特权 userns（T5 可 CI 重放）。

## 7. 各工单增量依赖

| 工单 | 新增运行时依赖 |
|---|---|
| T0/T1 | 无（构建期除外） |
| T2 | 无（§1 内核特性而已） |
| T3 | + pasta 或 slirp4netns |
| T4 | 无（state 目录按需创建） |
| T5 | 探针工具（可 SKIP） |
| T6 | 无新（第二 provider） |
| T7 | 无 |
| P3 | AppArmor profile（24.04 类）、Xvfb/orca（R12 场景）、`--publish` 无新依赖（pasta 原生 `-t/-u`） |
