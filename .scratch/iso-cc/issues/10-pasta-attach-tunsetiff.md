# 10 — pasta attach TUNSETIFF 阻塞调查（本机 WSL2）

**What to build:** 根因定位本机 pasta attach 模式 `TUNSETIFF ioctl on /dev/net/tun failed: Invalid argument`；产出 proven working invocation（或"此环境不可用"裁决 + 证据 + 替代路径建议）。

**Blocked by:** 无　**Blocks:** 09 的 A2（pasta provider impl）
**Owner:** lane-pasta-attach（已完成）
**Status:** done（2026-09-26，parent 亲验通过）

## Answer（PM 落）

**裁决：works（此环境可用）。** 根因不是 WSL2/内核/userns/时序，而是 passt 的命名默认：
未给 `-I/--ns-ifname` 时，ns 内 tap 名默认取 outbound 接口名（conf.c:1953-1958 → tap.c:1535；
passt.1:654-657 明文）。`--outbound-if4 lo` → TUNSETIFF 请求建名为 `lo` 的 tap → 撞 netns 内必有
的 loopback → tun_set_iff()（tun.c:2711/2715/2720）attach-to-existing 分支返回 EINVAL。
LD_PRELOAD shim（ifr_name='lo', errno=22）+ strace（setns 成功、open 成功）+ 对照实验
（--outbound-if4 lo 必败 vs -I tap9 必成）+ podman discussion #22570 五重证据。

**Working invocation**（parent 复验：netns 内 tap 存在、默认路由、外联 HTTP 200、宿主 loopback
经网关地址可达）：
```
pasta --userns /proc/<pid>/ns/user --netns /proc/<pid>/ns/net \
      --config-net --dns-forward 10.0.2.3 -f
```
**硬规则（进 09 spec）**：凡 outbound 接口名会与目标 netns 内现有接口撞名（lo 必然；wg0/eth0
常见），必须显式 `-I tap0`；`--map-host-loopback` 用默认（网关地址→宿主 loopback），不要用
outbound=lo 达成。

完整报告：/tmp/iso-cc-issue10-report.md（132 行，行号级证据）。

## 现场证据（2026-09-26，NixOS WSL2，内核 6.18.33.2-microsoft-standard-WSL2）

- pasta = nixpkgs passt-2026_07_16.090d739；slirp4netns 1.3.5 同机可用
- 形态一（显式 ns 路径）：
  ```
  unshare -Urn sh -c 'ip link set lo up && sleep 15' &   # $CH
  pasta --userns /proc/$CH/ns/user --netns /proc/$CH/ns/net \
        --outbound-if4 lo --config-net --dns-forward 10.0.2.3 -f -q
  → TUNSETIFF ioctl on /dev/net/tun failed: Invalid argument
  → Failed to set up tap device in namespace
  ```
- 形态二（位置 PID attach）：`pasta -q -f --config-net --dns-forward 10.0.2.3 --outbound-if4 lo $CH` → 同错
- 同一 netns 上 slirp4netns attach 成功（tap 创建、探针全绿）→ 内核 tun 模块本身可用

## 待验假设（按优先级）

1. open("/dev/net/tun") 时序：slirp4netns 先 open 后 setns；pasta 若 join 目标 userns 之后才 open，节点权限/caps 语义可能变化 → `strace -f -e trace=openat,ioctl pasta ...` 确认时序与错误点
2. userns 内 /dev/net/tun 的设备节点访问控制（cgroup device ACL / devtmpfs 挂载语义）
3. TUNSETIFF 参数协商（IFF_TUN|IFF_NO_PI / vnet_hdr）在该内核路径的 EINVAL 分支
4. passt 上游已知 issue（检索 passt 源码 isolation.c/tap.c 时序 + GitHub issues）
5. 低概率：WSL2 内核差异（slirp 正常 → 基本排除"内核不支持"）

## Acceptance（全部满足才算完成）

- [ ] 根因结论有 strace/源码行号级证据
- [ ] proven working invocation：attach 后 netns 内 `ip -br addr` 出现 pasta 配置的 tap、默认路由存在、宿主 loopback 经网关地址可达；输出全贴
- [ ] 若判不可用：裁决 + 复现最小集 + 上游 issue/文档链接 + 建议路径（slirp 为主 / 自定义 netstack provider / passt 上游修复版本钉死）
- [ ] 报告落 `/tmp/iso-cc-issue10-report.md`（命令、输出、结论、建议 pasta flag 集），并按 outputSchema 回传结论

## 边界（prohibited）

- 不改 `.scratch/**`、`docs/**`、仓库代码；不 git 操作；一次性脚本一律放 /tmp
