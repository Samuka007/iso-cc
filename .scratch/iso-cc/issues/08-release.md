# 08 — 版本化与多发行版打包分发（registry 除外）

**What to build:** 自动化版本化 + 多发行版软件包。设计蓝本 = podman 模式（会话研究结论）：**核心单二进制 + 外部 helper 二进制按发行版生态协调**。

**Blocked by:** 无　**Owner:** lane-package-release（完成）
**Status:** done（2026-09-27，parent 亲验通过）

## Answer（PM 落）

- 六锚全绿 + parent 复验：nextest 141/141；deb 元数据经 nixpkgs#dpkg 核验（Version 0.1.0-1；Depends passt >= 0.0~git20240220.1e6f92b【passt.top 官方 noble 静态 deb 同版，10 flag 逐一比对】/ slirp4netns >= 1.2.0【使用面仅 -c】；Recommends tun2proxy）；bundle 解包稀疏 PATH + ISO_CC_HELPER_DIR=libexec e2e rc=0（helper 解析链 manifest>env>PATH 的 env 层独立承载，counterfactual 无 env fail-loud 不穿透）；`--version` 无 tag 回落 0.1.0（nix/crane 无 .git 构建不失败）
- 版本注入实测：tag 副本 v0.1.0-1-g<hash>、-dirty 变体、tag/Cargo 不一致 panic rc=101（fail-loud）
- workflow actionlint 1.7.12 零告警（act 本地不可跑，三 job 逐命令本地复现留档）；PKGBUILD bash -n OK（namcap 不在 nixpkgs，人工清单回退）
- parent 复跑注记：NixOS 无 dpkg-deb（改用 nix dpkg）；稀疏 PATH 下 payload 需绝对路径（/usr/bin 无 true）——非缺陷，bundle 用户文档应注明

## 设计（来源：podman/passt 集成研究 + 本仓架构）

1. **版本化单一事实源**：git tag（`v*`）为准；`build.rs` 注入 `git describe --tags --dirty` → `iso-cc --version` 显示完整版本；Cargo.toml 版本随 tag 同步（脚本校验一致性，不一致 fail-loud）
2. **构建**：flake 已有 `x86_64-unknown-linux-musl` target → 静态单文件（D5：跨发行版单文件），CI nix 环境构建
3. **两分发形态**（照搬 podman：依赖下放发行版 or 捆绑）：
   - **发行版原生包**：`Depends: passt, slirp4netns`（版本下限）；`Recommends: tun2proxy`（socks 形态增强）。工具 = **nfpm**（单 YAML → deb/rpm/apk/arch；nixpkgs#nfpm）
   - **bundle 模式**（tar.gz）：iso-cc 静态二进制 + 上游静态 `pasta`/`slirp4netns`/`tun2proxy` 置于 `libexec/` 相对目录——解决 jammy 无 passt 等缺口（ADR 0007 既有预案）；票 14 manifest 钉路径已支持 libexec 定位，无需新机制
   - 路径解析优先级（对标 CONTAINERS_HELPER_BINARY_DIR）：manifest 钉路径 > `ISO_CC_HELPER_DIR` env > PATH（14 已实现前两者，补 env 缺口）
4. **CI/CD**：tag push → workflow：test → musl build → nfpm（deb/rpm/apk）→ PKGBUILD lint → sha256sum → GitHub Release 上传（签名 minisign 后置票）
5. **明确不做**：crates.io/npm 等 registry；APT/Pacman 源托管（后续票，工具已调研：aptly/artifactx/debanator/repo-add）

## Acceptance

- [ ] `iso-cc --version` 输出 tag+commits+dirty；与 git tag 不一致的 Cargo.toml → 构建 fail-loud
- [ ] nfpm 本地产出 deb+rpm+apk 三包；`dpkg-deb -c/-I` 抽验：二进制位、Depends 字段（passt slirp4netns 版本下限）、文件路径正确
- [ ] bundle tar.gz：三 helper 静态二进制 + iso-cc；解包后 `ISO_CC_HELPER_DIR=libexec` 下 run -- true e2e rc=0（无 PATH 依赖）
- [ ] PKGBUILD 模板（AUR 形态）lint 通过（namcap 或人工对照清单）
- [ ] release workflow YAML 就位（tag 触发矩阵）；`act`/dry-run 或人工推演留档
- [ ] clippy -D warnings 绿；nextest 全绿

## 轮子盘点

nfpm（单声明多格式，nixpkgs#nfpm）；nix-bundle-app/toDeb/toRpm（备选，QEMU debBuild 拒——重）；cargo-deb 拒（单 deb 格式）；上游静态 release（passt/slirp4netns 官方单文件，sha256 钉定）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 20 lane 文件集（probe/session/config/plan/doctor）；一次验证收尾
