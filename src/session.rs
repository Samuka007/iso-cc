use crate::config::{NetGateway, NetIpv6, Profile};
use crate::list;
use crate::netcfg;
use crate::ns;
use crate::provider;
use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// bootstrap plan（设计稿 §1.4 参数面）：同二进制 serde 往返 + `deny_unknown_fields`
/// （#5 fail-loud）；argv 传递 = 子进程交接零状态（无临时文件）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapPlan {
    pub mode: BootstrapMode,
    pub ipv6_off: bool,
    pub binds: Vec<(String, String)>,
    pub iface: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapMode {
    /// pasta spawn 之下：pasta 已建 userns/netns 并内建映射（E2），bootstrap 仅自建 mountns。
    Mountns,
    /// slirp4netns 回退：bootstrap 自建三 ns + 单条自映射（父侧 pre_exec 注入）。
    Selfmap,
}

/// 一个存活会话。`root` = 会话根：pasta（spawn 模式，gateway=pasta）或 bootstrap（slirp 回退）。
/// PDEATHSIG 链（设计稿 §1.3/§4-L1）：iso-cc←pasta（pre_exec 对账）；
/// pasta 模式下 bootstrap 自挂 PDEATHSIG→pasta（in-band 结构对账）；
/// slirp 模式下 bootstrap 由父侧 pre_exec 挂 PDEATHSIG→iso-cc（精确对账）。
pub struct Session {
    pub root: Child,
    /// attach 型网关（仅 slirp 回退）；pasta 即根时为 None。
    pub gateway: Option<Child>,
    /// 会话资产目录（正常退出后回收；残留交 doctor sweep 上报）。
    pub sess_dir: PathBuf,
}

// ===== 生命周期三层防线（票 13；设计稿 design-session-lanes §4）=====
//
// unsafe 纪律：PR_SET_CHILD_SUBREAPER / kill(2) / waitpid(2) 三个生命周期内核原语
// 在此以 libc 裸调用 + SAFETY 注释落地（票 13 契约文件集不含 ns.rs 与 Cargo.toml，
// nix "signal" feature 不可引入）；每个 unsafe 块只包单次 syscall。

/// L2 执行者（设计稿 §4-L2）：iso-cc 自设 PR_SET_CHILD_SUBREAPER(1)——会话树中任何
/// 进程的父链断掉时 reparent 到 iso-cc 而非 init，[`Session::wait`] 的收编循环得以
/// 直杀（CompScan #6：subreaper 与 PDEATHSIG 是配套机制；轮子盘点：内核原语，
/// 拒绝 systemd-run --scope 与第三方看护 crate）。
fn set_child_subreaper() -> anyhow::Result<()> {
    // SAFETY: prctl(2) 单参数变体，作用于 iso-cc 主进程自身，无 fork/exec 上下文
    // 前置条件；失败仅 EINVAL（参数非法，此处不可能）。
    let rc = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    if rc != 0 {
        return Err(anyhow!(
            "PR_SET_CHILD_SUBREAPER 失败: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// spawn 后 `?` 早退路径的泄漏面兜底（audit-facts §3/§6「spawn 错误路径孤儿」）：
/// arm 到 disarm 之间任何 `?` 早退，Drop 侧 SIGKILL + waitpid 收尸。
struct KillGuard {
    pid: Option<u32>,
}

impl KillGuard {
    fn arm(pid: u32) -> Self {
        Self { pid: Some(pid) }
    }

    /// 成功路径：pid 所有权移交 Session，guard 析构为 no-op。
    fn disarm(mut self) -> u32 {
        self.pid.take().expect("KillGuard 双重 disarm")
    }
}

impl Drop for KillGuard {
    fn drop(&mut self) {
        let Some(pid) = self.pid.take() else { return };
        // SAFETY: kill(2)/waitpid(2) 作用于本进程刚 spawn 的直接子女（spawn 返回值到
        // Session 构造之间的窗口）；SIGKILL 不可捕获，waitpid 收尸防僵尸。子进程已自行
        // 退出并被 try_wait reap → kill 得 ESRCH、waitpid 得 ECHILD，均忽略。
        let killed = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0;
        unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0) };
        if killed {
            eprintln!("[iso-cc] KillGuard: spawn 错误路径击杀泄漏子进程 pid={pid}");
        }
    }
}

pub enum ChildMode {
    /// 执行 agent 命令行（首元素为程序，其余为参数）
    Exec(Vec<OsString>),
    /// 内嵌纯净度探针（`--probe-json`）
    Probe,
}

/// 会话入口：egress fail-loud 断言 → 会话资产（sessions/<id>/）→ 网关 provider 化 spawn。
pub fn spawn(profile_name: &str, profile: &Profile, mode: ChildMode) -> anyhow::Result<Session> {
    let egress_iface = profile.egress_iface()?.to_string();
    // fail-loud #12：`-I` 撞名类 egress（lo/tap0）直接拒绝
    provider::validate_egress_iface(&egress_iface)?;
    // fail-loud #3：宿主 egress 接口存在且 UP（sysfs，spawn 前断言；R8 绝不回落）
    provider::host_iface_up(&egress_iface)?;
    // L2（设计稿 §4-L2）：会话树建立前自设 subreaper——收编循环的 reparent 前提。
    set_child_subreaper()?;

    let gateway = profile.gateway();
    let dns = profile.dns();

    let session_id = format!(
        "{profile_name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u32
    );
    // 会话资产移 sessions/<id>/（audit-facts §7 并发互踩修复）：localtime/timezone/
    // resolv.conf/gateway.log 全部逐会话独立，会话结束后的回收归 13 sweep。
    let sess_dir = session_dir(&session_id)?;
    std::fs::create_dir_all(&sess_dir)
        .with_context(|| format!("创建会话资产目录 {}", sess_dir.display()))?;

    // locale 资产：tzdb 内嵌 TZif（宿主无 tzdata 也成立），写会话目录供 bind。
    let mut binds: Vec<(PathBuf, String)> = Vec::new();
    if let Some(tz) = &profile.locale.tz {
        let raw = tzdb::raw_tz_by_name(tz).ok_or_else(|| anyhow!("tzdb 中无 {tz} 的 TZif 数据"))?;
        let p = sess_dir.join("localtime");
        std::fs::write(&p, raw).with_context(|| format!("写入 {}", p.display()))?;
        binds.push((p, "/etc/localtime".to_string()));
        let p = sess_dir.join("timezone");
        std::fs::write(&p, format!("{tz}\n"))?;
        binds.push((p, "/etc/timezone".to_string()));
    }
    let resolv = sess_dir.join("resolv.conf");
    // DNS：net.dns（默认 10.0.2.3）= pasta --dns-forward 地址 = slirp 内建转发器同址
    std::fs::write(&resolv, format!("nameserver {dns}\n"))?;
    binds.push((resolv, "/etc/resolv.conf".to_string()));
    for r in &profile.redirect {
        let (src, dst) = r
            .split_once('=')
            .ok_or_else(|| anyhow!("redirect 必须是 `src=dst`：{r:?}"))?;
        binds.push((PathBuf::from(src), dst.to_string()));
    }
    // 挂载点缺失预创建：宿主侧以真实 uid 执行（子进程 uid 未映射时 O_CREAT 会 EACCES）。
    // 残留 = N3 有界例外（doctor 报告；setup-manifested 迁移归 14）。
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

    // bootstrap plan（§1.4）：mode 位承载 provider 差异（§1.2 被否双模式并存的收敛点）
    let plan = BootstrapPlan {
        mode: match gateway {
            NetGateway::Pasta => BootstrapMode::Mountns,
            NetGateway::Slirp4netns => BootstrapMode::Selfmap,
        },
        ipv6_off: profile.ipv6() == NetIpv6::Off,
        binds: binds
            .iter()
            .map(|(s, d)| (s.to_string_lossy().into_owned(), d.clone()))
            .collect(),
        iface: provider::NS_IFNAME.to_string(),
        timeout_ms: 15_000,
    };

    let inner: Vec<OsString> = match &mode {
        ChildMode::Exec(cmdline) => cmdline.clone(),
        ChildMode::Probe => {
            let mut v = vec![
                std::env::current_exe()
                    .expect("current_exe 不可用")
                    .into_os_string(),
                OsString::from("probe-json"),
            ];
            if let Some(tz) = &profile.locale.tz {
                v.push(OsString::from("--expect-tz"));
                v.push(tz.clone().into());
            }
            v
        }
    };

    match gateway {
        NetGateway::Pasta => spawn_pasta(
            profile,
            &session_id,
            &sess_dir,
            &egress_iface,
            dns,
            &plan,
            inner,
        ),
        NetGateway::Slirp4netns => spawn_slirp(profile, &session_id, &sess_dir, &plan, inner),
    }
}

/// gateway=pasta（primary，设计稿 §1.3 上树）：pasta 即会话根，bootstrap 由 pasta 在
/// userns/netns 内拉起。R8 由内核结构性执行：pasta 死 → bootstrap/cc 经 PDEATHSIG 链即灭。
fn spawn_pasta(
    profile: &Profile,
    session_id: &str,
    sess_dir: &Path,
    egress_iface: &str,
    dns: Ipv4Addr,
    plan: &BootstrapPlan,
    inner: Vec<OsString>,
) -> anyhow::Result<Session> {
    let pasta_bin = provider::gateway_bin(NetGateway::Pasta)?;
    let plan_json = serde_json::to_string(plan).context("序列化 bootstrap plan")?;
    let mut bs_args: Vec<OsString> = vec![
        std::env::current_exe()
            .expect("current_exe 不可用")
            .into_os_string(),
        OsString::from("session-bootstrap"),
        OsString::from("--plan"),
        plan_json.into(),
        OsString::from("--session-id"),
        OsString::from(session_id),
        OsString::from("--"),
    ];
    bs_args.extend(inner);

    let log_file = sess_dir.join("gateway.log");
    let mut gw = Command::new(&pasta_bin);
    for a in provider::pasta::flag_args(egress_iface, dns, log_file.as_os_str()) {
        gw.arg(a);
    }
    gw.arg("--");
    for a in &bs_args {
        gw.arg(a);
    }
    // 双标记之一：pasta 自身带 ISO_CC_SESSION（list/sweep 对网关的识别键，§4-L3）
    gw.env("ISO_CC_SESSION", session_id);
    // TZ/LANG/env 与 CLAUDE_CONFIG_DIR 摘除经 pasta env 链传至 bootstrap/cc
    apply_env(&mut gw, profile);
    // pasta 自身诊断 → -l gateway.log（#4 tail 取证）；stdio 全 inherit：
    // child 与 pasta 共享 stdio（本机取证），probe JSON / cc 交互不得被日志劫持
    gw.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // PDEATHSIG→iso-cc + 精确对账（PR_SET_PDEATHSIG(2const) 竞态②，ns.rs §4-L1）
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut gw, move || {
        ns::set_pdeathsig_verified(libc::SIGKILL, expected_ppid)
    });
    let mut pasta = gw
        .spawn()
        .with_context(|| format!("spawn pasta（{}）", pasta_bin.display()))?;
    // KillGuard（audit-facts §3）：pasta 已 spawn，此后任何 `?` 早退不得遗留存活子进程
    let guard = KillGuard::arm(pasta.id());

    // fail-loud #4：pasta 前台早期退出检测（1s try_wait）+ gateway.log tail。
    // 判别键 = bootstrap 是否已 exec（bootstrapped 标记文件）：pasta 的退出码对 child
    // 透明（退出码取证项），child 瞬时非零完成（如 `sh -c 'exit 7'`）与 pasta 自身故障
    // 在 status 上不可区分；child 已 exec → pasta 退出即透传，交 wait() 正常返回。
    std::thread::sleep(Duration::from_secs(1));
    if let Some(status) = pasta.try_wait().context("try_wait pasta（早期退出检测）")? {
        if !sess_dir.join("bootstrapped").exists() {
            let tail = tail_lines(&log_file, 40);
            bail!(
                "pasta 早期退出（status={status}，fail-loud #4）：bootstrap 未曾 exec；{} 末尾：\n{tail}",
                log_file.display()
            );
        }
    }

    eprintln!(
        "[iso-cc] session {session_id}: root=pasta(pid={}) egress-iface={egress_iface} dns={dns} scope={:?} assets={}",
        pasta.id(),
        profile.scope(),
        sess_dir.display()
    );
    guard.disarm();
    Ok(Session {
        root: pasta,
        gateway: None,
        sess_dir: sess_dir.to_path_buf(),
    })
}

/// gateway=slirp4netns（回退，设计稿 §1.3 下树）：bootstrap 自映射进三 ns（selfmap，
/// 父侧 pre_exec），slirp4netns attach；parent 永不写 /proc/<pid>/maps（Facts §3 残留源消灭）。
fn spawn_slirp(
    profile: &Profile,
    session_id: &str,
    sess_dir: &Path,
    plan: &BootstrapPlan,
    inner: Vec<OsString>,
) -> anyhow::Result<Session> {
    let slirp_bin = provider::gateway_bin(NetGateway::Slirp4netns)?;
    let plan_json = serde_json::to_string(plan).context("序列化 bootstrap plan")?;
    let mut bs = Command::new(std::env::current_exe().expect("current_exe 不可用"));
    bs.arg("session-bootstrap")
        .arg("--plan")
        .arg(&plan_json)
        .arg("--session-id")
        .arg(session_id)
        .arg("--");
    for a in &inner {
        bs.arg(a);
    }
    apply_env(&mut bs, profile);

    // pre_exec 自映射（§1.3 下树）：binds 父进程预转换（§2.1），闭包体 = ns.rs 入口 B 单调用
    let binds = ns::cstring_binds(&plan.binds).context("bind 路径预转换")?;
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut bs, move || ns::enter_selfmap_ns(&binds, expected_ppid));
    let root = bs
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn session-bootstrap（selfmap）")?;
    // KillGuard（audit-facts §3）：bootstrap 已 spawn；网关 spawn 失败的 `?` 早退路径
    // 不得遗留已进 ns 的 bootstrap 子进程（旧 :163-170 泄漏面）
    let root_guard = KillGuard::arm(root.id());

    // slirp4netns attach：`<pid> tap0 -c`；双标记之一：slirp 自身带 ISO_CC_SESSION
    let log_file = sess_dir.join("gateway.log");
    let log =
        std::fs::File::create(&log_file).with_context(|| format!("创建 {}", log_file.display()))?;
    let mut gw = Command::new(&slirp_bin);
    for a in provider::slirp::attach_args(root.id()) {
        gw.arg(a);
    }
    gw.env("ISO_CC_SESSION", session_id);
    gw.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    let gateway = gw
        .spawn()
        .with_context(|| format!("spawn slirp4netns（{}）", slirp_bin.display()))?;
    let gw_guard = KillGuard::arm(gateway.id());

    eprintln!(
        "[iso-cc] session {session_id}: root=bootstrap(pid={}) gateway=slirp4netns(pid={}, attach) scope={:?} assets={}",
        root.id(),
        gateway.id(),
        profile.scope(),
        sess_dir.display()
    );
    root_guard.disarm();
    gw_guard.disarm();
    Ok(Session {
        root,
        gateway: Some(gateway),
        sess_dir: sess_dir.to_path_buf(),
    })
}


