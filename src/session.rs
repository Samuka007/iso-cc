use crate::config::{Engine, ExecBash, NetGateway, NetIpv6, Profile};
use crate::execrpc;
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
use std::time::{Duration, Instant};

/// bootstrap plan（设计稿 §1.4 参数面）：同二进制 serde 往返 + `deny_unknown_fields`
/// （#5 fail-loud）；argv 传递 = 子进程交接零状态（无临时文件）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapPlan {
    pub mode: BootstrapMode,
    pub ipv6_off: bool,
    /// rw bind（票 05 CC 内置对：backing → view，可写）。应用顺序先于 `binds`
    /// （挂叠底层 = 前置语义；同挂载点后到 ro bind 覆盖）。`#[serde(default)]`
    /// 保持旧 JSON 兼容（同 socks 键先例）。
    #[serde(default)]
    pub rw_binds: Vec<(String, String)>,
    pub binds: Vec<(String, String)>,
    pub iface: String,
    pub timeout_ms: u64,
    /// socks5 形态（工单 16）：Some = bootstrap 在 mountns 就绪后 spawn tun2proxy
    /// worker 并等 tun1；None = `if:` 形态（零改动）。`#[serde(default)]` 保持
    /// 旧 JSON 兼容（deny_unknown_fields 只拒未知键，不要求新键存在）。
    #[serde(default)]
    pub socks: Option<SocksPlan>,
    /// MCP loopback 快照缺口兜底（票 20）：true = bootstrap 在 wait_ready 后、
    /// exec 前对声明端口探活并对缺口起 marked socat。`#[serde(default)]` 保持
    /// 旧 JSON 兼容（socks 键先例）；mark 引擎路径恒 false（机制不适用）。
    #[serde(default)]
    pub mcp_fallback: bool,
}

/// socks worker 计划：proxy host 运行时在 ns 内经 netlink 发现（tap0 默认路由网关
/// = pasta 网关地址 = 宿主 loopback 映射），计划只携带端口与 tun 名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocksPlan {
    pub port: u16,
    pub tun: String,
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
    /// exec RPC 通道（仅 `exec.bash=host`，票 15）；wait 收尾时 shutdown。
    pub exec_server: Option<execrpc::ExecServer>,
}

// ===== 生命周期三层防线（票 13；设计稿 design-session-lanes §4）=====
//
// 纪律（票 14 归位）：PR_SET_CHILD_SUBREAPER / kill(2) / waitpid(2) 三个生命周期
// 内核原语的安全薄封装在 ns.rs（全 crate 唯一非安全面）；本模块策略层（KillGuard /
// 收编循环）只做编排，本文件词法无任何非安全块。

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
        // 原语 = ns.rs 安全薄封装；作用于本进程刚 spawn 的直接子女（spawn 返回值到
        // Session 构造之间的窗口）；SIGKILL 不可捕获，waitpid 收尸防僵尸。子进程已
        // 自行退出并被 try_wait reap → kill 得 ESRCH、waitpid 得 ECHILD，均忽略。
        let killed = ns::kill_pid(pid, libc::SIGKILL);
        ns::reap_waitpid(pid);
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

/// egress fail-loud 预检（cmd_run 的 `--print-plan` 同样断言：计划必须可按所印执行；
/// spawn 内部亦调用，幂等）。返回 (pasta outbound 接口, socks worker 计划)。
///
/// - `if:`：#12 撞名拒绝 + #3 sysfs UP 断言。
/// - `socks5://`：outbound = 宿主默认路由接口（#3）；proxy 端口宿主 TCP 可达
///   （#B 预检，R8：绝不回落）。
pub fn egress_preflight(
    profile_name: &str,
    profile: &Profile,
) -> anyhow::Result<(String, Option<SocksPlan>)> {
    match profile.egress().map_err(anyhow::Error::from)? {
        crate::config::Egress::If(name) => {
            provider::validate_egress_iface(&name)?;
            provider::host_iface_up(&name)?;
            if profile.engine() == Engine::Mark {
                mark_preflight(profile_name, profile, &name)?;
            }
            Ok((name, None))
        }
        crate::config::Egress::Socks5 { host, port } => {
            if profile.gateway() == NetGateway::Slirp4netns {
                bail!(
                    "socks5 egress 仅支持 net.gateway=pasta（slirp 回退组合未经工单 16 实测，fail-loud 拒绝落地）"
                );
            }
            let iface = provider::host_default_iface()?;
            provider::host_iface_up(&iface)?;
            provider::socks::host_preflight(&host, port)?;
            Ok((
                iface,
                Some(SocksPlan {
                    port,
                    tun: provider::socks::TUN_IFNAME.to_string(),
                }),
            ))
        }
    }
}

/// mark 引擎 spawn 前 fail-loud 预检（票 18；R8：绝不静默 fail-open）：
/// ① 路由面五件套（v4 规则/dev 路由/unreachable 兜底 + v6 规则/兜底）——任一缺失
/// 即拒绝（uid 流量会 fall-through main 表 = 宿主出口泄漏）；② file-cap 在位真实
/// 断言（助手 --probe 真降权一次）；③ CC backing 属主（chown 未应用 = cc 不可写）。
fn mark_preflight(profile_name: &str, profile: &Profile, iface: &str) -> anyhow::Result<()> {
    let face = crate::mark::route_face(iface);
    let missing = face.missing(iface);
    if !missing.is_empty() {
        return Err(anyhow!(
            "mark 路由面缺失（fail-closed 前提，绝不静默 fail-open）：{}——运行 `iso-cc setup` 并逐条应用其 rootful 步骤",
            missing.join("、")
        ));
    }
    let helper = crate::mark::pinned_helper(profile_name)?;
    crate::mark::probe_helper(&helper)?;
    if profile.cc_isolation() {
        let base = crate::mark::home_root(profile_name);
        let meta = std::fs::metadata(&base).with_context(|| {
            format!(
                "CC backing 基目录 {} 缺失——先运行 `iso-cc setup`",
                base.display()
            )
        })?;
        use std::os::unix::fs::MetadataExt as _;
        if meta.uid() != crate::config::MARK_UID {
            return Err(anyhow!(
                "CC backing {} 属主 = {} ≠ {}：rootful chown 未应用——应用 `iso-cc setup` 输出的 rootful 步骤",
                base.display(),
                meta.uid(),
                crate::config::MARK_UID
            ));
        }
    }
    Ok(())
}

/// 会话入口：egress fail-loud 断言 → 会话资产（sessions/<id>/）→ 网关 provider 化 spawn。
pub fn spawn(profile_name: &str, profile: &Profile, mode: ChildMode) -> anyhow::Result<Session> {
    let (egress_iface, socks_plan) = egress_preflight(profile_name, profile)?;
    // L2（设计稿 §4-L2）：会话树建立前自设 subreaper——收编循环的 reparent 前提
    // （原语 = ns::set_child_subreaper，票 14 归位）。
    ns::set_child_subreaper().map_err(|e| anyhow!("PR_SET_CHILD_SUBREAPER 失败: {e}"))?;

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

    // mark 引擎（票 18）：零 netns 会话——会话根 = file-cap 助手（降权 + 常驻
    // reaper）。netns 编排（locale/resolv binds、redirect/rw bind 检查、bootstrap
    // plan、网关 provider）整体不适用；TZ/LANG/env/CLAUDE_CONFIG_DIR/exec.bash
    // 轴语义原样保留（env 面引擎无关）。
    if profile.engine() == Engine::Mark {
        let mut inner = build_inner(&mode, profile_name, profile);
        if matches!(mode, ChildMode::Probe) {
            // verify 探针执行器同样从 mark 根取二进制（uid 4210 不可达宿主 target/ 链）
            let shim = crate::mark::ensure_shim_binary()?;
            inner[0] = shim.into_os_string();
        }
        // mark 会话可见资产（exec.sock/shim）落 mark 状态根（uid 4210 可遍历；
        // 宿主 $HOME 0700 链路不可达——票 18 宿主矩阵实测）。
        let mark_sess_dir = crate::mark::sessions_root().join(&session_id);
        std::fs::create_dir_all(&mark_sess_dir).with_context(|| {
            format!("创建 mark 会话资产目录 {}", mark_sess_dir.display())
        })?;
        let exec_channel = match profile.exec_bash() {
            ExecBash::Host => Some(execrpc::prepare(&mark_sess_dir, profile, &session_id, true)?),
            ExecBash::Sandbox => None,
        };
        let ctx = SpawnCtx {
            session_id: &session_id,
            sess_dir: &mark_sess_dir,
            plan: &BootstrapPlan {
                mode: BootstrapMode::Mountns,
                ipv6_off: true,
                rw_binds: Vec::new(),
                binds: Vec::new(),
                iface: String::new(),
                timeout_ms: 0,
                socks: None,
                // mark 零 netns：127.0.0.1 天然直达宿主，缺口兜底机制不适用（票 19 §6）
                mcp_fallback: false,
            },
            gateway_bin: PathBuf::new(),
            exec: exec_channel,
        };
        return spawn_mark(ctx, profile_name, profile, inner);
    }

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
    // locale binds（sess_dir 资产 → /etc/*）：dst 缺失则跳过该 bind（R2 视线主 =
    // TZ env；宿主无 /etc/localtime 时即此路径，历史行为保持）。
    binds.retain(|(_, dst)| Path::new(dst).exists());

    // redirect 挂载点 = setup-manifested（票 14 清单态，spec 变更（一）：会话过程
    // 零持久物）：run 零预创建，缺失 = fail-loud 提示 setup 收敛并登记。
    let mut redirect_binds: Vec<(PathBuf, String)> = Vec::new();
    for r in &profile.redirect {
        let (src, dst) = r
            .split_once('=')
            .ok_or_else(|| anyhow!("redirect 必须是 `src=dst`：{r:?}"))?;
        if !Path::new(dst).exists() {
            bail!(
                "挂载点 {dst} 缺失（fail-loud：run 零预创建，setup-manifested 归 setup）——先运行 `iso-cc setup` 收敛并登记"
            );
        }
        redirect_binds.push((PathBuf::from(src), dst.to_string()));
    }
    binds.extend(redirect_binds);

    // CC 内置重定向对（票 05 / R4 / D6）：cc_isolation=true（默认）时 rw bind
    // backing（profiles/<profile>/ 持久态）→ view（cc 默认路径）。挂载点两侧均由
    // setup 收敛并登记（backing=profile-state，view 预创建=mountpoint）；run 期
    // 缺失仍 fail-loud（票 14 契约不变）。false = 无内置对（共享语义，现状等价）。
    // 引擎轴（票 18）：backing 派生按引擎切换——mark 的 backing 在 mark 状态根
    //（/var/tmp/iso-cc-mark/profiles/<p>，uid DAC 属主），不在宿主 state 根。
    let mut rw_binds: Vec<(PathBuf, String)> = Vec::new();
    if profile.cc_isolation() {
        let pairs = if profile.engine() == Engine::Mark {
            crate::mark::cc_builtin_pairs_mark(profile_name)
        } else {
            crate::config::cc_builtin_pairs(profile_name)
        };
        for pair in pairs {
            for (side, path) in [
                ("backing", pair.backing.as_path()),
                ("view", pair.view.as_path()),
            ] {
                if !path.exists() {
                    bail!(
                        "挂载点 {} 缺失（cc 内置对 {key} 的 {side}，fail-loud：run 零预创建）——先运行 `iso-cc setup` 收敛并登记",
                        path.display(),
                        key = pair.key
                    );
                }
            }
            rw_binds.push((pair.backing, pair.view.to_string_lossy().into_owned()));
        }
    }

    // exec RPC 通道（票 15 spec#1）：exec.bash=host 时装配（bin/bash shim 符号链接 +
    // exec.sock bind+0600）。fallible 须在网关 spawn 前——env 注入需要通道路径。
    // 网关二进制解析（fail-loud #2 清单门）须先于通道 prepare：prepare 落盘
    // sock/shim，若其后才因清单缺失 bail，会把会话资产留成 residue（21:01 冒烟实测）。
    // exec RPC 通道（票 15 spec#1）：exec.bash=host 时装配（bin/bash shim 符号链接 +
    // exec.sock bind+0600）。fallible 须在网关 spawn 前——env 注入需要通道路径。
    let gateway_bin = provider::gateway_bin(gateway)?;
    let exec_channel = match profile.exec_bash() {
        ExecBash::Host => Some(execrpc::prepare(&sess_dir, profile, &session_id, false)?),
        ExecBash::Sandbox => None,
    };

    // bootstrap plan（§1.4）：mode 位承载 provider 差异（§1.2 被否双模式并存的收敛点）
    let has_socks = socks_plan.is_some();
    let plan = BootstrapPlan {
        mode: match gateway {
            NetGateway::Pasta => BootstrapMode::Mountns,
            NetGateway::Slirp4netns => BootstrapMode::Selfmap,
        },
        ipv6_off: profile.ipv6() == NetIpv6::Off,
        rw_binds: rw_binds
            .iter()
            .map(|(s, d)| (s.to_string_lossy().into_owned(), d.clone()))
            .collect(),
        binds: binds
            .iter()
            .map(|(s, d)| (s.to_string_lossy().into_owned(), d.clone()))
            .collect(),
        iface: provider::NS_IFNAME.to_string(),
        timeout_ms: 15_000,
        socks: socks_plan,
        mcp_fallback: profile.mcp_fallback(),
    };

    let inner = build_inner(&mode, profile_name, profile);

    // spawn 上下文（票 15 收敛参数面：exec 通道加入后 spawn_pasta 触 clippy 8 参上限）
    let ctx = SpawnCtx {
        session_id: &session_id,
        sess_dir: &sess_dir,
        plan: &plan,
        gateway_bin,
        exec: exec_channel,
    };
    match gateway {
        // socks 形态：pasta outbound 省略（自动检测宿主默认路由接口；显式
        // `--outbound-if4` 会破坏 map-host-loopback 网关地址映射，工单 16 取证）。
        NetGateway::Pasta => spawn_pasta(
            ctx,
            profile,
            if has_socks { None } else { Some(&egress_iface) },
            dns,
            inner,
        ),
        NetGateway::Slirp4netns => spawn_slirp(ctx, profile, inner),
    }
}

