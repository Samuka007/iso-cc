# iso-cc 票索引（PM 维护；票面 Answer 由 PM 独家更新）

正本口令原则：票/spec = 正本；消息只携带票指针 + 执行所有权。spec：`../spec.md`（含变更记录一/二/三/四）。设计稿：`../design-session-lanes.md`（已返回）。

| 票 | 状态 | Owner | 阻塞 |
|---|---|---|---|
| 01 project-skeleton | done | — | — |
| 02 config+doctor | done（本机绿） | — | — |
| 03 minimal-session | done（2026-09-26 本机验收） | — | — |
| 04 network-egress | 重排：由 09/12 吸收（pasta primary / 就绪等待） | — | 09, 12 |
| 05 cc-profile-isolation | ready | — | 03 |
| 06 verify-matrix | in-progress（v0.2 子集本机绿） | — | — |
| 07 slirp4netns-fallback | 吸收进 09（slirp = provider#2，selfmap 入口 + `-c`） | — | 09 |
| 08 release | ready | — | 全部 |
| 09 provider-pasta-primary | ready（票面按设计稿 §1/§5 修正后开工；首个冒烟 = pasta 退出码透传取证） | 待派 | 设计稿已落盘 |
| 10 pasta-attach-tunsetiff | done（2026-09-26，works；`-I` 硬规则） | — | — |
| 11 ns-setup-safe-api | ready（scope 按设计稿 §2：双入口 mountns/selfmap） | 待派 | 09 |
| 12 no-shell-netlink-failloud | ready（scope 修正：两 provider self-config → 只剩就绪等待 + /proc/sys 直写） | 待派 | 11 |
| 13 lifecycle-three-layer-kill | ready（audit-code-facts §6 五缺口：PDEATHSIG 对账、subreaper 收编、killpg、daemon 孙、网关标记 env） | 待派 | 12 |
| 14 setup-gc-manifest | ready（两级清单：manifest/setup/doctor 双向/gc；nix profile 同构先例） | 待派 | 13 |
| 15 bash-stub-exec-rpc | ready（spec 变更（四）；机制正本 ADR 0008 附/附2/附3；L1.5 优先验证） | 待派 | 13 |

## Phase D 裁决（2026-09-26 完成）

1. **总裁决 = 薄编排成熟 CLI**：网络面零自研；自研收缩 = config/doctor/verify、就绪等待、进程树终止三层、清单/GC、locale 注入。无候选工具整体替代（对比表无一全绿）。
2. D2/D4/D5/D7 维持且证据增强；D4 择型时 pasta+tun2proxy 与 embedded 并列实测。
3. **设计稿裁决（design-session-lanes.md）= pasta spawn 转正**：R8 内核结构性执行（bootstrap PDEATHSIG 对账 pasta pid）、uid_map 竞态消灭、unsafe 面收缩（mountns 入口 + slirp 专用 selfmap 入口）、attach 模式降为 slirp 同构参考。

## Phase I 串行序

**09 → 11 → 12 → 13 → 14 → 15**；每 lane parent 亲验后放行。设计稿 §8 有每 lane 触碰文件清单（无交叉证明）与冒烟命令。票面若与设计稿冲突，派发时以设计稿为准修正（12 配网命令删除已确认）。

## 事件簿

- 2026-09-26 session.rs:218 单 token 损坏（某 lane 越界改仓库）；parent 从 HEAD 恢复，cargo check 绿。纪律：lane 不得改仓库文件（例外：lane 自己的 deliverable 文件，如 research/*.md、design-session-lanes.md）；发现损坏只报告不修复。
