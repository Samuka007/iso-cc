mod config;
mod doctor;
mod execrpc;
mod execstub;
mod gc;
mod list;
mod manifest;
mod netcfg;
mod ns;
mod plan;
mod probe;
mod provider;
mod session;
mod setup;

use anyhow::{bail, Context as _};
use clap::{Parser, Subcommand};
use std::ffi::OsString;

#[derive(Parser)]
#[command(
    name = "iso-cc",
    version,
    about = "Rootless declarative sandbox sessions for Claude Code"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a command inside an isolated session
    Run {
        /// Profile 名（唯一 profile 时可省略）
        #[arg(long)]
        profile: Option<String>,
        /// 显式配置文件路径（默认：全局配置 + 项目 .iso-cc.toml）
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// 只打印等价执行计划，不执行
        #[arg(long)]
        print_plan: bool,
        /// 在会话内运行纯净度探针（代替命令）
        #[arg(long)]
        verify: bool,
        /// 要在会话内执行的命令（-- 之后的全部）
        #[arg(last = true)]
        command: Vec<OsString>,
    },
    /// 配置 vs 宿主现实的全量 diff（任何 FAIL → exit 1）
    Doctor {
        #[arg(long)]
        profile: Option<String>,
        /// 显式配置文件路径（默认：全局配置 + 项目 .iso-cc.toml）
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// 机器可读输出
        #[arg(long)]
        json: bool,
    },
    /// 收敛 setup-manifested 资源并登记清单（幂等；重跑 action diff = 0）
    Setup {
        /// Profile 名（唯一 profile 时可省略）
        #[arg(long)]
        profile: Option<String>,
        /// 机器可读输出
        #[arg(long)]
        json: bool,
    },
    /// 清单回收：默认 sweep 报告；--prune 清 stale 登记；--all 全量回收（终态清单与现实一致）
    Gc {
        /// 清 stale 条目（仅登记簿）
        #[arg(long)]
        prune: bool,
        /// Profile 名（profile-state 仅随显式 --profile 回收）
        #[arg(long)]
        profile: Option<String>,
        /// 全量回收（有活跃会话拒绝；provider 二进制永不删除，只除名）
        #[arg(long)]
        all: bool,
        /// 越过 mountpoint 用户数据化守卫（显式放弃守卫数据）
        #[arg(long)]
        force: bool,
        /// 跳过确认提示
        #[arg(long)]
        yes: bool,
    },
    /// 列出存活会话
    List {
        #[arg(long)]
        json: bool,
    },
    /// 会话内运行纯净度探针（红绿 + JSON）——T5 落地
    Verify {
        #[arg(long)]
        profile: Option<String>,
        /// 显式配置文件路径
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// 内部：探针执行器（verify 经会话调用；勿手动使用）
    #[command(hide = true)]
    ProbeJson {
        /// 期望时区（声明值）
        #[arg(long)]
        expect_tz: Option<String>,
        /// P8 允许根（票 05）：声明重定向集 ∪ R4 白名单 ∪ 工具状态根，spawn 侧推导
        #[arg(long = "allow-under")]
        allow_under: Vec<std::path::PathBuf>,
    },
    /// 内部：会话引导（等 tap0 → 配网 → exec）
    #[command(hide = true)]
    SessionBootstrap {
        /// bootstrap plan JSON（session::BootstrapPlan；deny_unknown_fields，#5 fail-loud）
        #[arg(long)]
        plan: String,
        /// 会话 id（bootstrap 自设 ISO_CC_SESSION 标记，§1.4）
        #[arg(long)]
        session_id: String,
        #[arg(last = true)]
        command: Vec<OsString>,
    },
}

