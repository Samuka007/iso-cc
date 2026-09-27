# 04 — 网络出口定向（pasta attach）

**What to build:** netns + pasta attach（egress 钉 `-i <if>`、`--map-host-loopback`、netns 内 resolv.conf bind、v6 默认关、auto 端口转发默认开）；会话内起 dev server 宿主 localhost 直达；宿主 loopback-only 服务（MCP/IDE）经网关地址可达；`net.localhost_forward` 声明端口预占 relay；`net.private` 路由策略（默认 tunnel）。

**Blocked by:** 03

**Status:** done（传输面由 09/12 吸收）；**egress 目标形态待定**：2026-09-27 用户裁决——4090 的 wg0 假设作废（用户确认不知其来源，不作真出口）。出口 = 用户未来提供的真实隧道接口 或 D4 SOCKS 择型（pasta+tun2proxy vs embedded）；本机验证一律 if:eth0（宿主物理出口）只证传输语义，不声称地理身份。

> **2026-09-26 实施重排**：发现 slirp 硬编码违反 ADR 0007（pasta 才是 primary）且 egress 接口纯装饰（fail-open）。实施拆入 **09**（provider 化 + fail-loud，A1 无阻塞）与 **10**（本机 pasta attach TUNSETIFF 调查，阻塞 09/A2）。本票 4090 验收沿用 09/A2 结论。

- [x] 会话内 `curl -4 ifconfig.me` = 103.155.37.8（本机宿主出口；4090 上应为 7891 出口 64.118.144.224——**需 embedded SOCKS provider 或隧道侧 TUN**，D4 修订）
- [ ] 宿主浏览器直开会话内 dev server（localhost 同端口）
- [ ] 会话内经网关地址访问宿主 loopback-only 服务
- [ ] 拔 wg0：全部挂起/失败，绝不回落宿主直连
