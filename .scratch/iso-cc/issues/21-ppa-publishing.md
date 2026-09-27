# 21 — PPA 发布：Ubuntu Launchpad 源码包（binary-repack 路线）

**What to build:** 把 iso-cc 发布到用户 PPA 的完整产物链。PPA 只收**源码包**（Launchpad 独立构建器编译，**构建期无网络**）——我们的静态 musl 二进制走 **binary-repack** 路线：预编译二进制进 orig tar，`debian/rules` 空编译。

**Blocked by:** 08（已完成，musl 构建与 bundle 产物可复用）　**Owner:** 待派

## 研究结论（2026-09-27，一手来源：ubuntu.com/docs/launchpad + Launchpad builder 行为）

1. PPA **不接受二进制 .deb 上传**；只收 source package（`.dsc`+orig+debian tar），Launchpad 构建器出二进制
2. 构建器**无网络**：cargo 构建路线（vendor 进 orig）被拒——vendor 体积大 + jammy builder rustc 太老（clap 4.6/nix 0.31 需更新工具链）→ **binary-repack 是正解**：我们自己在 release 流里出 musl 静态二进制（单一事实源），orig tar 打包预编译产物，`override_dh_auto_build` 置空
3. 上传链 = GPG（Launchpad 注册+邮箱验证）→ `dpkg-buildpackage -S` → `debsign -k <key>` → `dput ppa:<user>/<ppa>`（全程人工凭据，CI 只能产**未签名源码包**，签名/上传需操作者或 secrets）
4. 版本式样：`<ver>-0ubuntu1~ppa<N>`（每 series 单调递增）；series = noble/jammy 各一份源码包（changelog series 字段）

## Specification

1. `packaging/debian/` 模板树（模板变量：version/series/maintainer）：
   - control：`Build-Depends: debhelper-compat (= 13)`（**精简**，无编译器）；二进制包 `Architecture: any`
   - **per-series Depends 差异**：noble → `Depends: passt, slirp4netns, ${misc:Depends}`；jammy → `Depends: slirp4netns, ${misc:Depends}`（jammy 无 passt 包；netns 引擎 fallback=slirp 仍全功能，doctor 已有 slirp DNS Warn）——票面裁决：不硬编码缺失依赖
   - rules：`override_dh_auto_build` 置空 + `override_dh_auto_install` install 静态二进制到 `/usr/bin/iso-cc` + manpage + examples
   - copyright：iso-cc License（MIT/Apache）+ 内嵌第三方声明
2. `packaging/ppa-build.sh`：组装源码包（musl 二进制 ← 08 的构建产物 + debian/ 树渲染 + changelog 注入版本/series/日期 → `dpkg-buildpackage -S -sa`）；本地工具缺口（NixOS 无 dpkg-deb/devscripts）用 `nix run nixpkgs#dpkg` / `nixpkgs#devscripts` 补（08 已证 nix dpkg 可用）
3. 验证阶梯（不依赖 Launchpad）：`dpkg-source -b`/`dpkg-buildpackage -S` 本地产出 .dsc → `lintian`（接受 embedded-binary Warn，零 Error 为目标）→ `dpkg -I`（nix dpkg）抽验 control 字段
4. release.yml 追加 job：tag 时产出**未签名**源码包 artifacts（noble/jammy 各一）；上传步骤以注释形态给出（需 GPG/secrets，操作者启用）
5. 操作者手册（README 章节）：Launchpad 账号/PPA 创建/GPG 注册验证/debsign+dput 一页流

## Acceptance

- [ ] 本地产出 noble/jammy 两份源码包（.dsc+orig+debian.tar），`dpkg-source` 校验通过
- [ ] jammy 包 Depends 无 passt；noble 包 Depends 含 passt+slirp4netns（dpkg -I 抽验）
- [ ] lintian 零 Error（embedded-binary 类 Warn 允许，清单化）
- [ ] changelog 版本 = `<cargo version>-0ubuntu1~ppa1` 且 series 正确
- [ ] release.yml job 就位（actionlint 干净）；操作者手册成文
- [ ] clippy -D warnings 绿；nextest 全绿

## 轮子盘点

dpkg-buildpackage/dpkg-source/debsign/dput = Debian 原生（nix devscripts 补本机）；nfpm 不适用（只出二进制包）；releaseTools.debBuild/QEMU 路线拒（重 + builder 隔离偏离 PPA 模型）。

## 边界

- 真上传需用户 GPG/Launchpad 凭据（wizard 事项）；不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾
