use crate::config::{NetIpv6, Profile};
use anyhow::{anyhow, bail, Context};
use std::ffi::{CString, OsString};
use std::os::unix::{ffi::OsStrExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const CLONE_NEWNS: libc::c_int = 0x0002_0000;
const CLONE_NEWNET: libc::c_int = 0x4000_0000;
const CLONE_NEWUSER: libc::c_int = 0x1000_0000;

/// 一个存活会话：cc 子进程 + slirp4netns 网关。
/// 二者均挂 PDEATHSIG(SIGKILL)——父进程死 = 全家清场（N3）。
pub struct Session {
    pub child: Child,
    pub gateway: Child,
}

pub enum ChildMode {
    /// 执行 agent 命令行（首元素为程序，其余为参数）
    Exec(Vec<OsString>),
    /// 内嵌纯净度探针（`--probe-json`）
    Probe,
}

/// 会话入口：mountns/locale bind/netns + 网关 attach + exec。
pub fn spawn(profile_name: &str, profile: &Profile, mode: ChildMode) -> anyhow::Result<Session> {
    let state_dir = state_dir(profile_name)?;
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("创建状态目录 {}", state_dir.display()))?;

    // locale 资产：tzdb 内嵌 TZif（宿主无 tzdata 也成立），写状态目录供 bind。
    let mut binds: Vec<(PathBuf, String)> = Vec::new();
    if let Some(tz) = &profile.locale.tz {
        let raw = tzdb::raw_tz_by_name(tz).ok_or_else(|| anyhow!("tzdb 中无 {tz} 的 TZif 数据"))?;
        let p = state_dir.join("localtime");
        std::fs::write(&p, raw).with_context(|| format!("写入 {}", p.display()))?;
        binds.push((p, "/etc/localtime".to_string()));
        let p = state_dir.join("timezone");
        std::fs::write(&p, format!("{tz}\n"))?;
        binds.push((p, "/etc/timezone".to_string()));
    }
    let resolv = state_dir.join("resolv.conf");
    // DNS：slirp 内建转发器 10.0.2.3（上游=宿主 resolver；v0.2 已知边界，SOCKS provider 后收紧）
    std::fs::write(&resolv, "nameserver 10.0.2.3\n")?;
    binds.push((resolv, "/etc/resolv.conf".to_string()));
    for r in &profile.redirect {
        let (src, dst) = r
            .split_once('=')
            .ok_or_else(|| anyhow!("redirect 必须是 `src=dst`：{r:?}"))?;
        binds.push((PathBuf::from(src), dst.to_string()));
    }
    // 挂载点缺失预创建：宿主侧以真实 uid 执行（子进程 uid 未映射时 O_CREAT 会 EACCES）。
    // 残留 = N3 有界例外（doctor 报告）。
    binds.retain(|(_, dst)| {
        if !Path::new(dst).exists() {
            match std::fs::write(dst, b"") {
                Ok(()) => true,
                Err(e) => {
                    eprintln!(
                        "[iso-cc] note: 挂载点 {dst} 缺失且不可预创建（{e}）——跳过该 bind（TZ env 已覆盖主要视线）"
                    );
                    false
                }
            }
        } else {
            true
        }
    });

    let session_id = format!(
        "{profile_name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u32
    );

    let egress_iface = profile.egress_iface()?.to_string();
    let mut cmd = match &mode {
        ChildMode::Exec(cmdline) => {
            let mut args = vec![
                OsString::from("--egress-iface"),
                OsString::from(egress_iface.as_str()),
                OsString::from("--"),
            ];
            args.extend(cmdline.iter().cloned());
            bootstrap_cmd(&args)
        }
        ChildMode::Probe => {
            let mut args = vec![
                OsString::from("--egress-iface"),
                OsString::from(egress_iface.as_str()),
                OsString::from("--"),
                std::env::current_exe()
                    .expect("current_exe 不可用")
                    .into_os_string(),
                OsString::from("probe-json"),
            ];
            if let Some(tz) = &profile.locale.tz {
                args.push(OsString::from("--expect-tz"));
                args.push(tz.clone().into());
            }
            bootstrap_cmd(&args)
        }
    };
    apply_env(&mut cmd, profile, &session_id);

    let bind_list = binds.clone();
    let ipv6_off = profile.ipv6() == NetIpv6::Off;
    unsafe {
        cmd.pre_exec(move || {
            macro_rules! ck {
                ($e:expr) => {
                    if ($e) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                };
            }
            ck!(libc::unshare(CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWNET));
            ck!(libc::mount(
                std::ptr::null(),
                c"/".as_ptr(),
                std::ptr::null(),
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null()
            ));
            for (src, dst) in &bind_list {
                let s = CString::new(src.as_os_str().as_bytes())?;
                let d = CString::new(dst.as_bytes())?;
                ck!(libc::mount(
                    s.as_ptr(),
                    d.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND,
                    std::ptr::null()
                ));
                ck!(libc::mount(
                    std::ptr::null(),
                    d.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
                    std::ptr::null()
                ));
            }
            if ipv6_off {
                sh(&["sysctl", "-w", "net.ipv6.conf.all.disable_ipv6=1"]);
                sh(&["sysctl", "-w", "net.ipv6.conf.default.disable_ipv6=1"]);
            }
            // 父进程死 → 会话进程树全灭（零残留）
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }

    let child = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn 会话子进程")?;
    let pid = child.id();

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    std::fs::write(format!("/proc/{pid}/setgroups"), "deny")
        .with_context(|| format!("写 /proc/{pid}/setgroups"))?;
    std::fs::write(format!("/proc/{pid}/uid_map"), format!("0 {uid} 1"))
        .with_context(|| format!("写 /proc/{pid}/uid_map"))?;
    std::fs::write(format!("/proc/{pid}/gid_map"), format!("0 {gid} 1"))
        .with_context(|| format!("写 /proc/{pid}/gid_map"))?;
    // 网关：slirp4netns 在宿主 netns 侧收发，tap0 送进子 netns
    let gateway_log =
        std::fs::File::create(state_dir.join("gateway.log")).context("创建 gateway.log")?;
    let gateway = Command::new(gateway_path()?)
        .arg(pid.to_string())
        .arg("tap0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(gateway_log))
        .spawn()
        .context("spawn slirp4netns 网关")?;

    eprintln!(
        "[iso-cc] session {session_id}: pid={pid} gateway={} scope={:?} egress-iface={egress_iface}",
        gateway.id(),
        profile.scope()
    );

    Ok(Session { child, gateway })
}

fn bootstrap_cmd(args: &[OsString]) -> Command {
    let mut c = Command::new(std::env::current_exe().expect("current_exe 不可用"));
    c.arg("session-bootstrap");
    for a in args {
        c.arg(a);
    }
    c
}

/// 会话内引导：等网关 tap0 就绪 → 配网 → exec 真实命令（runc 模式）。
pub fn bootstrap(_egress_iface: &str, command: &[OsString]) -> anyhow::Result<()> {
    let prog = command
        .first()
        .ok_or_else(|| anyhow!("bootstrap 缺少命令"))?;
    sh(&["ip", "link", "set", "lo", "up"]);
    let mut ready = false;
    for _ in 0..150 {
        if sh_ok(&["ip", "link", "show", "tap0"]) {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !ready {
        bail!("等待 tap0 超时（网关未就绪）");
    }
    sh(&["ip", "link", "set", "tap0", "up"]);
    sh(&["ip", "addr", "add", "10.0.2.100/24", "dev", "tap0"]);
    sh(&[
        "ip", "route", "add", "default", "via", "10.0.2.2", "dev", "tap0",
    ]);
    let mut c = Command::new(prog);
    for a in &command[1..] {
        c.arg(a);
    }
    let err = c.exec();
    Err(anyhow!("exec 失败: {err}"))
}

impl Session {
    pub fn wait(mut self) -> anyhow::Result<std::process::ExitStatus> {
        let status = self.child.wait().context("等待会话子进程")?;
        // netns 消亡后网关应自行退出；兜底显式收割，防止拖住父进程
        if self.gateway.try_wait()?.is_none() {
            let _ = self.gateway.kill();
            let _ = self.gateway.wait();
        }
        Ok(status)
    }
}

pub fn state_dir(profile_name: &str) -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    Ok(Path::new(&home)
        .join(".local/state/iso-cc")
        .join(profile_name))
}

fn gateway_path() -> anyhow::Result<PathBuf> {
    which("slirp4netns").ok_or_else(|| anyhow!("slirp4netns 不在 PATH（doctor 应已检出）"))
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|cand| cand.is_file())
}

fn apply_env(cmd: &mut Command, profile: &Profile, session_id: &str) {
    cmd.env("ISO_CC_SESSION", session_id);
    if let Some(tz) = &profile.locale.tz {
        cmd.env("TZ", tz);
    }
    if let Some(lang) = &profile.locale.lang {
        cmd.env("LANG", lang);
        cmd.env("LC_ALL", lang);
    }
    for (k, v) in &profile.env {
        cmd.env(k, v);
    }
    // 防宿主 CLAUDE_CONFIG_DIR 泄漏进会话（R4）
    cmd.env_remove("CLAUDE_CONFIG_DIR");
}

fn sh(argv: &[&str]) {
    let _ = Command::new(argv[0]).args(&argv[1..]).output();
}

fn sh_ok(argv: &[&str]) -> bool {
    Command::new(argv[0])
        .args(&argv[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn cs(s: &str) -> CString {
    CString::new(s).expect("NUL")
}
