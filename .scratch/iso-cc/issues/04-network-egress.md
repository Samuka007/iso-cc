# 04 — 网络出口定向（pasta attach）

**What to build:** netns + pasta attach（egress 钉 `-i <if>`、`--map-host-loopback`、netns 内 resolv.conf bind、v6 默认关、auto 端口转发默认开）；会话内起 dev server 宿主 localhost 直达；宿主 loopback-only 服务（MCP/IDE）经网关地址可达；`net.localhost_forward` 声明端口预占 relay；`net.private` 路由策略（默认 tunnel）。

**Blocked by:** 03

**Status:** ready-for-agent

- [ ] 会话内 `curl -4 ifconfig.me` == egress 出口；宿主不变
- [ ] 宿主浏览器直开会话内 dev server（localhost 同端口）
- [ ] 会话内经网关地址访问宿主 loopback-only 服务
- [ ] 拔 wg0：全部挂起/失败，绝不回落宿主直连
