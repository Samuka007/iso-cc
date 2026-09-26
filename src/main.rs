mod config;
mod doctor;
mod list;
mod plan;

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
        /// 显式配置文件路径（默认：全局配置 + 项目 .iso-cc.toml）
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// 机器可读输出
        #[arg(long)]
        json: bool,
    },
    /// 会话内运行纯净度探针（红绿 + JSON）——T5 落地
    Verify {
        #[arg(long)]
        profile: Option<String>,
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
            command,
        }) => cmd_run(config, profile, print_plan, command),
        Some(Commands::Doctor {
            profile,
            config,
            json,
        }) => cmd_doctor(config, profile, json),
        Some(Commands::List { config, json }) => cmd_list(config, json),
        Some(Commands::Verify { profile: _ }) => bail!("verify: not implemented yet (T5)"),
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
    // T2/T3/T4 工单落地后走真实执行路径。
    bail!("会话执行尚未实现（T2/T3/T4）")
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

fn cmd_list(config: Option<std::path::PathBuf>, json: bool) -> anyhow::Result<()> {
    let (cfg, _) = load_config(config.as_deref())?;
    let _ = cfg; // list 零配置依赖；保留入口一致性
    let sessions = list::scan();
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        print!("{}", list::render_human(&sessions));
    }
    Ok(())
}
