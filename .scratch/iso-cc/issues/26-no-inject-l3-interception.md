# 26 — 拦截去注入化：L3 bind + L2 承担全部拦截，移除 CLAUDE_CODE_SHELL_PREFIX 注入

**What to build:** host 模式的拦截从「注入 L1.5 环境变量」改为「L3 mountns bind + 既有 L2」——cc 的会话 env 中不再出现任何点名 hook 的变量。动因（操作者 2026-09-28）：`CLAUDE_CODE_SHELL_PREFIX` 可被 cc 自身与任意子进程观测（env 是公开面），注入即自报拦截行为。

**Blocked by:** 无　**Owner:** 邻居 agent（w12:p1，实现完成）
**Status:** done（2026-09-28，parent 亲验；第 6 条 E1 重放待操作者放行 10001 实验场）

## Answer（PM 落）

- **验收 5/6 绿**（env 断言 / L3 stat 取证 / Bash+/bin/sh 双形态 / sandbox 回归 / clippy+nextest）+ parent 复跑：L3_exit7=7、L3_sh_exit9=9（双形态退出码直采）；netns 内 stat /bin/bash=76、/bin/sh=62（shim 符号链接尺寸）✓；nextest 142/142；clippy -D warnings 零告警
- **操作者追加裁定三条（正本更新）**：①prefix/SHELL 值面 = 经典路径 /bin/bash（bind 后即 shim，cc 风控不可辨识）；②L2 全量种子撤（npx 类不可穷尽，stdio MCP 拦截改由值面经典路径承担——/bin/bash=shim 仍经单载荷转发宿主，E1 语义不变）；③PROXY 族 env scrub 防出口泄露
- **parent 修复（L3 面回归）**：bind 列表无条件含 /usr/bin/bash → NixOS ENOENT → mountns 装配崩（-- /bin/true 复现）。修 = 存在性过滤 + skip note（对齐票 03 语义）；缺席路径天然不可执行，无拦截损失
- **第 6 条（E1 重放）**：需 10001 重建实验场（node + claude 二进制 + 鉴权材料跨机）——等操作者放行

## 机制依据（票 24 源码级取证，字节偏移在 REPORT.md）

- stdio MCP spawn：cc 经 SDK StdioClientTransport **shell:false 直接 execvp 裸名 `npx`**，不经任何 shell → **L2 PATH 前置即可拦**（<sess>/bin/npx = shim）
- Bash 工具：$SHELL（L1）或硬编码 /bin/bash、/usr/bin/bash（附3 取证字面量）→ **L3 bind 拦**
- hooks/statusline/REPL !cmd：硬编码 `/bin/sh -c` → **L3 bind /bin/sh 拦**
- 结论：L1.5 的全部覆盖面可由 L3 + 既有 L2/L1 等价承接，且 env 零新增

## Specification

1. **删**：host 模式注入的 `CLAUDE_CODE_SHELL_PREFIX`（L1.5）——会话 env 中不得出现
2. **增 L3**：netns 内 mountns bind——`/bin/bash`、`/usr/bin/bash`、`/bin/sh` → `<sess>/bin/<name>`（multi-call shim 符号链接到 iso-cc；bind 源=会话 bin 目录，逐会话隔离；mountns 作用域，宿主零 diff）
3. **execstub 增 argv 直通模式**：argv0 basename ∉ {bash, sh}（如 npx）→ 不走 -c 解析，宿主侧按原 argv execvp（宿主 PATH 解析真 npx）——长生命周期语义与 24 E1/E2 已证的 SCM_RIGHTS 通道相同
4. **保留**：L1 `SHELL=<sess>/bin/bash`（自然形态，普通用户自定义 shell 常态）；L2 PATH 前置；ISO_CC_SESSION / ISO_CC_EXEC_SOCK env（无 hook 点名语义；可在后续票做 argv0 路径推导去 env 化，非本票范围）
5. **sandbox 模式**：不 bind、不前置 PATH、不动 SHELL——行为零变化（回归锚）

## Acceptance

- [ ] 会话 env 断言：`CLAUDE_CODE_SHELL_PREFIX` 不存在（host/sandbox 两模式）；`env` 输出无新增 hook 点名变量
- [ ] `/bin/bash` 在 netns 内 `readlink -f /proc/self/exe` 于 shim 调用时 = iso-cc（L3 生效取证）
- [ ] stdio MCP 全链不回归：真 @playwright/mcp 经 L2（裸名 npx）宿主侧落点 + 常驻多轮工具调用（24 E1 回放）
- [ ] Bash 工具（$SHELL 与硬编码 /bin/bash 双形态）、hooks、statusline 三面绿
- [ ] sandbox 模式回归：/bin/bash 为真 bash、PATH 无会话 bin 前置
- [ ] clippy -D warnings 绿；nextest 全绿

## 边界

- 不改 `.scratch/**`、`docs/**`；不 git；一次验证收尾；ISO_CC_* env 的路径推导去 env 化留后续票


---

