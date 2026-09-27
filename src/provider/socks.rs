//! socks5 egress 形态（工单 16，D4 修订第二形态）：会话根仍是 pasta（spawn 模式，
//! outbound = 宿主默认路由接口），netns 内由 tun2proxy worker 在 tun1 上把全部出口
//! 流量送经宿主 SOCKS5（宿主 mixed-port 经 pasta `--map-host-loopback` 默认的
//! 「网关地址 → 宿主 loopback」映射可达）。
//!
//! 择型证据（阶段 A 实测，/tmp/iso-cc-exp16/REPORT.md）：组合成立，零自研 netstack
//! （embedded 降级为远期）。proven invocation：
//! `tun2proxy --tun tun1 --proxy socks5://<tap0-gw>:<port> -s -v info`
//! - 路由例外自动：`-s` 以 0/1+128.0.0/1 劫持默认路径，tap0 网关地址经最长前缀匹配
//!   留在链路路由，proxy 连接不环路；
//! - DNS：tproxy-config 启动即接管 ns 内 /etc/resolv.conf → 10.0.0.1（tun1 网关 =
//!   其内建虚拟 DNS），DNS 全程隧道内、经 proxy 远端解析；iso-cc 的 bind_ro resolv.conf
//!   之下接管仍成功且 worker 存活（RO 写测试 EROFS 复现）；
//! - 失败语义：worker 死 = tun1 路由无读者 = 协议黑洞，绝不回落直连（R8 同构）；
//!   spawn 失败 = bootstrap fail-loud。
//!
//! doctor 的 socks geo 对照（宿主侧预检）用本模块的最小 SOCKS5 CONNECT 客户端
//! （std-only：RFC 1928 无鉴权子集 + 明文 HTTP GET，不引入任何新依赖）。

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// worker 在清单里的 provider 条目键（bin 名 PATH 上可能是 `tun2proxy` 或 nixpkgs 的
/// `tun2proxy-bin`，见 [`BIN_CANDIDATES`]；清单 key 统一用 [`WORKER_KEY`]）。
pub const WORKER_KEY: &str = "tun2proxy";

/// PATH 上可接受的二进制名（顺序即优先级）。
pub const BIN_CANDIDATES: &[&str] = &["tun2proxy", "tun2proxy-bin"];

/// netns 内 worker 自建 TUN 名（与 pasta 的 tap0 并存；wait_ready netlink 断言就绪）。
pub const TUN_IFNAME: &str = "tun1";

/// worker argv 展开（spawn 与 `--print-plan` 单一事实源）。
/// 恒为 proven invocation 形态（阶段 A T1/T2/probe4 实测）：不传 `--dns`（默认 direct
/// + resolv.conf 接管即为实测形态）、不带 `-6`（IPv6 默认 off，P2 fail-closed）。
pub fn worker_args(gateway: Ipv4Addr, port: u16) -> Vec<OsString> {
    worker_args_for_url(format!("socks5://{gateway}:{port}"))
}

/// [`worker_args`] 的 URL 参数化变体（`--print-plan` 在 plan 期尚无网关地址，
/// 以 `socks5://<tap0-gw>:<port>` 占位渲染同一 flag 序列——所印即可执行的等价 CLI）。
pub fn worker_args_for_url(proxy_url: String) -> Vec<OsString> {
    vec![
        "--tun".into(),
        TUN_IFNAME.into(),
        "--proxy".into(),
        proxy_url.into(),
        "-s".into(),
        "-v".into(),
        "info".into(),
    ]
}

/// 宿主侧 spawn 前预检（fail-loud，R8：绝不回落）：proxy 端口 TCP 可达。
/// 3s 超时逐地址尝试（与 doctor RealSys::resolve_connect 同规）。
pub fn host_preflight(host: &str, port: u16) -> anyhow::Result<()> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("#B socks proxy 地址解析失败 {host}:{port}: {e}"))?
        .collect();
    if addrs.is_empty() {
        anyhow::bail!("#B socks proxy 地址解析为空：{host}:{port}；R8：绝不回落");
    }
    let mut last = String::from("无可用地址");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
            Ok(_) => return Ok(()),
            Err(e) => last = format!("{a}: {e}"),
        }
    }
    anyhow::bail!(
        "#B socks proxy {host}:{port} 不可达（{last}）；R8：绝不回落——检查宿主代理服务"
    );
}

/// 纯函数（可单测）：SOCKS5 CONNECT 请求字节（VER=5 CMD=1 RSV=0 ATYP=3 域名）。
/// 注意：问候字节 [05 01 00] 属于前一握手步，不在此重复。
fn connect_request(dst_host: &str, dst_port: u16) -> Vec<u8> {
    let mut v = vec![0x05, 0x01, 0x00, 0x03, dst_host.len() as u8];
    v.extend_from_slice(dst_host.as_bytes());
    v.extend_from_slice(&dst_port.to_be_bytes());
    v
}

