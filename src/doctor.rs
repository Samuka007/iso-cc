use crate::config::{Config, Profile};
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// cc 平台面必达域名（network-config 文档 + 研究取证；doctor 可达性校验集种子）。
pub const MUST_REACH: &[&str] = &[
    "api.anthropic.com",
    "claude.ai",
    "claude.com",
    "platform.claude.com",
    "registry.npmjs.org",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn new(name: impl Into<String>, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// 宿主事实的注入缝：doctor 的全部检查只依赖这个 trait（测试用 FakeSys）。
pub trait SysInspect {
    fn read_sysctl(&self, path: &str) -> io::Result<String>;
    fn iface_exists(&self, name: &str) -> bool;
    fn iface_has_route(&self, name: &str) -> bool;
    fn iface_has_default_route(&self, name: &str) -> bool;
    fn path_exists(&self, path: &Path) -> bool;
    fn which(&self, bin: &str) -> Option<PathBuf>;
    fn command_output(&self, cmd: &str) -> Option<String>;
    fn locale_available(&self, lang: &str) -> bool;
    fn resolve_connect(&self, host: &str, port: u16) -> Result<(), String>;
}

pub struct RealSys;

impl SysInspect for RealSys {
    fn read_sysctl(&self, path: &str) -> io::Result<String> {
        Ok(std::fs::read_to_string(path)?.trim().to_string())
    }
    fn iface_exists(&self, name: &str) -> bool {
        Path::new("/sys/class/net").join(name).exists()
    }
    fn iface_has_route(&self, name: &str) -> bool {
        routes_for(name).is_some_and(|r| !r.is_empty())
    }
    fn iface_has_default_route(&self, name: &str) -> bool {
        routes_for(name).is_some_and(|r| r.iter().any(|dest| dest == "00000000"))
    }
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn which(&self, bin: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(bin))
            .find(|cand| cand.is_file())
    }
    fn command_output(&self, cmd: &str) -> Option<String> {
        let out = std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    fn locale_available(&self, lang: &str) -> bool {
        let Ok(out) = std::process::Command::new("locale").arg("-a").output() else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        let stem = lang.split('.').next().unwrap_or(lang).to_lowercase();
        text.lines()
            .any(|l| l.trim() == stem || l.trim() == lang.to_lowercase())
    }
    fn resolve_connect(&self, host: &str, port: u16) -> Result<(), String> {
        let addrs: Vec<_> = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("解析失败: {e}"))?
            .collect();
        let mut last = String::from("无可用地址");
        for a in addrs {
            match TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
                Ok(_) => return Ok(()),
                Err(e) => last = format!("{a}: {e}"),
            }
        }
        Err(last)
    }
}

/// /proc/net/route 里属于某接口的目的地址（十六进制小端）。
fn routes_for(iface: &str) -> Option<Vec<String>> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    let mut dests = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 3 && cols[0] == iface {
            dests.push(cols[1].to_string());
        }
    }
    Some(dests)
}