/// session-bootstrap 主体（§1.4 新参面）：plan 解析（#5 fail-loud）→ 自设会话标记
/// → 按模式进 ns（mountns：pasta 之下由本进程自建；selfmap：父侧 pre_exec 已完成）
/// → 等 tap 就绪 → exec 真实命令（provider self-config，零配网命令，§3.1）。
pub fn bootstrap_run(
    plan_json: &str,
    session_id: &str,
    command: &[OsString],
) -> anyhow::Result<()> {
    let plan: BootstrapPlan = serde_json::from_str(plan_json).with_context(|| {
        format!("bootstrap plan 解析失败（fail-loud #5，deny_unknown_fields）：{plan_json}")
    })?;
    // 会话标记：bootstrap 自设（不依赖「网关是否透传 env」这一未取证行为，§1.4）
    std::env::set_var("ISO_CC_SESSION", session_id);
    match plan.mode {
        BootstrapMode::Mountns => enter_mountns_inband(&plan)?,
        BootstrapMode::Selfmap => {}
    }
    bootstrap_exec(&plan, session_id, command)
}

/// mountns 入口（pasta spawn 路径，§1.3 上树）——原 pre_exec 闭包的机械裁剪搬家：
/// 删 NEWUSER|NEWNET（pasta 已建 userns/netns，E2 内建映射）、删 maps 写段（parent 侧
/// uid_map 写面整体消失）、删 sysctl sh（fail-open 点；v6 直写归 12 的 /proc/sys 直写）。
/// PDEATHSIG→pasta + 结构对账：spawn 模式父=pasta、pid 不可预知，父死则被 reparent 到
/// pid 1 → 自尽；prctl 之后 pasta 死亡由内核信号覆盖。
fn enter_mountns_inband(plan: &BootstrapPlan) -> anyhow::Result<()> {
    let binds = ns::cstring_binds(&plan.binds).context("bind 路径预转换")?;
    // expected_ppid=0 = 结构对账（pasta 路径父 pid 不可预知，ns.rs 入口 A）
    ns::enter_mountns(&binds, 0).map_err(|e| anyhow!("mountns 装配（pasta 之下）失败: {e}"))
}

