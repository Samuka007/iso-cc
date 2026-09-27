//! 网络配置原语（票 12，设计稿 §3）：无 unsafe、同步、零外部命令。
//!
//! - 就绪等待 = netlink dump 断言（RTM_GETLINK → RTM_GETROUTE，CompScan #7 裁决）。
//!   09 取证 #5：ns 内 /sys/class/net 呈宿主视图（sysfs netns-tag 伪影）→ ns 内断言
//!   必须走 netlink；sysfs 仅宿主侧合法（provider::host_iface_up，#3）。
//! - crate：netlink-sys 0.9 + netlink-packet-route 0.33（同 org 同日发版，MIT，
//!   同步栈零 tokio；§3.2 裁决，被否 rtnetlink/tokio 与 neli/多一层抽象）。
//! - 被否 slirp `-r/--ready-fd` 旁证线（§3.2）：破坏 provider 无关性，pasta 无 ready-fd。

use std::io;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use netlink_packet_core::{NetlinkMessage, NetlinkPayload, NLM_F_DUMP, NLM_F_REQUEST};
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkMessage};
use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteMessage};
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
use netlink_sys::{protocols::NETLINK_ROUTE, Socket, SocketAddr};

const NETLINK_HEADER_LEN: usize = 16;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// IPv6 关断（§3.3）：`/proc/sys/net/ipv6/conf/{all,default}/disable_ipv6` 直写。
/// `all` 即刻生效于现存接口（内核 addrconf_disable_change 逐接口传播并撤地址），
/// `default` 覆盖其后新建接口（slirp `-c` 晚于本写点建 tap 的时序）。任一失败
/// fail-loud（#7）：文案含路径 + errno（设计稿 §6 三要素）。
pub fn disable_ipv6() -> io::Result<()> {
    disable_ipv6_via(&[
        "/proc/sys/net/ipv6/conf/all/disable_ipv6",
        "/proc/sys/net/ipv6/conf/default/disable_ipv6",
    ])
}

fn disable_ipv6_via(paths: &[&str]) -> io::Result<()> {
    for path in paths {
        write_sysctl(path, "1\n")?;
    }
    Ok(())
}

fn write_sysctl(path: &str, value: &str) -> io::Result<()> {
    std::fs::write(path, value)
        .map_err(|e| io::Error::new(e.kind(), format!("#7 /proc/sys 直写失败 {path}: {e}")))
}

/// 就绪等待（provider 无关，ns 内视角；bootstrap 内、exec 前调用）：
/// netlink dump 断言 ① iface 存在且 IFF_UP（RTM_GETLINK）② v4 默认路由
/// oif=iface（RTM_GETROUTE）。100ms 间隔轮询；超时 fail-loud（#8）：文案含
/// 步骤名/接口/超时/gateway.log 指针（修正 audit-facts §5 的静默反例）。
pub fn wait_ready(iface: &str, timeout: Duration) -> io::Result<()> {
    let mut sock = Socket::new(NETLINK_ROUTE)?;
    sock.bind(&SocketAddr::new(0, 0))?;
    let deadline = Instant::now() + timeout;
    let mut seq: u32 = 1;
    let mut last_step = "RTM_GETLINK（iface 存在 + IFF_UP）";
    loop {
        let mut ifindex = None;
        dump_once(
            &sock,
            RouteNetlinkMessage::GetLink(LinkMessage::default()),
            seq,
            |msg| {
                if let RouteNetlinkMessage::NewLink(link) = msg {
                    if iface_of(&link.attributes).as_deref() == Some(iface)
                        && link.header.flags.contains(LinkFlags::Up)
                    {
                        ifindex = Some(link.header.index);
                        return true;
                    }
                }
                false
            },
        )?;
        seq = seq.wrapping_add(1);
        if let Some(idx) = ifindex {
            last_step = "RTM_GETROUTE（v4 默认路由 oif=iface）";
            let mut ready = false;
            dump_once(
                &sock,
                RouteNetlinkMessage::GetRoute(RouteMessage::default()),
                seq,
                |msg| {
                    if let RouteNetlinkMessage::NewRoute(route) = msg {
                        if route.header.address_family == AddressFamily::Inet
                            && route.header.destination_prefix_length == 0
                            && route
                                .attributes
                                .iter()
                                .any(|a| matches!(a, RouteAttribute::Oif(o) if *o == idx))
                        {
                            ready = true;
                            return true;
                        }
                    }
                    false
                },
            )?;
            seq = seq.wrapping_add(1);
            if ready {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(timeout_error(iface, timeout, last_step));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn iface_of(nlas: &[LinkAttribute]) -> Option<String> {
    nlas.iter().find_map(|nla| match nla {
        LinkAttribute::IfName(name) => Some(name.clone()),
        _ => None,
    })
}

fn timeout_error(iface: &str, timeout: Duration, step: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "#8 wait_ready 超时（{}ms）：netlink 断言未就绪 iface={iface}，最后步骤 [{step}]；\
             两 provider 均 self-config（pasta --config-net / slirp -c），持续未就绪请查网关日志：\
             宿主侧 ~/.local/state/iso-cc/sessions/<id>/gateway.log",
            timeout.as_millis(),
        ),
    )
}

/// socks worker 代理地址发现（工单 16）：iface 默认路由的 v4 网关地址。
/// netns 内 ground truth 走 netlink dump（sysfs 为宿主视图伪影，09 取证 #5）；
/// 该地址经 pasta `--map-host-loopback` 默认映射宿主 loopback，即 worker 的
/// `socks5://<gw>:<port>` 代理地址。前置条件 = [`wait_ready`] 同 iface 已就绪
/// （v4 默认路由存在为 dump 断言的前提）。
pub fn default_gateway(iface: &str) -> io::Result<Ipv4Addr> {
    let mut sock = Socket::new(NETLINK_ROUTE)?;
    sock.bind(&SocketAddr::new(0, 0))?;
    let mut ifindex = None;
    dump_once(
        &sock,
        RouteNetlinkMessage::GetLink(LinkMessage::default()),
        1,
        |msg| {
            if let RouteNetlinkMessage::NewLink(link) = msg {
                if iface_of(&link.attributes).as_deref() == Some(iface) {
                    ifindex = Some(link.header.index);
                    return true;
                }
            }
            false
        },
    )?;
    let idx = ifindex.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("#B netlink 无 iface={iface}（worker 代理地址不可得）"),
        )
    })?;
    let mut gw = None;
    dump_once(
        &sock,
        RouteNetlinkMessage::GetRoute(RouteMessage::default()),
        2,
        |msg| {
            if let RouteNetlinkMessage::NewRoute(route) = msg {
                if let Some(g) = gateway_of(&route, idx) {
                    gw = Some(g);
                    return true;
                }
            }
            false
        },
    )?;
    gw.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("#B iface={iface} 默认路由无 v4 网关属性（worker socks5 地址不可得）"),
        )
    })
}

