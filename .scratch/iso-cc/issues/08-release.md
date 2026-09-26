# 08 — 发布（musl 静态产物 + 文档）

**What to build:** `nix build` 产出 musl 静态二进制；安装/使用文档（含隧道侧出 TUN 边界、localhost 语义分层、hostgw 别名指南）；cargo-deny 门禁（对齐 oh-my-pi deny.toml）；GitHub Release。

**Blocked by:** 06, 07

**Status:** ready-for-agent

- [ ] 两台宿主从二进制冷启动过 T5
- [ ] cargo-deny（licenses/advisories/bans）绿