/// 等网关 tap 就绪（provider 无关）→ exec 真实命令。
/// 两 provider 均 self-config（pasta `--config-net` / slirp `-c`，§3.1）：
/// 原 `ip addr add`/`ip route add`/`ip link set tap0 up` 配网命令整体删除。
/// 票 12：ipv6_off → /proc/sys 直写（§3.3）→ netlink 就绪等待（§3.2）→ exec；
/// 残留的吞错 shell 助手（lo up + tap 轮询）整体废除——bootstrap 期零 execve。
fn bootstrap_exec(
    plan: &BootstrapPlan,
    session_id: &str,
    command: &[OsString],
) -> anyhow::Result<()> {
    let prog = command
        .first()
        .ok_or_else(|| anyhow!("bootstrap 缺少命令"))?;
    // exec 前落 bootstrapped 标记（#4 判别键：pasta 早期退出时区分「child 已 exec」与
    // 「pasta 自身故障」；会话资产，13 sweep 收敛）
    let marker = session_dir(session_id)?.join("bootstrapped");
    std::fs::write(&marker, b"").with_context(|| format!("写 exec 前标记 {}", marker.display()))?;
    // §3.3 时机：v6 直写先于就绪等待；与 provider self-config 同 netns sysctl 并发无害。
    if plan.ipv6_off {
        netcfg::disable_ipv6()?;
    }
    netcfg::wait_ready(&plan.iface, Duration::from_millis(plan.timeout_ms))?;
    let mut c = Command::new(prog);
    for a in &command[1..] {
        c.arg(a);
    }
    let err = c.exec();
    Err(anyhow!("exec 失败: {err}"))
}

