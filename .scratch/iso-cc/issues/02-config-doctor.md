# 02 — config + doctor

**What to build:** TOML 声明（全局 + 项目覆盖）解析出 profile（egress/locale/net.scope/env/重定向列表）；`iso-cc doctor` 输出配置 vs 宿主现实的全量 diff（egress 接口+路由、zoneinfo、locale、provider 二进制、userns sysctl、claude 安装探测、必达域名可达性）；`--print-plan` 输出等价命令序列；`list` 扫 /proc 报告存活会话。配置非法 exit≠0 拒绝降级（srt refuseSettings 范本）。

**Blocked by:** 01

**Status:** in-progress（本机全绿；4090 实测待跑）

- [x] 本机：doctor 14 checks（egress/iface FAIL=本机无 wg0，fail-loud 正确；provider/agent/域名可达/重定向挂载点全 OK）；`--json` 机器可读
- [x] `--print-plan`：scope/ipv6/egress/locale/bind/逐项输出（`run --print-plan -- claude`）
- [x] 配置非法时 exit≠0 拒绝降级（deny_unknown_fields + expect_err 测试）
- [x] cc 必达域名可达性检查（5 域名 TCP 443）
- [x] `list` /proc 扫描（ISO_CC_SESSION 标记，零持久状态）
- [ ] 4090 实测 doctor 输出（含 wg0 存在的正向用例）
- [ ] `--profile` 指定不存在 profile 的报错路径手动验证（单测已覆盖 resolve 语义）

**实现笔记**：依赖经 `cargo add` resolver 锁定（anyhow 1.0.104 / clap 4.6.7 / serde 1.0.229 / toml 1.1.6 / thiserror 2.0.21 / serde_json / tempfile-dev）；T2 注意：NixOS zoneinfo 路径非 /usr/share/zoneinfo（doctor 已实测 FAIL），需路径发现逻辑。