/// 纯函数（可单测）：校验 SOCKS5 应答（greeting 与 CONNECT 共用前两字节判定）。
fn reply_ok(reply: &[u8]) -> Result<(), String> {
    if reply.len() < 2 {
        return Err(format!("应答过短（{}B）", reply.len()));
    }
    if reply[0] != 0x05 {
        return Err(format!("非 SOCKS5 应答（VER={:#x}）", reply[0]));
    }
    if reply[1] != 0x00 {
        return Err(format!("CONNECT 失败（REP={:#x}）", reply[1]));
    }
    Ok(())
}

/// 经 SOCKS5（无鉴权）对 dst_host:80 发明文 GET，返回 HTTP body（读至对端关闭）。
/// 仅用于 doctor 的 geo 预检（ip-api.com 明文端点）；错误一律 String（doctor Warn 面）。
pub fn http_get_via_socks5(
    proxy_host: &str,
    proxy_port: u16,
    dst_host: &str,
    dst_port: u16,
    path: &str,
) -> Result<String, String> {
    let addrs = (proxy_host, proxy_port)
        .to_socket_addrs()
        .map_err(|e| format!("proxy 地址解析失败: {e}"))?
        .collect::<Vec<_>>();
    let mut last = String::from("无可用地址");
    for a in addrs {
        if let Ok(mut s) = TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(8)));
            let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
            return dialogue(&mut s, dst_host, dst_port, path);
        }
        last = format!("{a}: 连接失败");
    }
    Err(last)
}

fn dialogue(s: &mut TcpStream, dst_host: &str, dst_port: u16, path: &str) -> Result<String, String> {
    s.write_all(&[0x05, 0x01, 0x00])
        .map_err(|e| format!("greeting 写失败: {e}"))?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply)
        .map_err(|e| format!("greeting 读失败: {e}"))?;
    reply_ok(&reply)?;
    s.write_all(&connect_request(dst_host, dst_port))
        .map_err(|e| format!("CONNECT 写失败: {e}"))?;
    // CONNECT 应答：VER REP RSV ATYP ADDR PORT，IPv4 最短 10B；读头部后按 ATYP 清尾
    let mut head = [0u8; 4];
    s.read_exact(&mut head)
        .map_err(|e| format!("CONNECT 应答读失败: {e}"))?;
    reply_ok(&head[..2])?;
    let drain = match head[3] {
        0x01 => 4 + 2,
        0x03 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l)
                .map_err(|e| format!("ATYP 域名长度读失败: {e}"))?;
            l[0] as usize + 2
        }
        0x04 => 16 + 2,
        other => return Err(format!("未知 ATYP={other:#x}")),
    };
    let mut rest = vec![0u8; drain];
    s.read_exact(&mut rest)
        .map_err(|e| format!("CONNECT 应答尾读失败: {e}"))?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {dst_host}\r\nConnection: close\r\nUser-Agent: iso-cc-doctor\r\n\r\n");
    s.write_all(req.as_bytes())
        .map_err(|e| format!("HTTP 请求写失败: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("HTTP 响应读失败: {e}"))?;
    let text = String::from_utf8_lossy(&raw);
    // 诚实边界：不解析 chunked（ip-api 的 Connection: close 响应为恒等编码）；
    // 取头部之后的 body，失败给原始前缀供 doctor 留档。
    match text.split_once("\r\n\r\n") {
        Some((headers, body)) => {
            let status = headers.lines().next().unwrap_or("");
            if status.contains(" 200") {
                Ok(body.to_string())
            } else {
                Err(format!("HTTP 非 200：{status}"))
            }
        }
        None => Err(format!("HTTP 响应无头部分隔（前 120B）：{}", &text[..text.len().min(120)])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_args_are_proven_form() {
        let args = worker_args(Ipv4Addr::new(172, 27, 0, 1), 7891);
        let joined: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            joined,
            vec![
                "--tun",
                "tun1",
                "--proxy",
                "socks5://172.27.0.1:7891",
                "-s",
                "-v",
                "info"
            ]
        );
    }

    #[test]
    fn connect_request_bytes_rfc1928() {
        let req = connect_request("ip-api.com", 80);
        assert_eq!(&req[..5], &[0x05, 0x01, 0x00, 0x03, 10]);
        assert_eq!(&req[5..15], b"ip-api.com");
        assert_eq!(&req[15..], &[0x00, 0x50]);
    }

    #[test]
    fn reply_verdicts() {
        assert!(reply_ok(&[0x05, 0x00]).is_ok());
        assert!(reply_ok(&[0x05, 0x01]).is_err()); // general failure
        assert!(reply_ok(&[0x04, 0x00]).is_err()); // 非 v5
        assert!(reply_ok(&[0x05]).is_err());
    }
}
