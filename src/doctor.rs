use crate::config::{Config, NetGateway, Profile};
use crate::list::LifecycleResidue;
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
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
    /// 可执行位断言（provider 钉定路径；RealSys = metadata + mode & 0o111）。
    fn path_executable(&self, path: &Path) -> bool;
    fn which(&self, bin: &str) -> Option<PathBuf>;
    fn command_output(&self, cmd: &str) -> Option<String>;
    /// provider 版本事实（与 [`crate::provider::version_output`] 同规：`--version`
    /// stdout 的首个非空行）；None = 不可测（drift 比对跳过为 Warn）。
    fn provider_version(&self, path: &Path) -> Option<String>;
    fn locale_available(&self, lang: &str) -> bool;
    fn resolve_connect(&self, host: &str, port: u16) -> Result<(), String>;
    /// L3 sweep 注入缝（票 13）：孤儿网关 + 无主会话目录。default = 无 residue。
    fn lifecycle_residue(&self) -> LifecycleResidue {
        LifecycleResidue::default()
    }
    /// 清单注入缝（票 14）：None = 清单缺失（setup 未跑）；Some = 条目集；
    /// Err = 损坏/schema 未知（fail-loud）。
    fn manifest_state(&self) -> Result<Option<Vec<crate::manifest::Entry>>, String>;
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
    fn path_executable(&self, path: &Path) -> bool {
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
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
    fn provider_version(&self, path: &Path) -> Option<String> {
        crate::provider::version_output(path).ok()
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
    fn lifecycle_residue(&self) -> LifecycleResidue {
        crate::list::sweep_residue()
    }
    fn manifest_state(&self) -> Result<Option<Vec<crate::manifest::Entry>>, String> {
        match crate::manifest::read() {
            Ok(Some(m)) => Ok(Some(m.entries)),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
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
            // #12 `-I` 撞名断言：outbound 名与目标 ns 既有接口撞名类直接拒绝
            if let Err(e) = crate::provider::validate_egress_iface(iface) {
                out.push(Check::new("egress/iface", Status::Fail, e.to_string()));
            } else if sys.iface_exists(iface) {
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

    // 4. 清单双向校验（票 14 清单态；替换 09 期 #2 过渡态：which + 显式报错）。
    //    forward（清单→现实）：条目存在性、provider 可执行+版本一致（漂移 Warn/缺失
    //    Fail）、挂载点存在（缺失 Fail = setup 未跑/被删）。
    //    reverse（现实→清单）：stale 条目（指向已消失 profile）= Warn + prune 提示；
    //    sweep 残留（§8 检查组，票 13）见第 8 节。诚实边界：不扫全盘，校验域 =
    //    config 可推导路径 ∪ state_dir ∪ /proc。
    let gateway = profile.gateway();
    let bin_name = gateway.bin_name();
    let entries: Vec<crate::manifest::Entry> = match sys.manifest_state() {
        Err(e) => {
            out.push(Check::new(
                "manifest/state",
                Status::Fail,
                format!("清单不可读（fail-loud）：{e}"),
            ));
            out.push(Check::new(
                "provider/selected",
                Status::Fail,
                format!("清单不可读，无法钉定 {bin_name}（fail-loud #2 清单态）：重跑 `iso-cc setup`"),
            ));
            Vec::new()
        }
        Ok(None) => {
            out.push(Check::new(
                "manifest/state",
                Status::Fail,
                format!(
                    "清单缺失（{}；setup 未跑）——先运行 `iso-cc setup`",
                    crate::manifest::manifest_path().display()
                ),
            ));
            out.push(Check::new(
                "provider/selected",
                Status::Fail,
                format!("net.gateway={gateway}：{bin_name} 绝对路径未钉定（fail-loud #2 清单态）——先运行 `iso-cc setup`；缺失报包名：passt / slirp4netns 或上游静态单文件"),
            ));
            Vec::new()
        }
        Ok(Some(entries)) => {
            out.push(Check::new(
                "manifest/state",
                Status::Ok,
                format!("{} entries", entries.len()),
            ));
            // forward：provider 条目 → 钉定路径可执行 + 版本一致
            match entries.iter().find(|e| {
                e.kind == crate::manifest::EntryKind::Provider && e.key == bin_name
            }) {
                None => out.push(Check::new(
                    "provider/selected",
                    Status::Fail,
                    format!("net.gateway={gateway}：清单无 {bin_name} 条目（fail-loud #2 清单态）——先运行 `iso-cc setup --profile <n>`"),
                )),
                Some(entry) => {
                    let path = entry.path.clone().unwrap_or_default();
                    let executable = sys.path_executable(&path);
                    if !executable {
                        out.push(Check::new(
                            "provider/selected",
                            Status::Fail,
                            format!("net.gateway={gateway}：钉定路径不可执行 {}（fail-loud #2 清单态）——重跑 `iso-cc setup`", path.display()),
                        ));
                    } else {
                        let current = sys.provider_version(&path);
                        let drifted = current.as_deref().is_some_and(|c| Some(c) != entry.version.as_deref());
                        let status = if current.is_none() || drifted {
                            Status::Warn
                        } else {
                            Status::Ok
                        };
                        let detail = match (&current, &entry.version) {
                            (Some(c), Some(v)) if c != v => format!(
                                "net.gateway={gateway} → {} 版本漂移：清单 {v:?} vs 现实 {c:?}（重跑 setup 收敛）",
                                path.display()
                            ),
                            (None, _) => format!(
                                "net.gateway={gateway} → {} 版本不可测（--version 失败）",
                                path.display()
                            ),
                            _ => format!(
                                "net.gateway={gateway} → {}（清单钉定，版本一致）",
                                path.display()
                            ),
                        };
                        out.push(Check::new("provider/selected", status, detail));
                    }
                }
            }
            // forward：挂载点条目存在性（缺失 Fail = setup 未跑或被删）
            let missing: Vec<String> = entries
                .iter()
                .filter(|e| e.kind == crate::manifest::EntryKind::Mountpoint)
                .filter(|e| e.path.as_ref().is_some_and(|p| !p.exists()))
                .map(|e| e.key.clone())
                .collect();
            let mp_total = entries
                .iter()
                .filter(|e| e.kind == crate::manifest::EntryKind::Mountpoint)
                .count();
            if missing.is_empty() {
                out.push(Check::new(
                    "manifest/mountpoints",
                    Status::Ok,
                    format!("{mp_total} 挂载点条目全部在位"),
                ));
            } else {
                out.push(Check::new(
                    "manifest/mountpoints",
                    Status::Fail,
                    format!(
                        "挂载点消失 ×{}：{}（setup 未跑或被删——重跑 `iso-cc setup`）",
                        missing.len(),
                        missing.join(", ")
                    ),
                ));
            }
            // reverse：stale 条目（指向已消失 profile）= Warn + prune 提示
            let stale: Vec<String> = entries
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        crate::manifest::EntryKind::Mountpoint
                            | crate::manifest::EntryKind::ProfileState
                    )
                })
                .filter_map(|e| {
                    e.key.split_once(':').map(|(p, _)| p).and_then(|p| {
                        (!_cfg.profile.contains_key(p)).then(|| e.key.clone())
                    })
                })
                .collect();
            if stale.is_empty() {
                out.push(Check::new(
                    "manifest/stale",
                    Status::Ok,
                    "无 stale 条目",
                ));
            } else {
                out.push(Check::new(
                    "manifest/stale",
                    Status::Warn,
                    format!(
                        "指向已消失 profile 的条目 ×{}：{}（`iso-cc gc --prune` 清理登记簿）",
                        stale.len(),
                        stale.join(", ")
                    ),
                ));
            }
            entries
        }
    };
    // #11：slirp 模式 DNS 上游=宿主 resolver（内建转发器）→ R1 降级恒 Warn（P3b/P13 现形）
    if gateway == NetGateway::Slirp4netns {
        out.push(Check::new(
            "provider/dns",
            Status::Warn,
            "slirp4netns 模式 DNS 上游=宿主 resolver（内建转发，R1 降级）",
        ));
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

    // 7. redirect 挂载点登记核验（config 可推导 → 清单；票 14 run 零预创建的镜像断言）：
    //    声明的 redirect dst 缺失且未登记 = setup 未跑，run 将 fail-loud → 这里 Fail。
    {
        let unregistered_missing: Vec<String> = profile
            .redirect
            .iter()
            .filter_map(|r| r.split_once('=').map(|(_, dst)| dst.to_string()))
            .filter(|dst| !Path::new(dst).exists())
            .filter(|dst| {
                !entries.iter().any(|e| {
                    e.kind == crate::manifest::EntryKind::Mountpoint
                        && e.path.as_ref().is_some_and(|p| p.to_string_lossy() == *dst)
                })
            })
            .collect();
        if unregistered_missing.is_empty() {
            out.push(Check::new(
                "manifest/redirect-registered",
                Status::Ok,
                format!("redirect {} 条（挂载点缺失 = 0 或已登记）", profile.redirect.len()),
            ));
        } else {
            out.push(Check::new(
                "manifest/redirect-registered",
                Status::Fail,
                format!(
                    "redirect 挂载点缺失且未登记 ×{}：{}（run 将 fail-loud）——先运行 `iso-cc setup`",
                    unregistered_missing.len(),
                    unregistered_missing.join(", ")
                ),
            ));
        }
    }

    // 8. lifecycle sweep（票 13 L3，fail-loud #10）：孤儿网关（pasta=argv 键 /
    //    slirp=env 键）+ 无主会话目录。现役 residue = Fail；清除动作归 gc（票 14）。
    let residue = sys.lifecycle_residue();
    if residue.gateways.is_empty() {
        out.push(Check::new(
            "sweep/orphan-gateways",
            Status::Ok,
            "无孤儿网关（pasta=argv 指纹 / slirp=env 标记，L3 sweep）",
        ));
    } else {
        let det = residue
            .gateways
            .iter()
            .map(|g| {
                let sid = g.session_id.as_deref().unwrap_or("?");
                format!(
                    "pid={} kind={} session={sid} cmd={:?}",
                    g.pid, g.kind, g.cmdline
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        out.push(Check::new(
            "sweep/orphan-gateways",
            Status::Fail,
            format!(
                "孤儿网关 ×{}：{det}（清除动作归 gc，票 14）",
                residue.gateways.len()
            ),
        ));
    }
    if residue.dirs.is_empty() {
        out.push(Check::new(
            "sweep/orphan-dirs",
            Status::Ok,
            "无无主会话目录（sessions/* 均有活跃会话归属或已回收）",
        ));
    } else {
        out.push(Check::new(
            "sweep/orphan-dirs",
            Status::Fail,
            format!(
                "无主会话目录 ×{}：{}（清除动作归 gc，票 14）",
                residue.dirs.len(),
                residue.dirs.join(", ")
            ),
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

    fn mentry(key: &str, path: &str, version: Option<&str>) -> crate::manifest::Entry {
        crate::manifest::Entry {
            kind: if key.contains(':') {
                crate::manifest::EntryKind::Mountpoint
            } else {
                crate::manifest::EntryKind::Provider
            },
            key: key.into(),
            path: Some(PathBuf::from(path)),
            version: version.map(Into::into),
            registered_at: 1_760_000_000_000,
            reason: "测试登记".into(),
        }
    }

    fn default_manifest() -> Vec<crate::manifest::Entry> {
        vec![mentry("pasta", "/usr/bin/fake", Some("1.0.0"))]
    }

    /// 最小 FakeSys：全部通过的基线，单测按需覆写。
    struct FakeSys {
        max_ns: String,
        clone_switch: Option<String>,
        iface: bool,
        default_route: bool,
        residue: LifecycleResidue,
        manifest: Result<Option<Vec<crate::manifest::Entry>>, String>,
        exec_ok: bool,
    }

    impl Default for FakeSys {
        fn default() -> Self {
            Self {
                max_ns: "1024".into(),
                clone_switch: None,
                iface: true,
                default_route: true,
                residue: LifecycleResidue::default(),
                manifest: Ok(Some(default_manifest())),
                exec_ok: true,
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
        fn path_executable(&self, _path: &Path) -> bool {
            self.exec_ok
        }
        fn which(&self, _bin: &str) -> Option<PathBuf> {
            Some(PathBuf::from("/usr/bin/fake"))
        }
        fn command_output(&self, _cmd: &str) -> Option<String> {
            Some("1.0.0".into())
        }
        fn provider_version(&self, _path: &Path) -> Option<String> {
            Some("1.0.0".into())
        }
        fn locale_available(&self, _lang: &str) -> bool {
            true
        }
        fn resolve_connect(&self, _host: &str, _port: u16) -> Result<(), String> {
            Ok(())
        }
        fn lifecycle_residue(&self) -> LifecycleResidue {
            self.residue.clone()
        }
        fn manifest_state(&self) -> Result<Option<Vec<crate::manifest::Entry>>, String> {
            self.manifest.clone()
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
            fn path_executable(&self, _: &Path) -> bool {
                true
            }
            fn which(&self, _: &str) -> Option<PathBuf> {
                Some(PathBuf::from("/x"))
            }
            fn command_output(&self, _: &str) -> Option<String> {
                None
            }
            fn provider_version(&self, _: &Path) -> Option<String> {
                None
            }
            fn locale_available(&self, _: &str) -> bool {
                true
            }
            fn resolve_connect(&self, _: &str, _: u16) -> Result<(), String> {
                Ok(())
            }
            fn manifest_state(&self) -> Result<Option<Vec<crate::manifest::Entry>>, String> {
                Ok(Some(default_manifest()))
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

    #[test]
    fn sweep_residue_fails_doctor() {
        let sys = FakeSys {
            residue: LifecycleResidue {
                gateways: vec![crate::list::GatewayResidue {
                    pid: 4242,
                    kind: "pasta".into(),
                    session_id: None,
                    cmdline: "bash -c sleep 299 x --outbound-if4 eth0".into(),
                }],
                dirs: vec!["crash-9".into()],
            },
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let gw = checks
            .iter()
            .find(|c| c.name == "sweep/orphan-gateways")
            .expect("sweep/orphan-gateways 检查存在");
        assert_eq!(gw.status, Status::Fail);
        assert!(gw.detail.contains("4242"), "{}", gw.detail);
        assert!(gw.detail.contains("--outbound-if4"), "{}", gw.detail);
        let dirs = checks
            .iter()
            .find(|c| c.name == "sweep/orphan-dirs")
            .expect("sweep/orphan-dirs 检查存在");
        assert_eq!(dirs.status, Status::Fail);
        assert!(dirs.detail.contains("crash-9"), "{}", dirs.detail);
        assert!(any_fail(&checks));
    }

    #[test]
    fn sweep_clean_is_ok() {
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
        assert!(checks
            .iter()
            .any(|c| c.name == "sweep/orphan-gateways" && c.status == Status::Ok));
        assert!(checks
            .iter()
            .any(|c| c.name == "sweep/orphan-dirs" && c.status == Status::Ok));
    }

    #[test]
    fn slirp_gateway_selection_and_dns_warn() {
        let p: Profile = toml::from_str("egress = 'if:wg0'\nnet.gateway = 'slirp4netns'").unwrap();
        let sys = FakeSys {
            manifest: Ok(Some(vec![mentry(
                "slirp4netns",
                "/usr/bin/fake",
                Some("1.0.0"),
            )])),
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
        assert!(!any_fail(&checks), "{checks:?}");
        assert!(checks
            .iter()
            .any(|c| c.name == "provider/selected" && c.detail.contains("slirp4netns")));
        assert!(checks
            .iter()
            .any(|c| c.name == "provider/dns" && c.status == Status::Warn));
    }

    #[test]
    fn missing_selected_provider_is_fail() {
        // 清单态 #2：provider 条目缺失 = Fail，提示 setup
        let sys = FakeSys {
            manifest: Ok(Some(Vec::new())),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        assert!(checks.iter().any(|c| c.name == "provider/selected"
            && c.status == Status::Fail
            && c.detail.contains("#2")
            && c.detail.contains("setup")));
    }

    #[test]
    fn manifest_missing_fails_with_setup_hint() {
        let sys = FakeSys {
            manifest: Ok(None),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        assert!(checks.iter().any(|c| c.name == "manifest/state" && c.status == Status::Fail));
        assert!(checks.iter().any(|c| c.name == "provider/selected" && c.status == Status::Fail));
    }

    #[test]
    fn provider_version_drift_warns() {
        let sys = FakeSys {
            manifest: Ok(Some(vec![mentry("pasta", "/usr/bin/fake", Some("0.0.9-drift"))])),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let c = checks
            .iter()
            .find(|c| c.name == "provider/selected")
            .expect("provider/selected 存在");
        assert_eq!(c.status, Status::Warn, "{}", c.detail);
        assert!(c.detail.contains("版本漂移"), "{}", c.detail);
        assert!(!any_fail(&checks));
    }

    #[test]
    fn stale_entry_warns_with_prune_hint() {
        let mut m = default_manifest();
        m.push(mentry("gone:/tmp", "/tmp", None));
        let sys = FakeSys {
            manifest: Ok(Some(m)),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let c = checks
            .iter()
            .find(|c| c.name == "manifest/stale")
            .expect("manifest/stale 存在");
        assert_eq!(c.status, Status::Warn, "{}", c.detail);
        assert!(c.detail.contains("gone:/tmp"), "{}", c.detail);
        assert!(c.detail.contains("gc --prune"), "{}", c.detail);
        assert!(!any_fail(&checks));
    }

    #[test]
    fn vanished_mountpoint_entry_fails() {
        let mut m = default_manifest();
        m.push(mentry("web:/nonexistent/mp", "/nonexistent/mp", None));
        let sys = FakeSys {
            manifest: Ok(Some(m)),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let c = checks
            .iter()
            .find(|c| c.name == "manifest/mountpoints")
            .expect("manifest/mountpoints 存在");
        assert_eq!(c.status, Status::Fail, "{}", c.detail);
        assert!(c.detail.contains("web:/nonexistent/mp"), "{}", c.detail);
    }

    #[test]
    fn unregistered_missing_redirect_fails() {
        let p: Profile =
            toml::from_str("egress = 'if:wg0'\nredirect = ['/tmp/src-x=/nonexistent/dst-x']").unwrap();
        let sys = FakeSys {
            manifest: Ok(Some(Vec::new())),
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
        let c = checks
            .iter()
            .find(|c| c.name == "manifest/redirect-registered")
            .expect("manifest/redirect-registered 存在");
        assert_eq!(c.status, Status::Fail, "{}", c.detail);
        assert!(c.detail.contains("/nonexistent/dst-x"), "{}", c.detail);
    }

    #[test]
    fn registered_existing_redirect_is_ok() {
        // 已登记且在位 → Ok（setup 收敛后的稳态）
        let mut m = default_manifest();
        m.push(mentry("x:/registered/dst", "/registered/dst", None));
        let sys = FakeSys {
            manifest: Ok(Some(m)),
            ..Default::default()
        };
        let p: Profile =
            toml::from_str("egress = 'if:wg0'\nredirect = ['/tmp/src-x=/registered/dst']").unwrap();
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let c = checks
            .iter()
            .find(|c| c.name == "manifest/redirect-registered")
            .expect("manifest/redirect-registered 存在");
        assert_eq!(c.status, Status::Ok, "{}", c.detail);
    }

    #[test]
    fn manifest_state_corrupt_fails() {
        let sys = FakeSys {
            manifest: Err("schema = 99 不支持".into()),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        assert!(checks.iter().any(|c| c.name == "manifest/state" && c.status == Status::Fail));
        assert!(checks.iter().any(|c| c.name == "provider/selected" && c.status == Status::Fail));
    }

    #[test]
    fn dead_pinned_provider_path_fails() {
        // 钉定路径可执行位缺失 = Fail（#2 清单态）
        let sys = FakeSys {
            exec_ok: false,
            manifest: Ok(Some(vec![mentry("pasta", "/nonexistent/bin/pasta", Some("1.0.0"))])),
            ..Default::default()
        };
        let p = profile("if:wg0");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &sys,
        );
        let c = checks
            .iter()
            .find(|c| c.name == "provider/selected")
            .expect("provider/selected 存在");
        assert_eq!(c.status, Status::Fail, "{}", c.detail);
        assert!(c.detail.contains("不可执行"), "{}", c.detail);
    }

    #[test]
    fn lo_egress_rejected_by_ns_ifname_rule() {
        let p = profile("if:lo");
        let checks = run(
            &Config {
                version: 1,
                profile: Default::default(),
            },
            "x",
            &p,
            &FakeSys::default(),
        );
        assert!(any_fail(&checks), "{checks:?}");
        assert!(checks
            .iter()
            .any(|c| c.name == "egress/iface" && c.detail.contains("#12")));
    }
}