/// P8 允许根（票 05）：`find $HOME -newer` 实测写集 ⊆ 允许根集（R4 核心不变式，
/// 判定核在 probe::p8_violations）。构成：
/// ① 工具状态根（sessions/<id>/ 资产与 profiles/ backing 是本工具自写面，声明性
///    存在，非 cc 泄漏）；② 声明重定向集——cc_isolation=true = 内置对 view 路径，
///    false = 宿主 cc 默认路径按声明共享；③ 用户 redirect 两侧；④ R4 写集例外
///    白名单（probe::P8_ALLOW_UNDER，cc autoInstallIdeExtension 等取证登记）。
fn p8_allowed_roots(profile_name: &str, profile: &Profile) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let home = Path::new(&home);
    let mut roots = vec![crate::manifest::state_dir()];
    if profile.engine() == Engine::Mark {
        // mark：会话可见写面在 mark 状态根（HOME 重写目标 + 会话资产），票 18
        roots.push(crate::mark::state_root());
    }
    if profile.cc_isolation() {
        roots.extend(
            crate::config::cc_builtin_pairs(profile_name)
                .into_iter()
                .map(|p| p.view),
        );
    } else {
        roots.extend([
            home.join(".claude"),
            home.join(".claude.json"),
            home.join(".claude.json.backup"),
        ]);
    }
    for r in &profile.redirect {
        if let Some((src, dst)) = r.split_once('=') {
            roots.push(PathBuf::from(src));
            roots.push(PathBuf::from(dst));
        }
    }
    for w in crate::probe::P8_ALLOW_UNDER {
        roots.push(home.join(w));
    }
    roots
}

/// spawn 期共享上下文：会话标识/资产目录/bootstrap plan/exec 通道（票 15）。
struct SpawnCtx<'a> {
    session_id: &'a str,
    sess_dir: &'a Path,
    plan: &'a BootstrapPlan,
    gateway_bin: PathBuf,
    exec: Option<execrpc::ExecChannel>,
}

/// gateway=pasta（primary，设计稿 §1.3 上树）：pasta 即会话根，bootstrap 由 pasta 在
/// userns/netns 内拉起。R8 由内核结构性执行：pasta 死 → bootstrap/cc 经 PDEATHSIG 链即灭。
fn spawn_pasta(
    mut ctx: SpawnCtx<'_>,
    profile: &Profile,
    egress_iface: Option<&str>,
    dns: Ipv4Addr,
    inner: Vec<OsString>,
) -> anyhow::Result<Session> {
    let session_id = ctx.session_id;
    let sess_dir = ctx.sess_dir;
    let plan = ctx.plan;
    let exec = &mut ctx.exec;
    let pasta_bin = &ctx.gateway_bin;
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
    let mut gw = Command::new(pasta_bin);
    for a in provider::pasta::flag_args(egress_iface, dns, log_file.as_os_str()) {
        gw.arg(a);
    }
    gw.arg("--");
    for a in &bs_args {
        gw.arg(a);
    }
    // 双标记之一：pasta 自身带 ISO_CC_SESSION（list/sweep 对网关的识别键，§4-L3）
    gw.env("ISO_CC_SESSION", session_id);
    // TZ/LANG/env 与 CLAUDE_CONFIG_DIR 摘除经 pasta env 链传至 bootstrap/cc；
    // exec.bash=host 时叠加拦截分层注入（L1.5/L1/L2 + ISO_CC_EXEC_SOCK，票 15）
    apply_env(&mut gw, profile, exec.as_ref());
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
        "[iso-cc] session {session_id}: root=pasta(pid={}) outbound-iface={} dns={dns} scope={:?} exec.bash={:?} assets={}",
        pasta.id(),
        egress_iface.unwrap_or("<auto: host default-route>"),
        profile.scope(),
        profile.exec_bash(),
        sess_dir.display()
    );
    guard.disarm();
    // serve 启动点（启动不失败）：置于最后可失败操作之后，KillGuard 面（spawn 后
    // `?` 早退）不扩大。
    let exec_server = exec.take().map(execrpc::ExecChannel::serve);
    Ok(Session {
        root: pasta,
        gateway: None,
        sess_dir: sess_dir.to_path_buf(),
        exec_server,
    })
}

/// gateway=slirp4netns（回退，设计稿 §1.3 下树）：bootstrap 自映射进三 ns（selfmap，
/// 父侧 pre_exec），slirp4netns attach；parent 永不写 /proc/<pid>/maps（Facts §3 残留源消灭）。
fn spawn_slirp(
    mut ctx: SpawnCtx<'_>,
    profile: &Profile,
    inner: Vec<OsString>,
) -> anyhow::Result<Session> {
    let session_id = ctx.session_id;
    let sess_dir = ctx.sess_dir;
    let plan = ctx.plan;
    let exec = &mut ctx.exec;
    let slirp_bin = &ctx.gateway_bin;
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
    apply_env(&mut bs, profile, exec.as_ref());

    // pre_exec 自映射（§1.3 下树）：binds 父进程预转换（§2.1），闭包体 = ns.rs 入口 B 单调用
    let rw = ns::cstring_binds(&plan.rw_binds).context("rw bind 路径预转换")?;
    let binds = ns::cstring_binds(&plan.binds).context("bind 路径预转换")?;
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut bs, move || {
        ns::enter_selfmap_ns(&rw, &binds, expected_ppid)
    });
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
    let mut gw = Command::new(slirp_bin);
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
        "[iso-cc] session {session_id}: root=bootstrap(pid={}) gateway=slirp4netns(pid={}, attach) scope={:?} exec.bash={:?} assets={}",
        root.id(),
        gateway.id(),
        profile.scope(),
        profile.exec_bash(),
        sess_dir.display()
    );
    root_guard.disarm();
    gw_guard.disarm();
    let exec_server = exec.take().map(execrpc::ExecChannel::serve);
    Ok(Session {
        root,
        gateway: Some(gateway),
        sess_dir: sess_dir.to_path_buf(),
        exec_server,
    })
}

/// 会话内命令向量（两引擎共用）：Exec = 原命令行；Probe = 内嵌纯净度探针
/// （--expect-tz + P8 允许根注入）。
fn build_inner(mode: &ChildMode, profile_name: &str, profile: &Profile) -> Vec<OsString> {
    match mode {
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
            // P8 写集探针（票 05）的允许根：声明重定向集 ∪ R4 白名单 ∪ 工具状态根。
            for root in p8_allowed_roots(profile_name, profile) {
                v.push(OsString::from("--allow-under"));
                v.push(root.into_os_string());
            }
            v
        }
    }
}

/// gateway=mark（票 18）：会话根 = file-cap 助手（编译期 uid 降权 + 常驻 reaper）。
/// 零 netns/mountns：binds/forward/ipv6 轴结构性不适用；生命周期 = 助手 subreaper +
/// PDEATHSIG 链 + 同 uid 收编（uid 4210 树对 iso-cc 的 kill/wait 全 EPERM——阶段 A
/// 事故留档——树死亡保证必须由同 uid 成员结构性执行，R8/US18）。
fn spawn_mark(
    mut ctx: SpawnCtx<'_>,
    profile_name: &str,
    profile: &Profile,
    inner: Vec<OsString>,
) -> anyhow::Result<Session> {
    let session_id = ctx.session_id;
    let sess_dir = ctx.sess_dir;
    let exec = &mut ctx.exec;
    let helper = crate::mark::pinned_helper(profile_name)?;

    let mut cmd = Command::new(&helper);
    // argv 可见性 flag（list/sweep 的 /proc/cmdline 键：uid 4210 树的 environ 对
    // 宿主 EACCES，env 标记键在 mark 引擎不可读——cmdline 全局可读）。
    cmd.arg("--session-id").arg(session_id);
    cmd.arg("--");
    for a in &inner {
        cmd.arg(a);
    }
    // 双标记（env 面语义与 pasta/slirp 一致）
    cmd.env("ISO_CC_SESSION", session_id);
    // TZ/LANG/env/CLAUDE_CONFIG_DIR 摘除 + exec.bash=host 拦截分层注入（引擎无关）
    apply_env(&mut cmd, profile, exec.as_ref());
    // mark 版 CC 隔离（R4/D6 的 mark 形态）：HOME 重写 → backing（uid 4210 DAC
    // 属主），cc 无感走默认 ~/.claude 路径；宿主 ~/.claude 对 uid 4210 DAC 不可达。
    if profile.cc_isolation() {
        cmd.env("HOME", crate::mark::home_root(profile_name));
    }
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // PDEATHSIG→iso-cc + 精确对账（ns.rs §4-L1）：iso-cc 死 ⇒ 助手死 ⇒ cc 链式死
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut cmd, move || {
        ns::set_pdeathsig_verified(libc::SIGKILL, expected_ppid)
    });
    let mut root = cmd
        .spawn()
        .with_context(|| format!("spawn mark 会话根（{}）", helper.display()))?;
    // KillGuard（audit-facts §3）：降权后的助手对宿主 SIGKILL EPERM——本窗口内
    // 失败路径都是助手自身已死（快速失败检测），kill 得 ESRCH 无害；存活助手的
    // bail 面为空（spawn 后仅剩 try_wait 窗口与 infallible 构造）。
    let guard = KillGuard::arm(root.id());

    // fail-loud：助手快速失败检测（launch-failure 退出码契约：1 = 降权/fork 失败
    // （file caps 缺失最常见），2 = 用法错误）。目标命令自身的快速退出（含 shell
    // 约定 126/127/信号）原样透传，不劫持语义。
    std::thread::sleep(Duration::from_secs(1));
    if let Some(status) = root.try_wait().context("try_wait mark 会话根")? {
        match status.code() {
            Some(1) | Some(2) => bail!(
                "mark 会话根启动失败（exit={}，fail-loud）：助手降权被拒——file caps 缺失或清单未收敛；运行 `iso-cc setup` 并逐条应用其 rootful 步骤（setcap/chown/ip rule）",
                status.code().unwrap_or(-1)
            ),
            _ => {
                // exec 已发生（或 exec 失败 127/信号死）：目标命令自身的退出态，透传。
            }
        }
    }

    eprintln!(
        "[iso-cc] session {session_id}: root=uidrun(pid={}) engine=mark uid={} egress-iface={}（uidrange→table {} + unreachable backstop）exec.bash={:?} assets={}",
        root.id(),
        crate::config::MARK_UID,
        egress_iface_of(profile),
        crate::config::MARK_TABLE,
        profile.exec_bash(),
        sess_dir.display()
    );
    guard.disarm();
    // serve 启动点（启动不失败）：置于最后可失败操作之后，KillGuard 面不扩大。
    let exec_server = exec.take().map(execrpc::ExecChannel::serve);
    Ok(Session {
        root,
        gateway: None,
        sess_dir: sess_dir.to_path_buf(),
        exec_server,
    })
}

/// mark banner 的 egress 展示（preflight 已断言 if: 形态；socks 在 validate 已拒）。
fn egress_iface_of(profile: &Profile) -> String {
    profile
        .egress_iface()
        .map(str::to_string)
        .unwrap_or_else(|_| "<invalid-egress>".into())
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
    let rw = ns::cstring_binds(&plan.rw_binds).context("rw bind 路径预转换")?;
    let binds = ns::cstring_binds(&plan.binds).context("bind 路径预转换")?;
    // expected_ppid=0 = 结构对账（pasta 路径父 pid 不可预知，ns.rs 入口 A）
    ns::enter_mountns(&rw, &binds, 0).map_err(|e| anyhow!("mountns 装配（pasta 之下）失败: {e}"))
}

/// 等网关 tap 就绪（provider 无关；socks 形态再等 worker tun1）→ exec 真实命令。
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
    if let Some(socks) = &plan.socks {
        spawn_socks_worker_and_wait(plan, socks, session_id)?;
    }
    // 票 20：MCP loopback 声明端口探活 + 快照缺口 socat 兜底（wait_ready 后、exec
    // 前；best-effort，任何失败 Warn 继续，绝不阻断 exec）。
    mcp_gap_fallback(plan, session_id);
    let mut c = Command::new(prog);
    for a in &command[1..] {
        c.arg(a);
    }
    let err = c.exec();
    Err(anyhow!("exec 失败: {err}"))
}

/// 声明端口探活（票 20 spec#1 语义）：connect 成功即可，不解析协议；1s 超时。
fn loopback_tcp_reachable(port: u16, timeout: Duration) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        timeout,
    )
    .is_ok()
}

/// MCP loopback 快照缺口兜底（票 20 spec#2/3）：对 cc 视线的 claude.json（ADR 0006
/// 重定向后路径——bootstrap 已进 mountns，`$HOME/.claude.json` 即 profile backing
/// 的 rw bind view）+ 项目 `.mcp.json` 扫描出的 http/sse loopback 声明端口逐个探活。
/// pasta 镜像集 = attach 时刻快照（exp19 §3.0），探活失败 = 快照缺口 → bind 探测后
/// 起 socat `TCP-LISTEN:<p>,bind=127.0.0.1,fork,reuseaddr TCP:<gw>:<p>` 补齐。
///
/// 冲突语义（spec#3，exp19 §4）：bind 探测 EADDRINUSE = 端口已被持有（会话内服务/
/// 兜底转发器）→ 本地服务优先，跳过 + Warn，绝不起转发器。
///
/// 生命周期（exp19 §5）：socat = marked 子进程（ISO_CC_SESSION）+ PDEATHSIG→bootstrap
/// （exec cc 后父 pid 不变 = PDEATHSIG→cc）——cc 退出即被内核击杀，先于 pasta
/// （pidns init）退出与 netns 销毁，「先转发进程 → 后 pasta」结构性成立（孤儿 socat
/// 会拽住 netns，exp19 实测）。日志落会话资产 mcp-fallback.log（13 sweep 回收）。
fn mcp_gap_fallback(plan: &BootstrapPlan, session_id: &str) {
    if !plan.mcp_fallback {
        return;
    }
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()));
    let project_mcp = std::env::current_dir().ok().map(|d| d.join(".mcp.json"));
    let ports =
        crate::config::mcp_loopback_ports(&home.join(".claude.json"), project_mcp.as_deref());
    if ports.is_empty() {
        return;
    }
    let gw = match netcfg::default_gateway(&plan.iface) {
        Ok(gw) => gw,
        Err(e) => {
            eprintln!(
                "[iso-cc] mcp-loopback: 网关地址发现失败（{e}）——缺口兜底跳过（缺口由会话内 P-MCP 探针红显）"
            );
            return;
        }
    };
    let socat = match provider::which("socat") {
        Some(p) => p,
        None => {
            eprintln!(
                "[iso-cc] mcp-loopback: socat 不在 PATH——缺口兜底不可用（缺口由会话内 P-MCP 探针红显）"
            );
            return;
        }
    };
    let log_file = match session_dir(session_id) {
        Ok(d) => d.join("mcp-fallback.log"),
        Err(e) => {
            eprintln!("[iso-cc] mcp-loopback: 会话资产目录不可用（{e}）——缺口兜底跳过");
            return;
        }
    };
    let log = match std::fs::File::create(&log_file) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "[iso-cc] mcp-loopback: 写 {} 失败（{e}）——缺口兜底跳过",
                log_file.display()
            );
            return;
        }
    };
    for port in ports {
        if loopback_tcp_reachable(port, Duration::from_secs(1)) {
            eprintln!("[iso-cc] mcp-loopback: port={port} reachable（pasta 原生镜像覆盖，零动作）");
            continue;
        }
        match std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!(
                    "iso-cc: mcp-loopback skip port={port} reason=session-conflict（本地服务优先，exp19 §4）"
                );
            }
            Err(e) => {
                eprintln!(
                    "iso-cc: mcp-loopback skip port={port} reason=bind-probe-failed（{e}）"
                );
            }
            Ok(listener) => {
                drop(listener);
                match spawn_socat_forwarder(&socat, gw, port, session_id, &log, &log_file) {
                    Ok(pid) => eprintln!(
                        "[iso-cc] mcp-loopback: socat port={port} -> {gw}:{port}（快照缺口兜底，marked pid={pid}）"
                    ),
                    Err(e) => eprintln!(
                        "iso-cc: mcp-loopback skip port={port} reason=socat-failed（{e:#}）"
                    ),
                }
            }
        }
    }
}

/// 起单端口 socat 转发器（exp19 proto3 E 已证形态）。返回 pid；早期退出（bind 竞态
/// 等）= Err，日志尾部随 Warn 上浮——缺口不静默。
fn spawn_socat_forwarder(
    socat: &Path,
    gw: Ipv4Addr,
    port: u16,
    session_id: &str,
    log: &std::fs::File,
    log_file: &Path,
) -> anyhow::Result<u32> {
    let mut c = Command::new(socat);
    c.arg(format!("TCP-LISTEN:{port},bind=127.0.0.1,fork,reuseaddr"))
        .arg(format!("TCP:{gw}:{port}"));
    // marked 子进程（票 13 语义）：list/sweep 识别键与 socks worker 同规
    c.env("ISO_CC_SESSION", session_id);
    c.stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().context("mcp-fallback.log 复制句柄")?))
        .stderr(Stdio::from(log.try_clone().context("mcp-fallback.log 复制句柄")?));
    // PDEATHSIG→bootstrap：exec cc 不换 pid = PDEATHSIG→cc；cc 死亡即内核击杀转发器，
    // 先于 pasta 收割 netns——teardown 顺序结构性保证，无显式收割表（exp19 §5）。
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut c, move || {
        ns::set_pdeathsig_verified(libc::SIGKILL, expected_ppid)
    });
    let mut child = c
        .spawn()
        .with_context(|| format!("spawn socat（{}）", socat.display()))?;
    let pid = child.id();
    // 早期退出检测（socks worker #B 同构，非致命）：150ms 内退出 = bind 失败类
    std::thread::sleep(Duration::from_millis(150));
    if let Some(status) = child.try_wait().context("try_wait socat 转发器")? {
        let tail = tail_lines(log_file, 20);
        bail!(
            "socat 早期退出（status={status}）；{} 末尾：\n{tail}",
            log_file.display()
        );
    }
    // 句柄有意丢弃（Child drop 不杀进程）：生命周期由 PDEATHSIG→bootstrap(cc) 结构性
    // 保证（socks worker 同规），Session/bootstrap 均不持有。
    drop(child);
    Ok(pid)
}

/// socks 分支（工单 16）：netns 内 spawn tun2proxy worker（标记 + PDEATHSIG→本进程）
/// → netlink 等 tun1 就绪。顺序 = tap0 就绪（网关地址存在的前提）→ 发现网关 →
/// worker → tun1 → exec cc。worker 死 = tun1 无读者 = 协议黑洞（fail-closed，R8 同构，
/// 绝不回落直连）；spawn/就绪失败 = fail-loud（#8 同规：文案含步骤/对象/日志指针）。
fn spawn_socks_worker_and_wait(
    plan: &BootstrapPlan,
    socks: &SocksPlan,
    session_id: &str,
) -> anyhow::Result<()> {
    let bin = provider::pinned_bin(provider::socks::WORKER_KEY)?;
    // worker 日志 → sessions/<id>/worker.log（对齐 slirp 的 gateway.log 取证形态）
    let log_file = session_dir(session_id)?.join("worker.log");
    let log =
        std::fs::File::create(&log_file).with_context(|| format!("创建 {}", log_file.display()))?;
    let gw = netcfg::default_gateway(&plan.iface)
        .map_err(|e| anyhow!("worker 代理地址发现失败（{e}）；tap0 未就绪或无默认路由"))?;
    let mut c = Command::new(&bin);
    for a in provider::socks::worker_args(gw, socks.port) {
        c.arg(a);
    }
    // worker 标记（env 键可读：非 dumpable-0 二进制，list/sweep 归 cc 树成员，
    // 其 netns inode 计入活跃集——会话归属判定零改动）
    c.env("ISO_CC_SESSION", session_id);
    c.stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().context("worker.log 复制句柄")?))
        .stderr(Stdio::from(log));
    // PDEATHSIG→bootstrap：exec cc 后父进程存续（exec 不触发信号），cc 死亡即内核
    // 击杀 worker——worker 存活 ⊆ 会话存活；tun1/路由随 netns 消亡回收，零 residue。
    let expected_ppid = std::process::id();
    ns::install_pre_exec(&mut c, move || {
        ns::set_pdeathsig_verified(libc::SIGKILL, expected_ppid)
    });
    let mut worker = c
        .spawn()
        .with_context(|| format!("spawn tun2proxy worker（{}）", bin.display()))?;
    let guard = KillGuard::arm(worker.id());

    // 就绪等待（带 worker 早期退出检测：tun1 永不就绪时给 worker.log 尾部而非裸超时）
    let deadline = Instant::now() + Duration::from_millis(plan.timeout_ms);
    loop {
        match netcfg::wait_tun_ready(&socks.tun, Duration::from_millis(250)) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(anyhow!("tun1 就绪断言失败（netlink）: {e}")),
        }
        if let Some(status) = worker.try_wait().context("try_wait tun2proxy worker")? {
            let tail = tail_lines(&log_file, 40);
            bail!(
                "tun2proxy worker 早期退出（status={status}，fail-loud #B）：tun1 未就绪；R8：绝不回落；{} 末尾：\n{tail}",
                log_file.display()
            );
        }
        if Instant::now() >= deadline {
            let tail = tail_lines(&log_file, 40);
            bail!(
                "#8 tun1 就绪等待超时（{}ms）：worker 已 spawn 但 tun1/默认路由未出现；R8：绝不回落；{} 末尾：\n{tail}",
                plan.timeout_ms,
                log_file.display()
            );
        }
    }
    guard.disarm();
    let worker_pid = worker.id();
    // worker 句柄有意丢弃（Child drop 不杀进程）：生命周期由 PDEATHSIG→bootstrap(cc)
    // 结构性保证，Session 不持 worker。
    drop(worker);
    eprintln!(
        "[iso-cc] socks worker: tun2proxy(pid={worker_pid}) tun={} proxy=socks5://{gw}:{} log={}",
        socks.tun,
        socks.port,
        log_file.display()
    );
    Ok(())
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
        // exec RPC 收尾（票 15 spec#4）：先停通道 + kill 在途 worker（后台任务存续至
        // 会话结束），再进 L2 收编——worker 的后台子进程（已 reparent、带标记）由收编
        // 循环收割。
        if let Some(s) = self.exec_server.take() {
            s.shutdown();
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
                        // 原语 = ns.rs 安全薄封装；pid = reparent 到本进程的收养子女
                        // （subreaper 语义，waitpid 合法）。已死未收尸者 kill 得 ESRCH、
                        // waitpid 直接收尸；存活者 SIGKILL 不可捕获，阻塞收尸。
                        ns::kill_pid(pid, libc::SIGKILL);
                        ns::reap_waitpid(pid);
                        eprintln!(
                            "[iso-cc] L2 收编: 孤儿 pid={pid}（ISO_CC_SESSION={id}）→ SIGKILL + reaped"
                        );
                        reaped = true;
                    }
                    None => {
                        eprintln!(
                            "[iso-cc] L2 登记: 收养孤儿 pid={pid} 无 ISO_CC_SESSION 标记——不杀；宿主侧 exec worker（票 15）带标记走 marked 支，本支仅域外孤儿"
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
/// `exec=Some`（exec.bash=host，票 15）叠加拦截分层注入：L1.5 `CLAUDE_CODE_SHELL_PREFIX`
/// （附 3 取证覆盖 Bash 工具/hooks/statusline/stdio MCP）+ L1 `SHELL`（fallback 面）+
/// L2 PATH 前置 shim 目录 + `ISO_CC_EXEC_SOCK` 通道指针。
fn apply_env(cmd: &mut Command, profile: &Profile, exec: Option<&execrpc::ExecChannel>) {
    if let Some(tz) = &profile.locale.tz {
        cmd.env("TZ", tz);
        // NixOS 宿主矩阵：tzdata 在 /etc/zoneinfo，TZDIR 只在 login env（会话
        // 常缺失）——缺失且该根有所声明时区数据时补齐，否则 glibc 解析 TZ 失败
        // 回落 UTC（P6a 实测 +0000）。引擎无关（env 面，不依赖 mountns bind）。
        if std::env::var_os("TZDIR").is_none()
            && Path::new("/etc/zoneinfo").join(tz).exists()
        {
            cmd.env("TZDIR", "/etc/zoneinfo");
        }
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
    if let Some(ch) = exec {
        cmd.env("CLAUDE_CODE_SHELL_PREFIX", &ch.shell_path);
        cmd.env("SHELL", &ch.shell_path);
        cmd.env("ISO_CC_EXEC_SOCK", &ch.sock_path);
        // L2：PATH 前置 shim 目录（目录内仅 `bash` 一键；宿主 worker PATH = 宿主基底，
        // 不受此影响——env 策略 server 侧，execrpc::worker_env_vars）。
        if let Some(p) = std::env::var_os("PATH") {
            let mut new_path = OsString::from(ch.bin_dir.as_os_str());
            new_path.push(":");
            new_path.push(&p);
            cmd.env("PATH", new_path);
        }
    }
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
            rw_binds: vec![("/state/profiles/p/claude".into(), "/home/u/.claude".into())],
            binds: vec![("/a".into(), "/b".into())],
            iface: provider::NS_IFNAME.into(),
            timeout_ms: 15_000,
            socks: None,
            mcp_fallback: true,
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: BootstrapPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
        assert!(json.contains("\"mode\":\"mountns\""), "{json}");
        assert!(
            json.contains("\"rw_binds\":[[\"/state/profiles/p/claude\",\"/home/u/.claude\"]]"),
            "{json}"
        );
        assert!(json.contains("[[\"/a\",\"/b\"]]"), "{json}");
        assert!(json.contains("\"mcp_fallback\":true"), "{json}");
    }

    #[test]
    fn bootstrap_plan_old_json_without_rw_binds_compat() {
        // 票 05 新键 rw_binds 走 serde(default)：旧 plan JSON（无该键）仍可解析
        let old: BootstrapPlan = serde_json::from_str(
            r#"{"mode":"mountns","ipv6_off":true,"binds":[],"iface":"tap0","timeout_ms":15000}"#,
        )
        .unwrap();
        assert!(old.rw_binds.is_empty());
        // 票 20 新键同规：旧 plan JSON（无 mcp_fallback 键）仍可解析，缺省 false
        assert!(!old.mcp_fallback);
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
            rw_binds: vec![],
            binds: vec![],
            iface: provider::NS_IFNAME.into(),
            timeout_ms: 15_000,
            socks: None,
            mcp_fallback: false,
        };
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("\"mode\":\"selfmap\""), "{json}");
    }

    #[test]
    fn socks_plan_roundtrip_and_old_json_compat() {
        let plan = BootstrapPlan {
            mode: BootstrapMode::Mountns,
            ipv6_off: true,
            rw_binds: vec![],
            binds: vec![],
            iface: provider::NS_IFNAME.into(),
            timeout_ms: 15_000,
            socks: Some(SocksPlan {
                port: 7891,
                tun: provider::socks::TUN_IFNAME.into(),
            }),
            mcp_fallback: false,
        };
        let json = serde_json::to_string(&plan).unwrap();
        assert!(
            json.contains("\"socks\":{\"port\":7891,\"tun\":\"tun1\"}"),
            "{json}"
        );
        let back: BootstrapPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
        // 旧 plan JSON（无 socks 键）仍可解析：serde(default) 只补缺，不拒旧态
        let old: BootstrapPlan = serde_json::from_str(
            r#"{"mode":"mountns","ipv6_off":true,"binds":[],"iface":"tap0","timeout_ms":15000}"#,
        )
        .unwrap();
        assert_eq!(old.socks, None);
        // socks 子对象未知键照样拒绝（#5 同规）
        let bad = serde_json::from_str::<BootstrapPlan>(
            r#"{"mode":"mountns","ipv6_off":false,"binds":[],"iface":"tap0","timeout_ms":15000,"socks":{"port":1,"tun":"tun1","x":2}}"#,
        );
        assert!(bad.is_err(), "socks 子对象未知键必须拒绝");
    }

    #[test]
    fn egress_preflight_socks_plan_shape() {
        // 结构面：socks 形态计划 = {port, tun1}（宿主事实检查归 spawn/doctor，这里不触网）。
        // host_preflight 对本机必有监听端口不可假设 → 只断言计划构造纯函数面（provider::socks）。
        let p: Profile = toml::from_str("egress = 'socks5://127.0.0.1:7891'").unwrap();
        match p.egress().unwrap() {
            crate::config::Egress::Socks5 { host, port } => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 7891);
                let plan = SocksPlan {
                    port,
                    tun: provider::socks::TUN_IFNAME.into(),
                };
                assert_eq!(plan.tun, "tun1");
            }
            other => panic!("应为 socks 形态: {other:?}"),
        }
    }

    #[test]
    fn p8_roots_declare_redirect_set_by_axis() {
        // 票 05：允许根 = 工具状态根 ∪ 声明集 ∪ redirect 两侧 ∪ R4 白名单。
        // true 轴声明集 = 内置对 view；false 轴 = 宿主 cc 默认路径（声明共享）。
        let on: Profile =
            toml::from_str("egress = 'if:wg0'\nredirect = ['/data/notes=/home/u/notes']").unwrap();
        let roots = p8_allowed_roots("ccx", &on);
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".into()));
        assert!(roots.contains(&crate::manifest::state_dir()));
        assert!(roots.contains(&home.join(".claude")));
        assert!(roots.contains(&PathBuf::from("/data/notes")));
        assert!(roots.contains(&home.join(".vscode/extensions")));

        let off: Profile = toml::from_str("egress = 'if:wg0'\nagent.cc_isolation = false").unwrap();
        let roots = p8_allowed_roots("ccx", &off);
        // false 轴：宿主默认路径仍声明（共享语义），内置对 view 不再来自声明集
        assert!(roots.contains(&home.join(".claude.json")));
    }
}
