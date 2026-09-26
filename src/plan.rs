use crate::config::{ExecBash, NetGateway, NetIpv6, NetPrivate, Profile};
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
        "plan[profile={profile_name} scope={:?} exec.bash={:?} ipv6={:?} private={:?} gateway={gateway} dns={dns}]",
        p.scope(),
        p.exec_bash(),
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
    // 票 15（spec 变更（四））：两轴独立声明，--print-plan 展开全部组合
    v.push(format!(
        "10a. net.scope={:?}: identity scope（tree=整树入 netns 一致性 US2；self=cc 本体面收窄，verify 探针恒测 cc 本体）",
        p.scope()
    ));
    match p.exec_bash() {
        ExecBash::Sandbox => {
            v.push(
                "10b. exec.bash=sandbox: bash 工具留在会话内执行（继承 netns；行为与现状等价）"
                    .into(),
            );
        }
        ExecBash::Host => {
            v.push(
                "10b. exec.bash=host: bash 调用经 <sessions/<id>/exec.sock> RPC 宿主侧 /bin/bash -c 执行（US9 真 localhost；bash 流量走宿主出口 = 显式声明的代价）".into(),
            );
            v.push(
                "10b1. 拦截分层注入: L1.5 CLAUDE_CODE_SHELL_PREFIX + L1 SHELL + L2 PATH 前置 <sessions/<id>/bin> → multi-call shim（iso-cc 自身，argv0 basename=bash）"
                    .into(),
            );
            v.push(
                "10b2. 覆盖面（ADR 0008 附 3 取证 + 通道实测）: Bash 工具/hooks/statusline/stdio MCP 经 L1.5 宿主执行；REPL !cmd 与 skill !cmd 硬编码 /bin/sh——L1/L2 不覆盖、L1.5 prefix 覆盖；WebSearch/WebFetch 无客户端 bash 面；完全 in-process 执行器无钩点（诚实边界）"
                    .into(),
            );
        }
    }
    v.push(format!(
        "10c. combo: net.scope={:?} + exec.bash={:?}",
        p.scope(),
        p.exec_bash()
    ));
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
            lines.iter().any(|l| l.contains("net.scope=Self")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("exec claude")));
    }

    #[test]
    fn exec_bash_host_plan_declares_channel_and_layers() {
        let p = prof("egress = 'if:wg0'\nexec.bash = 'host'");
        let lines = plan_lines("x", &p, Some(&OsString::from("claude")));
        let joined = lines.join("\n");
        assert!(joined.contains("exec.bash=Host"), "{joined}");
        assert!(joined.contains("exec.sock"), "{joined}");
        assert!(joined.contains("L1.5 CLAUDE_CODE_SHELL_PREFIX"), "{joined}");
        assert!(joined.contains("multi-call shim"), "{joined}");
        // 两轴独立：scope 默认 tree 与 host 组合可见（变更（四）四组合之 tree+host）
        assert!(joined.contains("net.scope=Tree"), "{joined}");
        assert!(joined.contains("combo: net.scope=Tree + exec.bash=Host"), "{joined}");
    }

    #[test]
    fn plan_expands_all_four_combos() {
        for (scope, bash, combo) in [
            ("tree", "sandbox", "combo: net.scope=Tree + exec.bash=Sandbox"),
            ("tree", "host", "combo: net.scope=Tree + exec.bash=Host"),
            ("self", "host", "combo: net.scope=Self_ + exec.bash=Host"),
            ("self", "sandbox", "combo: net.scope=Self_ + exec.bash=Sandbox"),
        ] {
            let p = prof(&format!("egress = 'if:wg0'\nnet.scope = '{scope}'\nexec.bash = '{bash}'"));
            let lines = plan_lines("x", &p, None);
            assert!(
                lines.iter().any(|l| l.contains(combo)),
                "{scope}+{bash} 组合未展开: {lines:?}"
            );
        }
    }

    #[test]
    fn sandbox_default_plan_states_equivalence() {
        let p = prof("egress = 'if:wg0'");
        let lines = plan_lines("x", &p, None);
        let joined = lines.join("\n");
        assert!(joined.contains("exec.bash=Sandbox"), "{joined}");
        assert!(joined.contains("行为与现状等价"), "{joined}");
        assert!(!joined.contains("exec.sock"), "sandbox 不得出现 exec.sock 通道面: {joined}");
    }

    #[test]
    fn localhost_forward_ports_listed() {
        let p = prof("egress = 'if:wg0'\nnet.localhost_forward = [5432, 6379]");
        let lines = plan_lines("x", &p, None);
        assert!(lines.iter().any(|l| l.contains("127.0.0.1:5432")));
        assert!(lines.iter().any(|l| l.contains("127.0.0.1:6379")));
    }
}