# 附：交接底稿（/tmp/iso-cc-handoff-ticket26.md 迁入防丢，2026-09-28）
# Handoff：iso-cc 票 26 —— 拦截去注入化（L3 bind + L2）

写给：w12:p1（iso-cc 打包/发行面 agent），当前任务收尾后接本票实现。
日期：2026-09-28 · 交接人：w12:p2（主 tracker 维护者）

## 任务一句话

host 模式的 shell/MCP 拦截从「注入 CLAUDE_CODE_SHELL_PREFIX」改为「mountns bind /bin/bash、/usr/bin/bash、/bin/sh → 会话 shim + PATH 前置」——cc 会话 env 零新增 hook 点名变量。

## 你需要读的正本（按序，勿凭记忆）

1. `.scratch/iso-cc/issues/26-no-inject-l3-interception.md` —— 验收锚与 Specification（本票唯一事实源）
2. `docs/adr/0008-net-scope-declaration.md` 附 4 —— 决策与理由（L1.5 取消）
3. `/tmp/iso-cc-exp24/REPORT.md` —— 票 24 实测：stdio MCP spawn = execvp 裸名 npx（L2 可拦）；E1 全链形态；E5 = 你要修的 126 复现
4. `src/execstub.rs` 现状 —— 25 已修 -l 容忍与二跳 sock 注入，在此基础上加 argv 直通模式

## 实施要点（浓缩）

- 删：session env 注入 `CLAUDE_CODE_SHELL_PREFIX`（host 模式）
- 增：netns 内 mountns bind `/bin/bash`、`/usr/bin/bash`、`/bin/sh` → `<sess>/bin/<name>`（multi-call shim 符号链接；注意 bind 后 bootstrap 自身不得再调 /bin/sh——现 bootstrap 已无 shell 依赖，验证即可）
- 增：execstub argv 直通模式——argv0 basename ∉ {bash, sh} 时宿主侧按原 argv execvp（宿主 PATH 解析真 npx），长生命周期（SCM_RIGHTS 通道，24 E1/E2 已证）
- 保留：L1 SHELL、L2 PATH 前置、ISO_CC_SESSION / ISO_CC_EXEC_SOCK env（去 env 化留后续票）
- sandbox 模式零变化（回归锚）

## 建议技能（Skill tool）

- `skill://herdr` —— 你需要与我（w12:p2）协调分支/提交时序
- `skill://tdd` —— 红绿顺序：先写 env 断言与 L3 取证测试（红），再实现

## 验收（完整版见票 26，此为速查）

env 无 CLAUDE_CODE_SHELL_PREFIX ｜ L3 bind exe 取证 ｜ 真 @playwright/mcp 全链不回归 ｜ Bash/hooks/statusline 三面绿 ｜ sandbox 不回归 ｜ clippy+nextest 全绿

## 边界

`.scratch/**`、`docs/**` 只读（主 tracker 归 w12:p2）；不 git（parent 统一提交）；一次验证收尾。

## 交接背景（30 秒版）

iso-cc = rootless 声明式沙箱（双引擎 netns/mark，正本 docs/REQUIREMENTS.md）。host 模式下我们把 cc 的 shell/MCP 执行转发到宿主侧（exec.sock RPC，SCM_RIGHTS stdio 直通，长生命周期已实测）。现有拦截靠注入 `CLAUDE_CODE_SHELL_PREFIX`（L1.5）+ SHELL（L1）+ PATH（L2）——操作者裁定注入可观测、取消，改 bind 方案。真 @playwright/mcp 全链已在票 24 实测通过（你的前提是信任该证据，无需重跑阶段 1）。

## E1 重放收口（2026-09-28，验证下放 E1Lab subagent）

- 报告正本：`~/workspace/iso-cc-e1-lab/REPORT-E1-10001.md`、`REPORT-L3-cc.md`（机器 10001，iso-cc v0.1.0-4-gafb538a-dirty）
- 免 claude 全链（判别器级）：@playwright/mcp stdio 五次 JSON-RPC 往返全绿（initialize/tools/list 25/navigate/screenshot/close）；MCP server 落点 = 会话 netns（exec_sock_fds=0）；teardown 零残留
- L3 取证补全：cc 与 root 双侧三条 bind 全落地（inode 对账 64554:412600 四路径同一 = shim）；「cc 缺失」系三重测量污染（shell 探针被 shim 弹宿主 / `$` 锚失效 / sh bind 落 /usr/bin/dash），每种在 root 同样复现。无代码变更
- 已知限制 G-A：裸 `npx` 会话内 127（npm exec 内部 /bin/bash 命中 L3 shim → 宿主落点丢 npx 缓存 PATH）——经典路径设计必然推论，不修；真 claude 前缀链不受影响
- 活体 `claude -p` 驱动：**操作者裁定暂缓（2026-09-28）**，凭据四件套不跨机；实验场留存于 10001（node20/claude 2.1.263/chromium/mcp.json/e1-drive.sh 于 /home/cc/e1-lab/），后续放行即可续跑
