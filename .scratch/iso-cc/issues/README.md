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
| 20 mcp-loopback-landing | in-flight（McpLoopLand lane） | — |

> **egress 目标形态（2026-09-27 用户裁决）**：4090 wg0 假设作废。出口 = 用户提供的真实隧道接口 或 D4 SOCKS 择型；地理身份探针（P13）在无隧道机器上恒按实红，不阻塞任何票。

| 票 | 状态 | 说明 |
|---|---|---|
| 04 network-egress | done（由 09/12 吸收）| 宿主浏览器直开 dev server（--publish 面）与 net.localhost_forward 未做——待排新票 |
| 05 cc-profile-isolation | ready | 下一主力票（R4/R5：redirect 集与 CC 状态重定向实测） |
| 06 verify-matrix | in-progress | v0.2 子集绿（8 探针）；R12 子矩阵与 P16–P27 工具面待真实 cc 环境 |
| 07 slirp4netns-fallback | done（吸收进 09 selfmap 入口） | 4090 全矩阵待跑 |
| 08 release-packaging | in-flight（PackageRelease lane：nfpm deb/rpm/apk + bundle tar + 版本注入 + release workflow；podman 模式蓝本） | — |
| 16 socks-form-tun2socks | done（2026-09-27 亲验） | D4 裁决=组合成立零自研；首个全绿 verify（P13 geo=SG）；embedded 降级远期 |
| engine=mark 变体 | pending（触发条件未触发） | exec.bash=host 已解 US9 主诉求；cc 直spawn 进程的透明 localhost 残余缺口观察中 |

## 事件簿

- 2026-09-26 session.rs:218 单 token 损坏（lane 越界）；parent HEAD 恢复。
- 2026-09-26 票 15 lane 越权写票面 Answer（55 行）；内容经 PM 亲验反签保留；违规记档。