/// 全量 doctor：返回检查清单（任何 Fail → 调用方 exit 1，fail-loud）。
pub fn run(
    _cfg: &Config,
    _profile_name: &str,
    profile: &Profile,
    sys: &dyn SysInspect,
) -> Vec<Check> {
    let mut out = Vec::new();

    // 1. userns 可用性（R5）
    match sys.read_sysctl("/proc/sys/user/max_user_namespaces") {
        Ok(v) if v.parse::<i64>().unwrap_or(0) > 0 => {
            out.push(Check::new("userns/max_user_namespaces", Status::Ok, v));
        }
        Ok(v) => out.push(Check::new(
            "userns/max_user_namespaces",
            Status::Fail,
            format!("{v}（必须 > 0）"),
        )),
        Err(e) => out.push(Check::new(
            "userns/max_user_namespaces",
            Status::Fail,
            e.to_string(),
        )),
    }
    // Debian/Ubuntu 专属开关；不存在 = 主线内核默认允许
    match sys.read_sysctl("/proc/sys/kernel/unprivileged_userns_clone") {
        Ok(v) if v == "1" => out.push(Check::new("userns/unprivileged_clone", Status::Ok, "1")),
        Ok(v) => out.push(Check::new(
            "userns/unprivileged_clone",
            Status::Fail,
            format!("{v}（必须 = 1）"),
        )),
        Err(_) => out.push(Check::new(
            "userns/unprivileged_clone",
            Status::Ok,
            "无此 sysctl（主线内核，默认允许）",
        )),
    }
    // Ubuntu 23.10+/24.04 AppArmor 限制（P3 后置；fail-loud 提示）
    if let Ok(v) = sys.read_sysctl("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") {
        if v == "1" {
            out.push(Check::new(
                "userns/apparmor",
                Status::Warn,
                "已启用限制：rootless userns 将被拒；需安装 iso-cc apparmor profile（P3）",
            ));
        }
    }

    // 2. egress 接口 + 路由（R7/D4）
    match profile.egress_iface() {
        Ok(iface) => {
            if sys.iface_exists(iface) {
                out.push(Check::new("egress/iface", Status::Ok, iface));
                let status = if sys.iface_has_default_route(iface) {
                    Status::Ok
                } else if sys.iface_has_route(iface) {
                    Status::Warn
                } else {
                    Status::Fail
                };
                out.push(Check::new("egress/route", status, format!("iface {iface}")));
            } else {
                out.push(Check::new(
                    "egress/iface",
                    Status::Fail,
                    format!("接口 {iface} 不存在"),
                ));
            }
        }
        Err(e) => out.push(Check::new("egress/iface", Status::Fail, e.to_string())),
    }

    // 3. locale（R2）
    if let Some(tz) = &profile.locale.tz {
        let zpath = Path::new("/usr/share/zoneinfo").join(tz);
        let ok = sys.path_exists(&zpath);
        out.push(Check::new(
            "locale/zoneinfo",
            if ok { Status::Ok } else { Status::Fail },
            format!("/usr/share/zoneinfo/{tz}"),
        ));
    }
    if let Some(lang) = &profile.locale.lang {
        out.push(Check::new(
            "locale/lang",
            if sys.locale_available(lang) {
                Status::Ok
            } else {
                Status::Warn
            },
            format!("{lang}（未生成则回退 C.UTF-8）"),
        ));
    }

    // 4. provider（R11/D7）
    let pasta = sys.which("pasta");
    let slirp = sys.which("slirp4netns");
    match (&pasta, &slirp) {
        (Some(p), _) => out.push(Check::new(
            "provider/pasta",
            Status::Ok,
            p.display().to_string(),
        )),
        (None, Some(s)) => out.push(Check::new(
            "provider/slirp4netns",
            Status::Ok,
            format!("{}（回退 provider）", s.display()),
        )),
        (None, None) => out.push(Check::new(
            "provider",
            Status::Fail,
            "pasta 与 slirp4netns 均未安装",
        )),
    }

    // 5. agent 发现（R9/D3：advisory）
    let cmd_name = profile.agent.command.as_deref().unwrap_or("claude");
    match sys.which(cmd_name) {
        Some(p) => {
            let ver = sys
                .command_output(cmd_name)
                .unwrap_or_else(|| "版本未知".into());
            let (status, note) = match profile.agent.version.as_deref() {
                Some(want) if !ver.contains(want) => {
                    (Status::Warn, format!("；声明 {want}，实测不符（advisory）"))
                }
                Some(want) => (Status::Ok, format!("；声明 {want} 命中")),
                None => (Status::Ok, String::new()),
            };
            out.push(Check::new(
                "agent/found",
                status,
                format!("{cmd_name} → {}（{ver}）{note}", p.display()),
            ));
        }
        None => out.push(Check::new(
            "agent/found",
            Status::Fail,
            format!("{cmd_name} 不在 PATH"),
        )),
    }

    // 6. 必达域名（doctor 在宿主上下文验证当前现实；会话内路径归 verify）
    for host in MUST_REACH {
        match sys.resolve_connect(host, 443) {
            Ok(()) => out.push(Check::new("reach", Status::Ok, format!("{host}:443"))),
            Err(e) => out.push(Check::new("reach", Status::Warn, format!("{host}: {e}"))),
        }
    }

    // 7. 重定向挂载点预览（R4）
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    for (name, p) in [
        ("~/.claude", PathBuf::from(&home).join(".claude")),
        ("~/.claude.json", PathBuf::from(&home).join(".claude.json")),
    ] {
        let exists = sys.path_exists(&p);
        out.push(Check::new(
            "redirect/mountpoint",
            if exists { Status::Ok } else { Status::Warn },
            format!("{name} 存在性 = {exists}（缺失将预创建）"),
        ));
    }

    out
}

pub fn any_fail(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}

