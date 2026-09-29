# iso-cc 票索引（PM 维护；票面 Answer 由 PM 独家更新）

正本口令原则：票/spec = 正本；消息只携带票指针 + 执行所有权。spec：`../spec.md`（变更记录一/二/三/四）。设计稿：`../design-session-lanes.md`。审计材料：`../audit-code-facts.md`。

## Phase I 完成态（2026-09-26，HEAD 5ff406a）

| 票 | 状态 | 亲验证据锚 |
|---|---|---|
| 09 provider-pasta-primary | done | root=pasta banner、exit7→7、print-plan `-I`、if:lo/nonexistent0 拒绝 |
| 10 pasta-attach-tunsetiff | done | works（`-I` 硬规则，passt 命名默认坑） |
| 11 ns-safe-api | done | session.rs unsafe=0（26 SAFETY 收敛 ns.rs）、双路径透传 |
| 12 no-shell-netlink | done | strace execve 无 ip/sysctl、netlink 就绪等待、PATH 最小化双 provider |
| 13 lifecycle-3layer | done | kill -9 pasta→树灭、收编日志、假孤儿→doctor FAIL |
| 14 setup-gc-manifest | done | setup×2 diff=0、清单门拒未 setup profile、gc 终态一致、unsafe 归位 |
| 15 bash-stub-exec-rpc | done | 判别亲验：host PID1=systemd 无 tap / sandbox PID1=bash 有 tap、rc=7 经通道 |

## 剩余工作

| 19 mcp-transparent-loopback | 探究 done（2026-09-27：pasta 原生镜像 = 零组件透明；择型 c≫a>b；快照边界入档） | — |
| 20 mcp-loopback-landing | ready（薄落地：P-MCP 探针 + 快照缺口 socat 兜底 + 冲突 Warn；估 2 人日内） | — |
| 18B mark-engine-impl | done（2026-09-27，parent 亲验：mark 全链 + 集成 bug 修复 + 128 测试） | — |
| 20 mcp-loopback-landing | done（2026-09-27 亲验）→ **兜底部分由 22 删除**（用户裁决：时序错位用重启解决，不打补丁） | — |
| 22 remove-mcp-fallback | ready（删 net.mcp_fallback socat 兜底；P-MCP 探针保留改两态） | 21 |

> **egress 目标形态（2026-09-27 用户裁决）**：4090 wg0 假设作废。出口 = 用户提供的真实隧道接口 或 D4 SOCKS 择型；地理身份探针（P13）在无隧道机器上恒按实红，不阻塞任何票。

| 票 | 状态 | 说明 |
|---|---|---|
| 04 network-egress | done（由 09/12 吸收）| 宿主浏览器直开 dev server（--publish 面）与 net.localhost_forward 未做——待排新票 |
| 05 cc-profile-isolation | ready | 下一主力票（R4/R5：redirect 集与 CC 状态重定向实测） |
| 06 verify-matrix | in-progress | v0.2 子集绿（8 探针）；R12 子矩阵与 P16–P27 工具面待真实 cc 环境 |
| 07 slirp4netns-fallback | done（吸收进 09 selfmap 入口） | 4090 全矩阵待跑 |
| 08 release-packaging | done（2026-09-27，parent 亲验：deb/rpm/apk + bundle e2e + 版本注入 fail-loud；源托管/签名后置） | — |
| 16 socks-form-tun2socks | done（2026-09-27 亲验） | D4 裁决=组合成立零自研；首个全绿 verify（P13 geo=SG）；embedded 降级远期 |
| engine=mark 变体 | pending（触发条件未触发） | exec.bash=host 已解 US9 主诉求；cc 直spawn 进程的透明 localhost 残余缺口观察中 |

## 事件簿

- 2026-09-26 session.rs:218 单 token 损坏（lane 越界）；parent HEAD 恢复。
- 2026-09-26 票 15 lane 越权写票面 Answer（55 行）；内容经 PM 亲验反签保留；违规记档。

