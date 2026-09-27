# 23 — R12 浏览器驱动层探究：Playwright / browser-use 作为 orca 之外的驱动位

**What to build（探究后落地）:** 用户指认的典型用例：agent 驱动 Chrome 的自动化框架（browser-use，基于 Playwright/CDP；Playwright 本体 = Microsoft 自动化库）。R12 已选 orca serve（服务端模式，loopback SOCKS 钉定）为主路径；本票探究 Playwright 系作为**驱动层**的组合位。

**Blocked by:** 无（探究不依赖任何在途票）　**Owner:** 未定

## 要探究的问题

1. **驱动与浏览器的位置正交性**：Playwright/browser-use 驱动 Chrome 的通道是 CDP（websocket/TCP）。若 Chrome 跑在会话 netns 内（R12 身份一致的前提），宿主侧驱动连 CDP 需要**入站转发**（票 17 net.publish=auto 的 -t 转发或 pasta 原生镜像的反向）；若驱动也进会话（npm/npx playwright 在会话内跑），零转发但依赖会话内 Node。两形态对比
2. **与 R12 路径①（宿主显示 socket bind-mount）组合**：headed Chrome 在会话内渲染到宿主显示器（AF_UNIX bind 透传），Playwright CDP 走 netns 内 loopback——身份+渲染+驱动三点各就各位的可行性
3. **指纹面**：Playwright 默认注入自动化痕迹（navigator.webdriver 等）——与 R12 的指纹稳定要求（orca clean 模式）冲突度评估；browser-use 同题
4. **与 orca 的关系**：替代（同用途二选一）还是互补（orca 管 OAuth 账号态、Playwright 管纯自动化抓取）
5. **登录态持久**：Playwright 的 storageState 与 R4 重定向集（profiles/<p>/）的对接

## Acceptance（探究报告）

- [ ] 两形态（驱动进会话 / CDP 入站转发）各出原型证据（/tmp 脚本）
- [ ] 指纹冲突清单（对照 R12 验收：UA/canvas/WebGL/fonts）
- [ ] 择型建议 + 落地票草（工作量、文件集）

## 边界

- /tmp 脚本 + 报告，不改仓库代码；不 git
