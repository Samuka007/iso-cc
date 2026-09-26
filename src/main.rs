mod config;
mod doctor;
mod list;
mod plan;
mod probe;
mod session;

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
    },
    /// 内部：会话引导（等 tap0 → 配网 → exec）
    #[command(hide = true)]
    SessionBootstrap {
        #[arg(long)]
        egress_iface: String,
        #[arg(last = true)]
        command: Vec<OsString>,
    },
}

fn main() {
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
        Some(Commands::List { json, .. }) => cmd_list(json),
        Some(Commands::Verify { profile, config }) => cmd_verify(profile, config),
        Some(Commands::ProbeJson { expect_tz }) => {
            let probes = probe::run(expect_tz.as_deref().unwrap_or("UTC"));
            println!("{}", probe::to_json(&probes)?);
            eprint!("{}", probe::render_human(&probes));
            if probe::any_fail(&probes) {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::SessionBootstrap {
            egress_iface,
            command,
        }) => session::bootstrap(&egress_iface, &command),
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
