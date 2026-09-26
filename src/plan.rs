use crate::config::{NetGateway, NetIpv6, NetPrivate, NetScope, Profile};
use crate::provider;
use std::ffi::{OsStr, OsString};

/// `--print-plan` 的唯一产出：由 config + 声明推导，确定性、可单测。
/// gateway argv 展开与实际 spawn 同源（provider::*::flag_args）——所印即可执行的等价 CLI。
pub fn plan_lines(profile_name: &str, p: &Profile, command: Option<&OsString>) -> Vec<String> {
    let mut v = Vec::new();
    let iface = p.egress_iface().unwrap_or("<invalid-egress>");
    let gateway = p.gateway();
    let dns = p.dns();
    v.push(format!(
        "plan[profile={profile_name} scope={:?} ipv6={:?} private={:?} gateway={gateway} dns={dns}]",
        p.scope(),
        p.ipv6(),
        p.net.private.unwrap_or(NetPrivate::Tunnel)
    ));
    v.push(format!(
        "0. egress assert: {iface} must exist + UP on host (sysfs #3); lo/tap0 rejected (#12 `-I` rule)"
    ));
    match gateway {
        NetGateway::Pasta => {
            // pasta 全实参（含 `-I` 硬规则）；plan 期 session-id 未生成 → 占位
            let log = OsStr::new("<sessions/<session-id>/gateway.log>");
            let flags = provider::pasta::flag_args(iface, dns, log)
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            v.push(format!(
                "1. gateway=pasta spawn (session root): pasta {flags} -- <iso-cc> session-bootstrap --plan <json> --session-id <session-id> -- <cmd>"
            ));
            v.push(
                "2. bootstrap(mode=mountns): PDEATHSIG->pasta + getppid check; unshare CLONE_NEWNS; rprivate /; bind_ro x N".into(),
            );
        }
        NetGateway::Slirp4netns => {
            v.push(
                "1. gateway=slirp4netns (fallback, attach): slirp4netns <bootstrap-pid> tap0 -c"
                    .into(),
            );
            v.push(
                "2. bootstrap(mode=selfmap): parent pre_exec unshare CLONE_NEWUSER|CLONE_NEWNS|CLONE_NEWNET + self single-entry maps + rprivate + binds + PDEATHSIG->iso-cc (parent never writes /proc/<pid>/maps)".into(),
            );
        }
    }
    // bind 清单（会话资产位于 sessions/<session-id>/，并发互踩已修复）
    if p.locale.tz.is_some() {
        let tz = p.locale.tz.as_deref().unwrap_or("?");
        v.push(format!(
            "3. bind sessions/<id>/localtime + timezone (tzdb: {tz}) -> /etc/localtime + /etc/timezone"
        ));
    } else {
        v.push("3. (no tz declared — host default)".into());
    }
    v.push(format!(
        "4. bind sessions/<id>/resolv.conf -> /etc/resolv.conf (nameserver {dns})"
    ));
    for r in &p.redirect {
        let (src, dst) = r.split_once('=').unwrap_or((r.as_str(), "?"));
        v.push(format!("5. bind {src} -> {dst}"));
    }
    v.push(
        "6. tap self-config by provider (pasta --config-net / slirp -c): no ip addr/route commands"
            .into(),
    );
    if p.ipv6() == NetIpv6::Off {
        v.push(
            "7. ipv6=off: /proc/sys disable_ipv6 direct-write in bootstrap (landed with issue 12; sysctl sh removed in 09)".into(),
        );
    }
    for port in &p.net.localhost_forward {
        v.push(format!(
            "8. relay 127.0.0.1:{port} -> gateway -> host loopback"
        ));
    }
    for (k, val) in &p.env {
        v.push(format!("9. env {k}={val:?}"));
    }
    match p.scope() {
        NetScope::Self_ => {
            v.push(
                "10. scope=self: bash -c 走 exec.sock RPC 到宿主 netns 执行（T2 后接入）".into(),
            );
        }
        NetScope::Tree => {
            v.push("10. scope=tree: bash 留在 netns 内执行".into());
        }
    }
    match command {
        Some(c) => v.push(format!(
            "11. exec {} (env: TZ/LANG/CLAUDE 重定向已生效)",
            c.to_string_lossy()
        )),
        None => v.push("11. (no command given — plan only)".into()),
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
        assert!(joined.contains("gateway=pasta"), "{joined}");
        assert!(joined.contains("-I tap0"), "{joined}");
        assert!(joined.contains("--outbound-if4 wg0"), "{joined}");
        assert!(joined.contains("--config-net"), "{joined}");
        assert!(joined.contains("--dns-forward 10.0.2.3"), "{joined}");
        assert!(joined.contains("disable_ipv6"), "{joined}");
        assert!(joined.contains("Asia/Singapore"), "{joined}");
        assert!(joined.contains("scope=Tree"), "{joined}");
    }

    #[test]
    fn pasta_plan_expands_full_argv_with_bind_manifest() {
        let p = prof("egress = 'if:eth0'\nredirect = ['/home/me/.claude=/home/u/.claude']");
        let lines = plan_lines("x", &p, None);
        let joined = lines.join("\n");
        assert!(
            joined.contains("pasta -f -q -l <sessions/<session-id>/gateway.log> --config-net --outbound-if4 eth0 -I tap0 --dns-forward 10.0.2.3 --no-ndp --no-dhcpv6 --no-ra"),
            "{joined}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("bind /home/me/.claude -> /home/u/.claude")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("resolv.conf -> /etc/resolv.conf (nameserver 10.0.2.3)")),
            "{lines:?}"
        );
    }

    #[test]
    fn slirp_gateway_plan_declares_attach_and_selfmap() {
        let p = prof("egress = 'if:wg0'\nnet.gateway = 'slirp4netns'");
        let lines = plan_lines("x", &p, None);
        let joined = lines.join("\n");
        assert!(
            joined.contains("slirp4netns <bootstrap-pid> tap0 -c"),
            "{joined}"
        );
        assert!(joined.contains("selfmap"), "{joined}");
        assert!(joined.contains("never writes /proc/<pid>/maps"), "{joined}");
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