| 21 ppa-publishing | in-flight（binary-repack 源码包路线：deb 树+组装脚本+lintian 阶梯；上传需用户 GPG/Launchpad 凭据） | — |

| 21 execrpc-nix-sandbox-test | done（2026-09-27：host_shell 解析器 + 沙箱守卫，nix build 全绿亲验） | — |
| 22 remove-mcp-fallback | done（2026-09-27 亲验：兜底删除、P-MCP 两态化、BREAKING 进 CHANGELOG） | — |
| 23 playwright-driver-exploration | 探究待派（已按用户收窄：仅『会话内 Playwright 驱动宿主已登录 Chrome』用例，1235 删） | — |
| 26 no-inject-l3-interception | ready——已 handoff 邻居实现（/tmp/iso-cc-handoff-ticket26.md；ADR0008 附4 已记） | 15/25✅ |

## 邻位协作（herdr 双 agent，w12 workspace）——协议已接受并执行

- 邻居已完成：PpaPublish 死 lane kill + 死件归档（/tmp/ppa-archived/）+ 主 checkout 清空；worktree /home/nixos/workspace/iso-cc-packaging（分支 packaging/ppa，基于我方 HEAD）建立；nfpm 降级已随迁（ef07e97）；AptlinePub 已通知迁址；aptline 独立仓库 /home/nixos/workspace/aptline 不受影响
- 我方处置：其遗留撞号工单 21-ppa-publishing.md → 归档 issues-ppa/P1-ppa-publishing.md（P 命名空间）
- 状态：邻居继续在 worktree 做 R2 APT registry + PPA（等待操作者凭据四件套：R2 token/GPG/域名/secrets——wizard 项）
- 双向回执闭环（2026-09-27）：协议双方确认，零异议。nfpm 两版冲突裁决 = 降级系 PM 于 08 验证后执行（票 08 已加后记澄清时序），操作者可否决回滚 ef07e97

- w12:p1 = 邻居 agent：打包/发行面（packaging/**、.github/**、PPA/debian 源码包，自称工单 21 撞号已要求改 P 前缀）；**要求其建 linked worktree（branch packaging/ppa）迁移其未提交改动后在该 worktree 工作**
- w12:p2 = 本 agent：src/**、flake.nix、.scratch/**（主 tracker 01-23）；已提交独占集（flake .c 过滤 + execrpc 沙箱修复 + 正本）
- nfpm.yaml 归邻居，但保留用户裁决的依赖降级（Depends 仅 passt；tun2proxy→Recommends；slirp4netns→Suggests）
- 提案已投递待回执；回执后本表加邻位产出一行

| 24 L15-stdio-MCP-grounding | ready（L1.5×stdio MCP 常驻链从未实测；@playwright/mcp 真负载逐跳 grounding） | — |

| 27 installer-readme-exe-relative | done（2026-09-29 亲验：install/uninstall 环路 + exe-relative 层稀疏 PATH e2e + README 重写 146 测试） | — |

## 收尾态（2026-09-27，二次收口）

**全部实现票完成**（01-05、08-25 done；探针 141+2、双引擎、三形态出口）。剩余需外部输入：
- **06 重估**：本机已有真 claude -p（票 24/E1 实证）——探针矩阵可部分推进，不再完全阻塞
- 邻居 R2 registry：等操作者凭据四件套（R2 token/GPG/域名/secrets）
- 裁决项：默认引擎、minisign 签名、nixpkgs 上游 PR、mihomo 持久化
- **06**：真实 cc 探针扩充（R12 子矩阵、P16-P27 工具面）——需真实 cc 环境/账号
- **真机验证项**：真实 cc 登录凭据落点、/status 视线（票 05）、L3/L4 拦截层（票 15 边界）
- **07 残余**：跨发行版传输语义（降级待办，等真实多环境）
- **后续决策**：minisign 签名、APT/Pacman 源托管（aptly/artifactx/debanator 已调研）、nixpkgs 上游 PR