fn main() {
    // multi-call stub 分派（票 15 / ADR 0008 附 2）：argv0 basename == "bash" = 转发
    // 模式（L1 SHELL / L1.5 prefix / L2 PATH shim 三种调用形态共用同一入口；不返回）。
    if execstub::is_stub_invocation(std::env::args_os().next().as_deref()) {
        execstub::forward();
    }
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run {
            profile,
            config,
            print_plan,
            verify,
            command,
        }) => {
            if verify && !command.is_empty() {
                bail!("--verify 与命令互斥");
            }
            if verify {
                return cmd_verify(profile, config);
            }
            cmd_run(config, profile, print_plan, command)
        }
        Some(Commands::Doctor {
            profile,
            config,
            json,
        }) => cmd_doctor(config, profile, json),
        Some(Commands::Setup { profile, json }) => cmd_setup(profile, json),
        Some(Commands::Gc {
            prune,
            profile,
            all,
            force,
            yes,
        }) => cmd_gc(gc::GcOpts {
            prune,
            all,
            yes,
            force,
            profile,
        }),
        Some(Commands::List { json, .. }) => cmd_list(json),
        Some(Commands::Verify { profile, config }) => cmd_verify(profile, config),
        Some(Commands::ProbeJson {
            expect_tz,
            allow_under,
        }) => {
            let probes = probe::run(expect_tz.as_deref().unwrap_or("UTC"), &allow_under);
            println!("{}", probe::to_json(&probes)?);
            eprint!("{}", probe::render_human(&probes));
            if probe::any_fail(&probes) {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::SessionBootstrap {
            plan,
            session_id,
            command,
        }) => session::bootstrap_run(&plan, &session_id, &command),
        None => {
            println!("iso-cc — see `iso-cc --help`");
            Ok(())
        }
    }
}

fn load_config(
    explicit: Option<&std::path::Path>,
) -> anyhow::Result<(config::Config, Vec<String>)> {
    match explicit {
        Some(p) => config::load(p, None).context("加载配置失败"),
        None => config::load(&config::global_path(), Some(&config::project_path()))
            .context("加载配置失败"),
    }
}

fn resolve(
    cfg: &config::Config,
    profile: &Option<String>,
) -> anyhow::Result<(String, config::Profile)> {
    let (name, prof) = config::resolve_profile(cfg, profile.as_deref())?;
    config::require_valid(prof)?;
    Ok((name.to_string(), prof.clone()))
}

fn cmd_run(
    config: Option<std::path::PathBuf>,
    profile: Option<String>,
    print_plan: bool,
    command: Vec<OsString>,
) -> anyhow::Result<()> {
    let (cfg, warnings) = load_config(config.as_deref())?;
    for w in &warnings {
        eprintln!("note: {w}");
    }
    let (name, prof) = resolve(&cfg, &profile)?;
    // egress 预检（工单 16 两形态）：`if:` → #12 撞名拒绝 + #3 sysfs UP；
    // `socks5://` → #B proxy 可达 + 宿主默认路由接口 #3。R8 绝不回落。
    // --print-plan 同样断言：计划必须可按所印执行。
    let _ = session::egress_preflight(&prof)?;
    let plan = plan::plan_lines(&name, &prof, command.first());
    if print_plan || command.is_empty() {
        for l in plan {
            println!("{l}");
        }
    }
    if command.is_empty() {
        return Ok(());
    }
    if print_plan {
        return Ok(());
    }
    let session = session::spawn(&name, &prof, session::ChildMode::Exec(command))?;
    let status = session.wait()?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn cmd_verify(profile: Option<String>, config: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let (cfg, warnings) = load_config(config.as_deref())?;
    for w in &warnings {
        eprintln!("note: {w}");
    }
    let (name, prof) = resolve(&cfg, &profile)?;
    let expect_tz = prof.locale.tz.clone();
    let session = session::spawn(&name, &prof, session::ChildMode::Probe)?;
    let status = session.wait()?;
    if !status.success() {
        bail!("verify 红：环境与声明不一致（详见上方 JSON/摘要）");
    }
    let _ = expect_tz;
    Ok(())
}

fn cmd_doctor(
    config: Option<std::path::PathBuf>,
    profile: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let (cfg, warnings) = load_config(config.as_deref())?;
    for w in &warnings {
        eprintln!("note: {w}");
    }
    let (name, prof) = resolve(&cfg, &profile)?;
    let checks = doctor::run(&cfg, &name, &prof, &doctor::RealSys);
    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        print!("{}", doctor::render_human(&checks));
    }
    if doctor::any_fail(&checks) {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_list(json: bool) -> anyhow::Result<()> {
    let sessions = list::scan();
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        print!("{}", list::render_human(&sessions));
    }
    Ok(())
}

fn cmd_setup(profile: Option<String>, json: bool) -> anyhow::Result<()> {
    let (cfg, warnings) = load_config(None)?;
    for w in &warnings {
        eprintln!("note: {w}");
    }
    let (name, prof) = resolve(&cfg, &profile)?;
    setup::run(&name, &prof, json)
}

fn cmd_gc(opts: gc::GcOpts) -> anyhow::Result<()> {
    let (cfg, warnings) = load_config(None)?;
    for w in &warnings {
        eprintln!("note: {w}");
    }
    gc::run(&opts, &cfg.profile)
}
