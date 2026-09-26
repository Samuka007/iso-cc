# 07 — slirp4netns 回退 provider

**What to build:** provider trait 第二实现（slirp4netns），Ubuntu 22.04（无 passt 包）全流程可用；`net.private` 路由策略落地（默认 tunnel / 可选 host）。

**Blocked by:** 04

**Status:** ready-for-agent

- [ ] 4090（universe slirp4netns 1.0.1）过 T5 全矩阵
- [ ] provider 切换仅改 config，不改调用方
