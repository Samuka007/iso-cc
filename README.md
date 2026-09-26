# iso-cc

在任意 Linux 发行版上，以 rootless、声明式、零持久状态的方式，为 Claude Code（或任意命令）拉起「网络出口定向 + locale 覆盖 + CC 配置隔离」的轻量会话。

- 设计与需求基线：[docs/REQUIREMENTS.md](docs/REQUIREMENTS.md)
- 词汇表与不变式：[CONTEXT.md](CONTEXT.md)
- 依赖面：[docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)
- 决策记录：[docs/adr/](docs/adr/)

## 状态

规格阶段完成（T0 骨架）。工单在 `.scratch/iso-cc/issues/`，按阻塞边推进。

## 开发

```bash
nix develop          # rust stable + musl target + clippy/rustfmt + pasta + slirp4netns
cargo build
nix build .#checks.x86_64-linux.clippy   # -D warnings
```

> 仓库刻意不提交 `result*`（nix GC 根）与 `target/`；flake.lock 锁定 rust-overlay 版本，工具链版本只随锁更新演进，`nix collect-garbage -d` 定期清理旧闭包。
