//! pasta spawn 模式（primary，设计稿 §1.1）：pasta 即会话根，bootstrap 由其在 ns 内拉起。

use std::ffi::{OsStr, OsString};
use std::net::Ipv4Addr;

/// spawn 模式 flag 集（proven invocation，设计稿 §1.1 + 工单 16 socks 分支取证）：
/// `pasta -f -q -l <log> --config-net [--outbound-if4 <if>] -I tap0 --dns-forward <dns>
///  --no-ndp --no-dhcpv6 --no-ra`。
///
/// - `-f` 必须显式传：无 `-f` 时 pasta fork 后台 + syslog（AltScan §9-OQ2），
///   iso-cc 的 wait 对象是 pasta 进程。
/// - `-I` 硬规则恒显式（issue 10 §Answer：默认命名取 outbound 接口名，必撞）。
/// - `egress`：`if:` 形态 = `Some(<iface>)`（显式钉定 outbound，形态语义本身）；
///   socks 形态 = `None`（outbound = 宿主默认路由接口，pasta 自动检测）。**socks 形态
///   禁止显式 outbound**：实测（/tmp/iso-cc-exp16/）显式 `--outbound-if4` 会使
///   `--map-host-loopback` 默认的「网关地址 → 宿主 loopback」映射失效（worker 经网关
///   地址拨代理必黑洞），自动检测形态正常。
/// - `-l`：pasta 自身诊断入 gateway.log（#4 tail 取证），stdio 保持 inherit
///   ——child（bootstrap/cc）与 pasta 共享 stdio（本机取证），会话 I/O 不得被日志劫持。
/// - 就绪 spec（票 12，§3.2）：`--config-net` self-config tap0（地址 + v4 默认路由），
///   bootstrap 以 netlink dump 断言就绪（netcfg::wait_ready，provider 无关）；
///   ready-fd 线被否——pasta 无 `-r`，引入即破坏 provider 无关性。
pub fn flag_args(egress: Option<&str>, dns: Ipv4Addr, log_file: &OsStr) -> Vec<OsString> {
    let mut v = vec![
        "-f".into(),
        "-q".into(),
        "-l".into(),
        log_file.to_os_string(),
        "--config-net".into(),
        "-I".into(),
        crate::provider::NS_IFNAME.into(),
        "--dns-forward".into(),
        dns.to_string().into(),
        "--no-ndp".into(),
        "--no-dhcpv6".into(),
        "--no-ra".into(),
    ];
    if let Some(iface) = egress {
        v.splice(5..5, ["--outbound-if4".into(), iface.into()]);
    }
    v
}