pub fn render_human(checks: &[Check]) -> String {
    let mut s = String::new();
    for c in checks {
        let tag = match c.status {
            Status::Ok => "OK  ",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        };
        s.push_str(&format!("[{tag}] {} — {}\n", c.name, c.detail));
    }
    let fails = checks.iter().filter(|c| c.status == Status::Fail).count();
    let warns = checks.iter().filter(|c| c.status == Status::Warn).count();
    s.push_str(&format!(
        "\n{} checks: {} ok, {warns} warn, {fails} fail\n",
        checks.len(),
        checks.len() - fails - warns
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 FakeSys：全部通过的基线，单测按需覆写。
    struct FakeSys {
        max_ns: String,
        clone_switch: Option<String>,
        iface: bool,
        default_route: bool,
    }

    impl Default for FakeSys {
        fn default() -> Self {
            Self {
                max_ns: "1024".into(),
                clone_switch: None,
                iface: true,
                default_route: true,
            }
        }
    }

    impl SysInspect for FakeSys {
        fn read_sysctl(&self, path: &str) -> io::Result<String> {
            if path.ends_with("max_user_namespaces") {
                return Ok(self.max_ns.clone());
            }
            if path.ends_with("unprivileged_userns_clone") {
                return match &self.clone_switch {
                    Some(v) => Ok(v.clone()),
                    None => Err(io::Error::new(io::ErrorKind::NotFound, "no sysctl")),
                };
            }
            if path.ends_with("apparmor_restrict_unprivileged_userns") {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no apparmor"));
            }
            Err(io::Error::new(io::ErrorKind::NotFound, "unknown"))
        }
        fn iface_exists(&self, _name: &str) -> bool {
            self.iface
        }
        fn iface_has_route(&self, _name: &str) -> bool {
            self.default_route || true
        }
        fn iface_has_default_route(&self, _name: &str) -> bool {
            self.default_route
        }
        fn path_exists(&self, _path: &Path) -> bool {
            true
        }
        fn which(&self, _bin: &str) -> Option<PathBuf> {
            Some(PathBuf::from("/usr/bin/fake"))
        }
        fn command_output(&self, _cmd: &str) -> Option<String> {
            Some("1.0.0".into())
        }
        fn locale_available(&self, _lang: &str) -> bool {
            true
        }
        fn resolve_connect(&self, _host: &str, _port: u16) -> Result<(), String> {
            Ok(())
        }
    }

    fn profile(egress: &str) -> Profile {
        toml::from_str(&format!("egress = {egress:?}")).unwrap()
    }

    #[test]
    fn healthy_host_is_all_ok() {
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &FakeSys::default(),
        );
        assert!(!any_fail(&checks), "{checks:?}");
    }

    #[test]
    fn missing_iface_is_fail() {
        let p = profile("if:wg0");
        let sys = FakeSys {
            iface: false,
            ..Default::default()
        };
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        assert!(any_fail(&checks));
    }

    #[test]
    fn clone_switch_zero_is_fail() {
        let p = profile("if:wg0");
        let sys = FakeSys {
            clone_switch: Some("0".into()),
            ..Default::default()
        };
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        assert!(any_fail(&checks));
    }

    #[test]
    fn apparmor_restriction_is_warn_not_fail() {
        let p = profile("if:wg0");
        struct AaSys;
        impl SysInspect for AaSys {
            fn read_sysctl(&self, path: &str) -> io::Result<String> {
                if path.ends_with("apparmor_restrict_unprivileged_userns") {
                    Ok("1".into())
                } else if path.ends_with("max_user_namespaces") {
                    Ok("100".into())
                } else {
                    Err(io::Error::new(io::ErrorKind::NotFound, "n/a"))
                }
            }
            fn iface_exists(&self, _: &str) -> bool {
                true
            }
            fn iface_has_route(&self, _: &str) -> bool {
                true
            }
            fn iface_has_default_route(&self, _: &str) -> bool {
                true
            }
            fn path_exists(&self, _: &Path) -> bool {
                true
            }
            fn which(&self, _: &str) -> Option<PathBuf> {
                Some(PathBuf::from("/x"))
            }
            fn command_output(&self, _: &str) -> Option<String> {
                None
            }
            fn locale_available(&self, _: &str) -> bool {
                true
            }
            fn resolve_connect(&self, _: &str, _: u16) -> Result<(), String> {
                Ok(())
            }
        }
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &AaSys,
        );
        assert!(!any_fail(&checks));
        assert!(checks
            .iter()
            .any(|c| c.name == "userns/apparmor" && c.status == Status::Warn));
    }
}
