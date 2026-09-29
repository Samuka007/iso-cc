# 27 — 一键安装脚本 + README 使用文档 + exe-relative libexec 解析层

**What to build:** 「解包到 /usr/local/bin 即可使用」的任意发行版快速安装路径：install.sh（装/卸）+ README 使用文档 + bundle 布局的零配置解析。

**Blocked by:** 无　**Owner:** lane-installer-readme（完成）
**Status:** done（2026-09-29，parent 亲验通过）

## Answer（PM 落）

- 五锚全绿 + parent 复验：nextest 146/146；install→run→uninstall 环路亲验（PREFIX 沙箱）；**exe-relative 层亲验**——稀疏 PATH（无 pasta 可见）+ 零 env 下 pasta 经 `<exe_dir>/../lib/iso-cc/libexec` 解析、会话 e2e rc=0；负例（libexec 移走）= 四层链全落空 fail-loud 文案
- **版本时序注记**：exe-relative 层随下一版 release 生效；对 v0.1.0 旧 release 资产跑 install.sh → 旧二进制无此层，需 ISO_CC_HELPER_DIR（旧行为）——installer 不背兼容责
- 发现并如实记录：发布资产名带 v 前缀（git-describe 产物），installer 双命名兼容，未动邻居域 bundle.sh

## Specification

1. **exe-relative 解析层**（provider/mod.rs）：`pinned_bin` 优先级链插入第三层——manifest 钉路径 > `ISO_CC_HELPER_DIR` > **`<exe_dir>/../lib/iso-cc/libexec`**（current_exe 解析符号链接）> PATH。布局对应安装目标 `/usr/local/bin/iso-cc` + `/usr/local/lib/iso-cc/libexec/{pasta,slirp4netns,tun2proxy}`；dev/nix 构建该层自然不存在，回落 PATH 无害。单测：临时目录伪造布局断言命中
2. **packaging/install.sh**（POSIX sh，curl-bash 风格）：
   - `install [VERSION]`：探测 arch（x86_64/aarch64 → amd64/arm64）→ 从 GitHub Release 拉 `iso-cc-bundle-<ver>-linux-<arch>.tar.gz` + `sha256sums.txt` 校验 → 解包 → `iso-cc` → /usr/local/bin、`libexec/` → /usr/local/lib/iso-cc/libexec → `iso-cc --version` 收尾自证
   - `uninstall`：删上述两路径 + 提示用户态清理（`iso-cc gc --all`）
   - 幂等（重复安装覆盖）；`PREFIX` 可覆盖（默认 /usr/local，供测试）；缺 curl/wget/tar 明确报错
3. **README.md 重写**：是什么（一段）｜安装（install.sh 一行 + 手动 + 发行版包 + nix flake）｜快速上手（最小 config + run + verify）｜双引擎与 egress 形态表｜exec.bash 双轴与 MCP 行为（含快照边界一句）｜生命周期（setup/doctor/list/gc）｜卸载｜文档指针（REQUIREMENTS/ADR/CHANGELOG）。现状节（T0 骨架）删除——已过时
4. **CHANGELOG**：补 0.1.0 的安装段

## Acceptance

- [ ] exe-relative 层单测（伪造布局）+ 优先级序断言（env 优先于 exe-relative）
- [ ] `PREFIX=/tmp/iso-cc-inst-test ./install.sh install <ver>` → `/tmp/iso-cc-inst-test/{bin,lib/iso-cc/libexec}` 就位 + `--version` 自证；`uninstall` 后路径清空
- [ ] 稀疏 PATH 下 bundle e2e 不回归（无 ISO_CC_HELPER_DIR，靠 exe-relative 命中 libexec）
- [ ] README 覆盖：安装/快速上手/双引擎表/卸载四节；无过时信息（T0 现状节删除）
- [ ] clippy -D warnings 绿；nextest 全绿

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 nfpm/release.yml（邻居域）；一次验证收尾