impl Session {
    pub fn wait(mut self) -> anyhow::Result<std::process::ExitStatus> {
        let status = self.root.wait().context("等待会话根进程")?;
        // slirp attach 模式：netns 消亡后网关应自行退出；兜底显式收割，防止拖住父进程
        if let Some(gw) = self.gateway.as_mut() {
            if gw.try_wait()?.is_none() {
                let _ = gw.kill();
                let _ = gw.wait();
            }
        }
        // L2 收编循环（设计稿 §4-L2）：PDEATHSIG fork 即清、孙进程不在 L1 覆盖面
        // （audit-facts §6），setsid 逃逸者经 subreaper reparent 到本进程，此处直杀。
        self.reap_adopted();
        // 会话资产回收（spawn() 09 期注释契约「会话结束后的回收归 13 sweep」）：目录内
        // 全部为可再生资产（localtime/timezone/resolv.conf/gateway.log/bootstrapped），
        // R4 持久态（~/.claude）不在会话目录。回收失败交 L3 sweep 上报（doctor）。
        if let Err(e) = std::fs::remove_dir_all(&self.sess_dir) {
            eprintln!(
                "[iso-cc] note: 会话资产目录 {} 回收失败（{e}）——交 doctor sweep（L3）上报",
                self.sess_dir.display()
            );
        }
        Ok(status)
    }

