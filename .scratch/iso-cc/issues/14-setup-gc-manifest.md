# 14 — setup/gc 清单：两级资源模型落地 + provider 钉路径 + unsafe 归位

**What to build:** 按设计稿 §5 + spec 变更（一）落地（正本 = `../design-session-lanes.md` §5/§6-#2/§8-14 行）。

**Blocked by:** 13（已完成）　**Owner:** lane-setup-gc（完成）
**Status:** done（2026-09-26，parent 亲验通过）

## Answer（PM 落）

- parent 复跑：setup 首跑 action diff=1 → 二跑 **diff=0**（幂等）；钉路径 `run -- true` rc=0（root=pasta）；**未 setup 的 slirp profile 被正确拒绝**（"清单无 slirp4netns 条目（fail-loud #2）——先运行 setup"= 清单门契约行为）；`gc --all` 终态清单与现实一致（残留=0）；session.rs unsafe=0
- nextest 56/56；clippy 干净
- 设计裁决入档：①mountpoint 只登记 setup 创建路径（永不误删用户文件；key=<profile>:<dst>）；②run 期预创建整体切 fail-loud（变更（一）会话零持久物），locale binds 保持 session-scoped best-effort（NixOS /etc 语义不变）；③gc --all 守卫 = size≠0 ∨ 目录非空 ∨ mtime>registered_at → --force 才过；④provider 版本取 --version 首个非空行（pasta license 尾块噪声规避）
- 探针澄清：P13 geo 红为网络地理伪影（本机出口 IP 被 ip-api 定位 Perth）；声明 tz 与 egress geo 匹配时套件 7 pass/0 fail

## Specification

1. **manifest**：`~/.local/state/iso-cc/manifest.json`（user 属主；serde_json + tempfile 原子写；`schema` 字段版本化 fail-loud，nix profile 同构三机制）；entries = `[{kind: provider|mountpoint|profile-state, key, path?, version?, registered_at, reason}]`
2. **setup**（`iso-cc setup [--profile] [--json]`）幂等收敛循环：①provider 收敛（which + `--version` → 绝对路径+版本 upsert；缺失报包名不代装）；②挂载点收敛（redirect 集内缺失 mkdir/touch + upsert；`/etc/timezone` bind 已弃——R2 视线不依赖）；③manifest 原子重建。重跑 action diff = 0
3. **doctor 双向校验**：forward（条目存在性、provider 可执行+版本一致——漂移 Warn/缺失 Fail，替换 09 期过渡态 #2）；reverse（13 的 sweep 检查组接入；stale 条目 Warn + prune 提示）
4. **gc**（`iso-cc gc [--prune] [--profile] [--all] [--yes]`）：默认 sweep 报告；`--prune` 清 stale 登记；`--all` 有活跃会话拒绝、mountpoint 条目 size/mtime 变化 = 用户数据化拒绝（除非 --force）、profile-state 仅随显式 --profile、**provider 二进制永不删除**；终态清单与现实一致
5. **run 消费清单**：provider 绝对路径自清单钉定（#2 过渡态切清单态）；清单缺失 = Fail 提示 setup
6. **unsafe 归位**：13 期落在 session.rs 的 3 个生命周期原语（prctl/kill/waitpid，:50-95 区段）迁移 ns.rs（或 ns 同级 lifecycle 原语模块），session.rs 恢复 unsafe=0 不变量

## Acceptance

- [ ] `iso-cc setup && iso-cc setup` 第二次 action diff = 0（输出留档）
- [ ] doctor 双向：删 provider 条目/改版本 → 对应 Fail/Warn；伪造孤儿 → sweep Fail（13 已证）
- [ ] `gc --all` 后清单与现实一致（残留=0）
- [ ] provider 钉路径生效：清单钉定后稀疏 PATH（含 provider store 目录、不含 ip/sysctl 所在目录）`run -- true` 双 provider rc=0（12 号票 A2 前提闭合）
- [ ] session.rs unsafe = 0（grep 证据）；clippy -D warnings 绿；nextest 全绿；探针 6/1/1 不回归

## 轮子盘点

清单先例 = nix profile manifest（CompScan #8 一手源码：版本字段 :126-148/原子写 :254/printDiff :921），只取模式不耦合 store；实现站 serde_json（已有）+ tempfile（cargo add 定版）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；不动 15 范围（exec RPC）；一次验证收尾