/// socks worker tun 就绪等待（工单 16）：tun2proxy `-s` 以 0.0.0.0/1 + 128.0.0.0/1
/// 双 halving 路由承载默认路径（真 0/0 恒留在 tap0 网关上）——[`wait_ready`] 的
/// 「v4 默认路由」断言在此恒假。断言改为：① iface 存在且 UP ② 存在 oif=iface 的
/// prefixlen∈{0,1} 路由（halving 或真默认，两者取其一即可）。
pub fn wait_tun_ready(iface: &str, timeout: Duration) -> io::Result<()> {
    let mut sock = Socket::new(NETLINK_ROUTE)?;
    sock.bind(&SocketAddr::new(0, 0))?;
    let deadline = Instant::now() + timeout;
    let mut seq: u32 = 1;
    let mut last_step = "RTM_GETLINK（iface 存在 + IFF_UP）";
    loop {
        let mut ifindex = None;
        dump_once(
            &sock,
            RouteNetlinkMessage::GetLink(LinkMessage::default()),
            seq,
            |msg| {
                if let RouteNetlinkMessage::NewLink(link) = msg {
                    if iface_of(&link.attributes).as_deref() == Some(iface)
                        && link.header.flags.contains(LinkFlags::Up)
                    {
                        ifindex = Some(link.header.index);
                        return true;
                    }
                }
                false
            },
        )?;
        seq = seq.wrapping_add(1);
        if let Some(idx) = ifindex {
            last_step = "RTM_GETROUTE（oif=iface 的 halving/默认路由）";
            let mut ready = false;
            dump_once(
                &sock,
                RouteNetlinkMessage::GetRoute(RouteMessage::default()),
                seq,
                |msg| {
                    if let RouteNetlinkMessage::NewRoute(route) = msg {
                        if route.header.address_family == AddressFamily::Inet
                            && matches!(route.header.destination_prefix_length, 0 | 1)
                            && route
                                .attributes
                                .iter()
                                .any(|a| matches!(a, RouteAttribute::Oif(o) if *o == idx))
                        {
                            ready = true;
                            return true;
                        }
                    }
                    false
                },
            )?;
            seq = seq.wrapping_add(1);
            if ready {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(timeout_error(iface, timeout, last_step));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// 纯判定核心（可单测）：v4 默认路由（AF_INET + prefixlen=0 + oif）的 Gateway 属性。
fn gateway_of(route: &RouteMessage, oif: u32) -> Option<Ipv4Addr> {
    if route.header.address_family != AddressFamily::Inet
        || route.header.destination_prefix_length != 0
    {
        return None;
    }
    let mut gw = None;
    let mut oif_ok = false;
    for a in &route.attributes {
        match a {
            RouteAttribute::Oif(o) if *o == oif => oif_ok = true,
            RouteAttribute::Gateway(RouteAddress::Inet(ip)) => gw = Some(ip),
            _ => {}
        }
    }
    if oif_ok {
        gw.copied()
    } else {
        None
    }
}

/// 发送一个 dump 请求（REQUEST|DUMP）并收取回复直至 NLMSG_DONE。
/// `visit` 命中即停止匹配（但消费完整个 dump，防旧消息污染下一次轮询）。
/// 内核 NACK（非零 errno）fail-loud；解析畸形（长度非法）fail-loud。
fn dump_once<F>(sock: &Socket, req: RouteNetlinkMessage, seq: u32, mut visit: F) -> io::Result<bool>
where
    F: FnMut(RouteNetlinkMessage) -> bool,
{
    let mut request = NetlinkMessage::from(req);
    request.header.flags = NLM_F_REQUEST | NLM_F_DUMP;
    request.header.sequence_number = seq;
    request.finalize();
    let mut buf = vec![0u8; request.buffer_len()];
    request.serialize(&mut buf);
    sock.send(&buf, 0)?;

    let mut found = false;
    loop {
        let (data, _) = sock.recv_from_full()?;
        let mut offset = 0usize;
        let mut done = false;
        while offset < data.len() {
            let msg = NetlinkMessage::<RouteNetlinkMessage>::deserialize(&data[offset..]).map_err(
                |e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("netlink 消息解析失败（offset={offset}）: {e}"),
                    )
                },
            )?;
            let len = msg.header.length as usize;
            if len < NETLINK_HEADER_LEN || offset + len > data.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "netlink 消息长度畸形（len={len}, offset={offset}, 报文={}B）",
                        data.len()
                    ),
                ));
            }
            offset += len;
            match msg.payload {
                NetlinkPayload::InnerMessage(inner) => {
                    if !found {
                        found = visit(inner);
                    }
                }
                NetlinkPayload::Done(_) => {
                    done = true;
                    break;
                }
                NetlinkPayload::Error(err) => {
                    if let Some(code) = err.code {
                        return Err(io::Error::from_raw_os_error(code.get()));
                    }
                    // code=None = ACK，继续
                }
                _ => {}
            }
        }
        if done {
            return Ok(found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 票 12 验收 #3 前半：wait_ready 喂超时 → 带上下文失败
    /// （步骤名/接口/超时/netlink/gateway.log 指针全在文案；宿主 netns 下
    /// 该接口名恒不存在 → 恒超时，确定性）。
    #[test]
    fn wait_ready_timeout_fails_loud_with_context() {
        let err = wait_ready("iso-cc-nonexistent0", Duration::from_millis(250)).unwrap_err();
        let msg = format!("{err}");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{msg}");
        assert!(msg.contains("wait_ready"), "{msg}");
        assert!(msg.contains("iso-cc-nonexistent0"), "{msg}");
        assert!(msg.contains("250ms"), "{msg}");
        assert!(msg.contains("netlink"), "{msg}");
        assert!(msg.contains("gateway.log"), "{msg}");
    }

    /// 票 12 验收 #3 后半：disable_ipv6 写失败路径 fail-loud
    /// （路径 + errno 均在文案；用恒不存在的路径，绝不触碰真实 sysctl）。
    #[test]
    fn disable_ipv6_write_failure_fails_loud() {
        let err = disable_ipv6_via(&["/proc/iso-cc-nonexistent/disable_ipv6"]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("#7"), "{msg}");
        assert!(
            msg.contains("/proc/iso-cc-nonexistent/disable_ipv6"),
            "{msg}"
        );
        assert!(msg.contains("os error"), "{msg}");
    }

    /// 工单 16：gateway_of 只认 v4 默认路由 + oif 匹配，Gateway 属性解出 Ipv4Addr。
    #[test]
    fn gateway_of_extracts_v4_gateway_of_matching_oif() {
        let mut route = RouteMessage::default();
        route.header.address_family = AddressFamily::Inet;
        route.header.destination_prefix_length = 0;
        route.attributes = vec![
            RouteAttribute::Oif(7),
            RouteAttribute::Gateway(RouteAddress::Inet(Ipv4Addr::new(172, 27, 0, 1))),
        ];
        assert_eq!(gateway_of(&route, 7), Some(Ipv4Addr::new(172, 27, 0, 1)));
        assert_eq!(gateway_of(&route, 9), None);
        route.header.destination_prefix_length = 24;
        assert_eq!(gateway_of(&route, 7), None);
        route.header.address_family = AddressFamily::Inet6;
        route.header.destination_prefix_length = 0;
        assert_eq!(gateway_of(&route, 7), None);
    }
}
