use crate::config::{NetIpv6, NetPrivate, NetScope, Profile};
use std::ffi::OsString;

/// `--print-plan` 的唯一产出：由 config + 声明推导，确定性、可单测。
/// 真正的执行细节（realpath、pasta 参数展开）在 T2/T3 接入后逐行补齐。
pub fn plan_lines(profile_name: &str, p: &Profile, command: Option<&OsString>) -> Vec<String> {
    let mut v = Vec::new();
    let iface = p.egress_iface().unwrap_or("<invalid-egress>");
    v.push(format!(
        "plan[profile={profile_name} scope={:?} ipv6={:?} private={:?}]",
        p.scope(),
        p.ipv6(),
        p.net.private.unwrap_or(NetPrivate::Tunnel)
    ));
    v.push("1. unshare CLONE_NEWUSER|CLONE_NEWNS|CLONE_NEWNET".into());
    v.push("2. mountns: make-rprivate /".into());
    if let Some(tz) = p.locale.tz.as_deref() {
        v.push(format!(
            "3. bind /usr/share/zoneinfo/{tz} -> /etc/localtime (realpath) + /etc/timezone"
        ));
    } else {
        v.push("3. (no tz declared — host default)".into());
    }
    for r in &p.redirect {
        let (src, dst) = r.split_once('=').unwrap_or((r.as_str(), "?"));
        v.push(format!("4. bind {src} -> {dst}"));
    }
    v.push(format!(
        "5. pasta attach: -i {iface} --map-host-loopback (egress pinned)"
    ));
    if p.ipv6() == NetIpv6::Off {
        v.push("6. disable ipv6 in netns (fail-closed)".into());
    }
    for port in &p.net.localhost_forward {
        v.push(format!(
            "7. relay 127.0.0.1:{port} -> gateway -> host loopback"
        ));
    }
    for (k, val) in &p.env {
        v.push(format!("8. env {k}={val:?}"));
    }
    match p.scope() {
        NetScope::Self_ => {
            v.push("8. scope=self: bash -c 走 exec.sock RPC 到宿主 netns 执行（T2 后接入）".into());
        }
        NetScope::Tree => {
            v.push("8. scope=tree: bash 留在 netns 内执行".into());
        }
    }
    match command {
        Some(c) => v.push(format!(
            "9. exec {} (env: TZ/LANG/CLAUDE 重定向已生效)",
            c.to_string_lossy()
        )),
        None => v.push("9. (no command given — plan only)".into()),
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prof(toml_str: &str) -> Profile {
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn tree_scope_plan_contains_pinned_egress_and_v6_off() {
        let p = prof("egress = 'if:wg0'\nlocale.tz = 'Asia/Singapore'");
        let lines = plan_lines("sg", &p, None);
        let joined = lines.join("\n");
        assert!(joined.contains("-i wg0"));
        assert!(joined.contains("disable ipv6"));
        assert!(joined.contains("Asia/Singapore"));
        assert!(joined.contains("scope=Tree"));
    }

    #[test]
    fn self_scope_plan_declares_rpc_transport() {
        let p = prof("egress = 'if:wg0'\nnet.scope = 'self'");
        let lines = plan_lines("x", &p, Some(&OsString::from("claude")));
        assert!(
            lines.iter().any(|l| l.contains("exec.sock RPC")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("exec claude")));
    }

    #[test]
    fn localhost_forward_ports_listed() {
        let p = prof("egress = 'if:wg0'\nnet.localhost_forward = [5432, 6379]");
        let lines = plan_lines("x", &p, None);
        assert!(lines.iter().any(|l| l.contains("127.0.0.1:5432")));
        assert!(lines.iter().any(|l| l.contains("127.0.0.1:6379")));
    }
}
