# 16 — D4 SOCKS 形态择型实验：netns 内 tun2socks 组合消费宿主 mixed-port

**What to build:** 实验证明 + 择型落地：沙箱全部出口流量经宿主 SOCKS5（mihomo mixed-port，**已就绪**）离开。这是 D4 修订两候选（pasta+tun2proxy 组合 vs embedded netstack 自研）的**实测裁决**——组合若成立则零自研。

**Blocked by:** 无　**Owner:** lane-socks-form（完成）
**Status:** done（2026-09-27，parent 亲验通过；**D4 择型裁决 = pasta+tun2proxy 组合成立，embedded 降级远期**）

## Answer（PM 落）

- **首个全绿 verify**：socks profile 下 8 探针 **7 pass / 0 fail / 1 skip**，P1 出口=SG、P13 geo=Asia/Singapore==声明、P2/P15/P3b 全过，exit 0（parent 复跑确认）
- e2e 亲验：`run -- curl ifconfig.me` → 67.159.48.149 + `{"countryCode":"SG","timezone":"Asia/Singapore"}`；worker=tun2proxy(pid=2, pidns 内) → tun1 → socks5://172.27.0.1:7891 → mihomo
- fail-closed：坏端口（controller 非 socks5）→ 隧道超时、宿主 IP 不出现；死端口 → spawn 前 fail-loud「R8：绝不回落」
- 两条实施期发现（已固化注释+测试）：①pasta 显式 `--outbound-if4` 使 map-host-loopback 失效（socks 形态省略该 flag）；②tun2proxy halving 路由用 wait_tun_ready 非 wait_ready
- 择型：组合零自研成立（tun2proxy 0.8.3 via nixpkgs，sha256 在案）；embedded netstack 降级远期
- 隐私：订阅 URL/凭据仓库零命中（grep 亲验）

## 已就绪的实验环境（parent 已建，勿重建勿杀）

- 宿主 SOCKS5/HTTP：`127.0.0.1:7891`（服务 proc 名 `mihomo-sg-tmp`，**存活中，勿动**；controller 9091）
- 出口已验证：`curl -x http://127.0.0.1:7891 ip-api.com` → `67.159.48.146 = SG/Singapore/Asia-Singapore`
- 配置/订阅文件：`/tmp/tunnel-provider.yaml` 与 `/tmp/mihomo-tmp/`（**隐私物：不得复制进仓库**，票面/代码一律不出现订阅 URL）
- pasta 网关地址 → 宿主 loopback 已证（issue 10 round-trip）

## 实验目标（阶段 A，/tmp 一次性脚本）

`unshare -Urn` + pasta attach（proven invocation 形态，issue 10）→ netns 内 `tun2proxy --tun tun1 --proxy socks5://<gw-addr>:7891` → netns 内 `curl ifconfig.me` == `67.159.48.146`（SG）。验证点：
1. tun2proxy 能否经 tap0 默认路由到达宿主 loopback 的代理（proxy 地址路由例外是否自动）
2. DNS 走向：socks5 域名远端解析是否生效（curl 域名而非 IP 即测得）
3. `--daemon` 或前台+后台化的存活形态；退出清理（tun1/路由）
4. `nix build nixpkgs#tun2proxy` 可用性；不可用则下载上游静态 release（记 sha256）

## 落地目标（阶段 B，实验成立后）

- config：`net.egress` 扩第二形态 `socks5://<host>:<port>`（与 `if:<name>` 并列，D4 修订授权）；默认仍 `if:` 
- bootstrap：socks 形态时 spawn tun2proxy（marked 子进程，ISO_CC_SESSION 标记）→ netlink 等 tun1 就绪 → exec cc；worker 存活=会话期，死=协议黑洞（fail-closed 同构 R8）
- doctor：socks 形态检查 = 宿主端口可达 + 出口 geo 与声明 locale 对照（P13 自然转绿的条件）
- verify：P1/P13/P15 走 tun1 → 应全绿（SG）

## Acceptance

- [ ] 阶段 A：netns 内 curl 出口 == SG IP，全程命令+输出留档 /tmp/iso-cc-exp16/
- [ ] 阶段 B：`iso-cc run --config <socks 配置> -- curl -s ifconfig.me` == SG IP；verify 套件 P1/P2/P13/P15 全绿（P13 geo=SG）
- [ ] fail-closed：杀 mihomo → 会话内 curl 失败（绝不回落直连）
- [ ] 择型结论：组合 vs embedded 的裁决段落（若组合成立，embedded 降级为远期）
- [ ] clippy -D warnings 绿；nextest 全绿；既有探针不回归

## 边界

- 订阅 URL/节点凭据不入仓库（/tmp 之外零复制）；`.scratch/**`、`docs/**` 只读；不 git
