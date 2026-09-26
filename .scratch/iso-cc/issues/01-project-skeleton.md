# 01 — 项目骨架（flake devShell + crate + CI）

**What to build:** `nix develop` 一条命令得到完整工具链（rust stable + musl target + clippy/rustfmt + pasta + slirp4netns + curl/dig/jq/node），`cargo build`/`cargo clippy -D warnings` 全绿，CI workflow 就位；CLI 子命令骨架（run/doctor/list/verify）可 `--help`。

**Blocked by:** None — can start immediately.

**Status:** done（2026-09-26）

- [x] `nix build .#checks.x86_64-linux.clippy` 绿
- [x] `nix build .#checks.x86_64-linux.nextest` 绿（--no-tests=pass）
- [x] `nix build .#packages.x86_64-linux.default` 产出静态二进制
- [x] devShell 内 `pasta --version` / `slirp4netns --version` 可执行（pasta 2026_07_16 / slirp4netns 1.3.5）
- [x] GitHub 仓库建立（private），CI workflow 触发成功（run 36243583777）

证据：rust 1.98.1（oxalica overlay，flake.lock 锁定）；依赖经 `cargo add` resolver 解析（anyhow 1.0.104 / clap 4.6.7+derive / serde 1.0.229+derive / toml 1.1.6 / thiserror 2.0.21）；`nix build` 四 checks 全绿；CI in_progress。
