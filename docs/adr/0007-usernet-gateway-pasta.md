# D7: 用户态网关选 pasta（类别必须，实例择优）

状态：已接受（2026-09-26）

## 决策

T3 起引入 pasta 作为唯一 usernet provider（slirp4netns 为 trait 第二实现，T6）。

## 论证

新 netns 只有 `lo`；rootless 下唯一的 netns↔宿主流量通路是用户态网关（veth + 宿主路由/NAT 需要 root 且产生持久规则，被 D2 拒绝）。因此问题不是"要不要网关"，而是"要哪个"——放弃网关类别等价于放弃 R1/R5/R6 之一。

选 pasta 而非 slirp4netns：出口钉死按接口名（`-i`，R7 契约 1:1）；性能（splice 路径，podman 5 默认）；安全暴露面最小且经 podman 全量用户检验（网关是方案里最暴露的组件）；`--map-host-loopback` 原生支撑宿主服务回访（R3）；上游静态单文件解决 jammy 无包问题；`-t/-u` 端口转发支撑未来 `--publish`。

## 代价与边界

多一个 provision 的二进制（doctor pin 版本）；用户态 TCP 栈吞吐天花板（对 claude 文本流量无感）；边缘协议语义由网关代实现。T2（locale-only）不依赖它。

## 已拒绝

veth+策略路由（root+漂移，D2）；env 代理（可绕过，R1）；隧道客户端 per-sandbox（N 实例，D4/R7）；自研用户态栈（重新发明 libslirp/passt 的十年）。

## 后记（2026-09-26）：embedded Rust 网关 = 第三实现候选

Rust 生态积木已核实齐全：`netstack-smoltcp`（TUN→smoltcp→Tokio 异步 TCP/UDP 流）+ `socket2`（`SO_BINDTODEVICE` 出口钉死）+ `tun` 设备。两条用途不同的 embedded 变体：

1. **接口形态网关**（替代 pasta，直连出站）：relay 层 500–1500 行，协议边角（PMTU/分片/half-close）所有权内移。触发条件：绝对单文件分发 / jammy 静态 pasta 供应链不可接受。
2. **SOCKS 形态网关（embedded tun2socks）**：TUN-in-netns + smoltcp 流还原 + tokio-socks pump（`CONNECT`/`UDP ASSOCIATE`），glue 约 300–500 行——**D4 修订后为 SOCKS 形态正解候选**：直接消费现有 mixed-port，pasta 不参与；出站 socket 须在宿主 netns 侧（setns 模式同 pasta，或双进程帧中继），否则 netns 内够不到宿主 127.0.0.1；DNS 显式拦 53 经 SOCKS 转发。`tun2proxy` 虽现成但面向 SOCKS 上游模型，本形态即"自己写它的 netns 内嵌版"。

fail-closed 注：netstack 进程 panic = tap 无 ACK = 会话断网，与 R8 同构。
