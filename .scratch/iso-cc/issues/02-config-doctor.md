# 02 — config + doctor

**What to build:** TOML 声明（全局 + 项目覆盖）解析出 profile（egress/locale/net.scope/env/重定向列表）；`iso-cc doctor` 输出配置 vs 宿主现实的全量 diff（egress 接口+路由、zoneinfo、locale、provider 二进制、userns sysctl、claude 安装探测、必达域名可达性）；`--print-plan` 输出等价命令序列；`list` 扫 /proc 报告存活会话。配置非法 exit≠0 拒绝降级（srt refuseSettings 范本）。

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] 4090 实测 doctor 输出正确 diff；故意破坏项逐条报错
- [ ] 配置非法时 exit≠0，绝不静默回退
- [ ] cc 必达域名可达性检查（api.anthropic.com / OAuth 三件套 / registry.npmjs.org）
