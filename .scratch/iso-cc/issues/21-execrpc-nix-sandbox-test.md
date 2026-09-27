# 21 — exec-rpc 测试的 /bin/bash 硬编码破坏 nix 沙箱构建（CI 红线）

**What to build:** 修复 18B 引入的测试环境依赖：execrpc 测试 spawn `/bin/bash`，nix 沙箱无该 FHS 路径 → `nix build .#packages.x86_64-linux.default`（含 nextest）失败，CI 必红。本地 cargo 构建不暴露（宿主 /bin/bash 存在）。

**Blocked by:** 无　**Owner:** MarkEngineImpl（退回原 lane，上下文在）

## Specification

1. execrpc 相关单测不再假设 `/bin/bash` 存在：测试 shell 路径参数化（env 注入或 `sh` 探测），沙箱内用 nix store bash 或跳过（skip 语义 fail-loud 注释说明）
2. 复现锚：`nix build -L .#packages.x86_64-linux.default` 必须全绿（当前 drv k25pxka 失败：`宿主侧 spawn /bin/bash 失败: No such file or directory`）

## Acceptance

- [ ] `nix build -L .#packages.x86_64-linux.default` 全绿（含 nextest）
- [ ] 本地 cargo nextest 全绿不回归
- [ ] clippy -D warnings 绿

## 边界

- 只动测试与测试支撑面（不改 execrpc 运行时语义）；不 git
