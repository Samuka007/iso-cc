use clap::{Parser, Subcommand};

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
    /// Run a command inside an isolated session (egress + locale + CC config)
    Run,
    /// Diff declared config against host reality
    Doctor,
    /// List live sessions (scan /proc; sessions die with their process tree)
    List,
    /// Run purity probes inside a session (red/green + JSON)
    Verify,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Run) => anyhow::bail!("run: not implemented yet (T2/T3/T4)"),
        Some(Commands::Doctor) => anyhow::bail!("doctor: not implemented yet (T1)"),
        Some(Commands::List) => anyhow::bail!("list: not implemented yet (T1)"),
        Some(Commands::Verify) => anyhow::bail!("verify: not implemented yet (T5)"),
        None => {
            println!("iso-cc — see `iso-cc --help`");
            Ok(())
        }
    }
}