    /// L2 收编（设计稿 §4-L2 + PM 修订双键定界）：收编范围 = 会话根后裔（subreaper
    /// 语义下 reparent 后 = 本进程子女）∩ ISO_CC_SESSION 标记。标记者 SIGKILL + reap；
    /// 无标记者只登记上报、不杀（票 15 host 执行语义预留）。循环至一轮扫描无标记子女
    /// （每轮至少收编一个，进程数有限 → 终止）。
    fn reap_adopted(&mut self) {
        let self_pid = std::process::id();
        loop {
            let mut reaped = false;
            for pid in list::children_of(self_pid) {
                match list::proc_marker(pid) {
                    Some(id) => {
                        // SAFETY: kill(2)/waitpid(2)；pid = reparent 到本进程的收养子女
                        // （subreaper 语义，waitpid 合法）。已死未收尸者 kill 得 ESRCH、
                        // waitpid 直接收尸；存活者 SIGKILL 不可捕获，阻塞收尸。
                        unsafe {
                            libc::kill(pid as libc::pid_t, libc::SIGKILL);
                            libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0);
                        }
                        eprintln!(
                            "[iso-cc] L2 收编: 孤儿 pid={pid}（ISO_CC_SESSION={id}）→ SIGKILL + reaped"
                        );
                        reaped = true;
                    }
                    None => {
                        eprintln!(
                            "[iso-cc] L2 登记: 收养孤儿 pid={pid} 无 ISO_CC_SESSION 标记——不杀（host 执行语义预留，票 15）；无标记即 doctor sweep 域外"
                        );
                    }
                }
            }
            if !reaped {
                return;
            }
        }
    }
}

/// 会话资产根：`~/.local/state/iso-cc/sessions/`（13 sweep 的枚举基础，§4-L3；
/// list::session_dir_names 复用）。
pub(crate) fn sessions_root() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    Ok(Path::new(&home).join(".local/state/iso-cc/sessions"))
}

/// 单会话资产目录：`sessions/<session-id>/`（localtime/timezone/resolv.conf/gateway.log）。
fn session_dir(session_id: &str) -> anyhow::Result<PathBuf> {
    Ok(sessions_root()?.join(session_id))
}

/// TZ/LANG/显式 env 与 CLAUDE_CONFIG_DIR 摘除。调用对象 = 会话 env 的传播点：
/// pasta（spawn 模式，经 env 链传至 bootstrap/cc）或 bootstrap（slirp 模式）。
/// ISO_CC_SESSION 双标记不在此：pasta/slirp 由 Command::env 注入，bootstrap 自设（§1.4）。
fn apply_env(cmd: &mut Command, profile: &Profile) {
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

/// gateway.log 末尾 max 行（#4 tail 取证）。
fn tail_lines(path: &Path, max: usize) -> String {
    let Ok(s) = std::fs::read_to_string(path) else {
        return "(gateway.log 不可读)".into();
    };
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(max);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_plan_roundtrip() {
        let plan = BootstrapPlan {
            mode: BootstrapMode::Mountns,
            ipv6_off: true,
            binds: vec![("/a".into(), "/b".into())],
            iface: provider::NS_IFNAME.into(),
            timeout_ms: 15_000,
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: BootstrapPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
        assert!(json.contains("\"mode\":\"mountns\""), "{json}");
        assert!(json.contains("[[\"/a\",\"/b\"]]"), "{json}");
    }

    #[test]
    fn bootstrap_plan_unknown_field_rejected() {
        let err = serde_json::from_str::<BootstrapPlan>(
            r#"{"mode":"selfmap","ipv6_off":false,"binds":[],"iface":"tap0","timeout_ms":15000,"extra":1}"#,
        );
        assert!(err.is_err(), "deny_unknown_fields 必须拒绝未知键（#5）");
    }

    #[test]
    fn bootstrap_plan_unknown_mode_rejected() {
        let err = serde_json::from_str::<BootstrapPlan>(
            r#"{"mode":"weird","ipv6_off":false,"binds":[],"iface":"tap0","timeout_ms":15000}"#,
        );
        assert!(err.is_err(), "未知 mode 必须拒绝（#5）");
    }

    #[test]
    fn selfmap_plan_json_shape() {
        let plan = BootstrapPlan {
            mode: BootstrapMode::Selfmap,
            ipv6_off: false,
            binds: vec![],
            iface: provider::NS_IFNAME.into(),
            timeout_ms: 15_000,
        };
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("\"mode\":\"selfmap\""), "{json}");
    }
}
