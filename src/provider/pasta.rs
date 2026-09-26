//! pasta spawn 模式（primary，设计稿 §1.1）：pasta 即会话根，bootstrap 由其在 ns 内拉起。

use std::ffi::{OsStr, OsString};
use std::net::Ipv4Addr;

/// spawn 模式 flag 集（proven invocation，设计稿 §1.1）：
/// `pasta -f -q -l <log> --config-net --outbound-if4 <if> -I tap0 --dns-forward <dns>
///  --no-ndp --no-dhcpv6 --no-ra`。
///
/// - `-f` 必须显式传：无 `-f` 时 pasta fork 后台 + syslog（AltScan §9-OQ2），
///   iso-cc 的 wait 对象是 pasta 进程。
/// - `-I` 硬规则恒显式（issue 10 §Answer：默认命名取 outbound 接口名，必撞）。
/// - `-l`：pasta 自身诊断入 gateway.log（#4 tail 取证），stdio 保持 inherit
///   ——child（bootstrap/cc）与 pasta 共享 stdio（本机取证），会话 I/O 不得被日志劫持。
pub fn flag_args(egress: &str, dns: Ipv4Addr, log_file: &OsStr) -> Vec<OsString> {
    vec![
        "-f".into(),
        "-q".into(),
        "-l".into(),
        log_file.to_os_string(),
        "--config-net".into(),
        "--outbound-if4".into(),
        egress.into(),
        "-I".into(),
        crate::provider::NS_IFNAME.into(),
        "--dns-forward".into(),
        dns.to_string().into(),
        "--no-ndp".into(),
        "--no-dhcpv6".into(),
        "--no-ra".into(),
    ]
}
