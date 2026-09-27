# 17 — net.publish：出站入站面（auto 转发默认开）

**What to build:** US8/R12 入站面。改写自旧"--publish 显式声明"设想（2026-09-27 用户质疑后收窄：bash 工具起的 dev server 在 exec.bash=host 下本就宿主监听，零机制；pasta `-t all` 原生支持 auto 转发）。

**Blocked by:** 引擎裁决（spec 变更（五）/票 18）——**HOLD**：publish 是 netns 引擎的 localhost 桥接件；mark 引擎下 localhost 结构性成立，本票可能整体作废或仅剩显式 SPEC 面

## Specification

1. config `net.publish = "auto"（默认）| "off" | 显式 SPEC 列表`
   - auto：pasta `-t 127.0.0.1/all -u none`（仅宿主 loopback 绑定；只转发宿主未绑定端口，不劫持现有监听——passt(1) SPEC 语义实证）
   - off：`-t none -u none`（fail-closed 入站）
   - 显式：用户 SPEC 透传（如 `127.0.0.1/6768` 给 R12 orca serve 配对）
2. slirp 回退：其默认 auto 转发（>1024）与 auto 语义对齐——**票面要求 lane 以 slirp4netns README/man 一手取证**后落行为注释；off 时 slirp 用 `-r`? 需取证（不可关闭则 doctor Warn 说明差异，不许静默）
3. doctor：auto/off 一致性检查；宿主端口冲突取证（pasta 只绑未绑定端口，冲突=现有监听者胜 → 文案说明）
4. plan：展开 `-t/-u` 实参

## Acceptance

- [ ] auto：会话内 `python3 -m http.server 6768` → 宿主 `curl 127.0.0.1:6768` 200；宿主已占用端口不被劫持（对照取证）
- [ ] off：宿主无法达会话端口（fail-closed 入站）
- [ ] slirp 路径行为对齐 + 一手取证入档
- [ ] clippy -D warnings 绿；nextest 全绿；探针不回归

## 轮子盘点

pasta `-t all` / slirp 默认转发均现成；零新组件。拒绝内核端口重定向（R5/R6）。

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾
