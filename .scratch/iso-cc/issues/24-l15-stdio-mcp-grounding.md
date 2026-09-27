# 24 — grounding：L1.5 × stdio MCP 全链实测（@playwright/mcp 为样例负载）

**What to build:** 票 23/15 的验证缺口：host 模式下"cc（netns）→ L1.5 prefix → shim → exec.sock RPC → 宿主侧 spawn stdio MCP server → cc 持续调用其工具"全链**从未用真 MCP server 实测过**。本票 = 该链的逐跳 grounding + 缺口修复。

**Blocked by:** 无　**Owner:** 待派

## 背景（诚实状态）

- 附3 取证声称 L1.5 "实测覆盖 stdio MCP"——行为观测，机制未知（spawn 是否经 prefix shell / 还是 spawn 命令串拼 prefix，两种机制拦截点不同）
- 票 15 的实现验证用的是合成 marker 探针（one-shot 命令形态）；**常驻 stdio 进程形态从未实测**
- SCM_RIGHTS fd 直通设计上支持长生命周期 stdio，但 pipe 半途死亡、cc 重连语义、host 侧收割时机均为空白

## 实验矩阵（/tmp 脚本，@playwright/mcp 真负载）

1. **spawn 机制取证**：CLAUDE_CODE_SHELL_PREFIX 注入下，cc（替身驱动等价链）spawn MCP server 时的 argv 形态与落点（netns 侧 or 宿主侧）——strace/进程树留证
2. **常驻生命周期**：MCP server 宿主侧 spawn 后：stdio 双向持续可用时长 = 会话期？cc 调工具（initialize / tools/list / tools/call 开关网页）多轮往返
3. **真实负载**：@playwright/mcp 真实起浏览器（headless 或 --cdp-endpoint 连宿主 Chrome，后者联动票 23）→ 会话内工具调用打开网页截图 → 成功即全链通
4. **失败语义**：MCP server 中途死亡（kill）→ cc 侧 pipe EOF 行为取证；会话结束时 worker 收割（13 双键）取证
5. **netns 对照组**：不注入 L1.5（engine=netns 默认 sandbox）→ MCP server 落 netns 内 → playwright 驱动浏览器的浏览器流量走会话出口（这是身份一致形态，两模式语义差异写清）

## Acceptance

- [ ] spawn 机制 argv/落点留证；机制结论（经 prefix shell / 拼接 / 不走）
- [ ] 真实 @playwright/mcp 常驻链全绿：多轮工具调用往返、会话期存活、teardown 收割
- [ ] 失败语义两例（worker 死 / 会话 teardown）行为留档
- [ ] 若链路有缺口（one-shot 假设不成立等）→ 缺口清单 + 修复票草；不阻塞报告
- [ ] clippy/nextest 不回归（如有代码修复）

## 边界

- /tmp 脚本 + 报告；真 MCP server 经 npx 拉 @playwright/mcp（公开源，零隐私）；不 git；不改 20/23 在途面
