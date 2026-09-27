# 23 — 探究：会话内 Playwright 驱动宿主已登录的 Chrome 实例

**What to build（探究后落地）:** 用户用例：宿主 Chrome 以 `--remote-debugging-port=9222` 监听，会话内（netns）Playwright 通过 CDP 驱动这个**宿主已登录**的 Chrome 实例做自动化。

**Blocked by:** 无　**Owner:** 未定

## 机制预判（待原型证实）

- Chrome 调试端口在**会话 attach 前**已监听 → pasta 原生镜像（票 19 同型，loopback-only mihomo 7891 已证）→ 会话内 Playwright 直连 `127.0.0.1:9222`，零组件
- 调试端口在 **attach 后**才开 → 快照边界，`127.0.0.1` 不通 → endpoint 换网关地址 `172.27.0.1:9222`（map-host-loopback 动态生效）
- 身份语义：浏览流量走宿主 Chrome = 宿主身份（用户明确这不是登录/身份用例）

## Acceptance（探究报告）

- [ ] 原型：会话内 playwright `connectOverCDP` 打开宿主 Chrome 已登录站点并读取登录态（脱敏留证，/tmp 脚本）
- [ ] 两分支取证：attach 前监听（直连）与 attach 后开端口（网关地址切换）各自成立/失败形态
- [ ] 结论 + 落地票草（工作量；如需 iso-cc 侧配置/探针支持，列文件集）

## 边界

- /tmp 脚本 + 报告，不改仓库代码；不 git
