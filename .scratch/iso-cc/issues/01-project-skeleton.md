# 01 — 项目骨架（flake devShell + crate + CI）

**What to build:** `nix develop` 一条命令得到完整工具链（rust stable + musl target + clippy/rustfmt + pasta + slirp4netns + curl/dig/jq/node），`cargo build`/`cargo clippy -D warnings` 全绿，CI workflow 就位；CLI 子命令骨架（run/doctor/list/verify）可 `--help`。

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [ ] `nix build .#checks.x86_64-linux.clippy` 绿
- [ ] `nix build .#checks.x86_64-linux.nextest` 绿（--no-tests=pass）
- [ ] `nix build .#packages.x86_64-linux.default` 产出静态二进制
- [ ] devShell 内 `pasta --version` / `slirp4netns --version` 可执行
- [ ] GitHub 仓库建立（private），CI workflow 触发成功
