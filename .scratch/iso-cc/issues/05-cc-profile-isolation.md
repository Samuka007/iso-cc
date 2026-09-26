# 05 — CC 配置隔离（路径重定向）

**What to build:** mountns 声明式重定向 `~/.claude/` → profile 目录、`~/.claude.json(.backup)` → per-profile 文件；unset `CLAUDE_CONFIG_DIR`；挂载点预创建与 doctor 报告；写集探针（`find ~ -newer` ⊆ 声明集，白名单含 R4 登记例外）；双开测试；首登流程文档。

**Blocked by:** 03

**Status:** ready-for-agent

- [ ] cc 视角无感：`/status` 显示默认 `~/.claude`；落盘可验（登录后 profile 出 `.credentials.json`，宿主无新增）
- [ ] 双开互不干扰；宿主 `~/.claude*` 零 diff
- [ ] 写集断言绿
