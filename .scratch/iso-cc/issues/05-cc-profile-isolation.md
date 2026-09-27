# 05 — CC 配置隔离：默认内置重定向对 + 持久 profile-state + 写集探针

**What to build:** R4/D6 落地。机制件已全就位（redirect 通用面/setup 收敛/doctor 核验/manifest kind），本票补"cc 默认语义"与验收探针。

**Blocked by:** 14（已完成）　**Owner:** 待派

## Specification

1. **默认重定向对**（config 新轴 `agent.cc_isolation = true（默认）| false`）：为 true 时 session 侧自动前置 binds——
   - `~/.claude/` → `<state>/profiles/<profile>/claude/`
   - `~/.claude.json` → `<state>/profiles/<profile>/claude.json`（`.backup` 同法）
   - **持久路径**（`profiles/<profile>/`，非 `sessions/<id>/`——登录态/会话史跨会话存续）；clean-room 默认不继承宿主 `~/.claude`
2. **setup 收敛 profile-state**：上述 dst 由 setup mkdir/touch + manifest 登记 `profile-state` kind（gc 仅随显式 `--profile` 回收，14 已实现语义）；run 期缺失仍 fail-loud（14 契约不变）
3. `unset CLAUDE_CONFIG_DIR` 已在 session env（session.rs env_remove）——本票加探针断言
4. **写集探针（P8，probe.rs）**：会话内 `find $HOME -newer <marker>` 结果 ⊆ 声明重定向集 ∪ 白名单（`~/.vscode/extensions` 等 R4 登记例外）；白名单外出现即 Fail（R4 核心不变式）
5. **--print-plan**：内置对展开可见；doctor：profile-state 条目核验（存在性/登记一致性）

## Acceptance（替身验证；真机 cc 验证项单列）

- [ ] 替身 cc（脚本模拟写 `~/.claude/foo`、`~/.claude.json`）会话内写 → 落 `profiles/<profile>/`；宿主 `~/.claude*` mtime/内容零变化
- [ ] 跨会话持久：会话 1 写 → 会话 2 可见
- [ ] 双开模拟：宿主侧进程直写 `~/.claude` 与会话写入并发互不干扰；会话后宿主零 diff
- [ ] P8 探针：白名单内放行、白名单外 Fail（单测覆盖判定核）
- [ ] `agent.cc_isolation=false` 时行为与现状等价（无内置对）；CLAUDE_CONFIG_DIR 探针绿
- [ ] clippy -D warnings 绿；nextest 全绿；探针套件不回归

## 真机验证项（不阻塞本票，登记待用户机器）

- 真实 cc 登录 → `profiles/<p>/claude/.credentials.json` 出现、宿主无新增
- 真实 cc `/status` 显示默认 `~/.claude`

## 轮子盘点

无 SDK：cc 状态隔离 = mountns bind（已建机制）+ 声明集 + 探针；CLAUDE_CONFIG_DIR 被拒（ADR 0006：版本路由不可控）；overlayfs HOME 被拒（R3）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 06 探针矩阵其余项/08 release；一次验证收尾
