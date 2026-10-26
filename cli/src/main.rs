mod call;
mod cargo;
mod clean;
mod compile;
mod dev;
mod library;
mod new;
mod paths;
mod project;
mod restart;
mod run;
mod target;
mod update;
mod watch;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "amane", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// enumeration of all the available commands
#[derive(Subcommand, Debug)]
enum Commands {
    New {
        // Target installation directory [defaults to ~/.config/amane]
        folder: Option<String>,
        #[arg(long)]
        example: bool,
    },
    // depricated command, idk when it will be removed
    #[command(hide = true)]
    Startup {
        #[arg(long)]
        example: bool,
    },
    Target { folder: Option<String>, },
    Update { folder: Option<String>, },
    Compile {
        #[arg(long)]
        restart: bool,
    },
    Run,
    Restart,
    Dev,
    Clean {
        #[arg(long)]
        yes: bool,
    },
    Ipc {
        #[command(subcommand)]
        action: IpcAction,
    },
}

#[derive(Subcommand, Debug)]
enum IpcAction {
    Call {
        // target name
        name: String,
        // args
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { folder, example } => new::run(folder.as_deref(), example),
        Commands::Startup { example } => startup(example),
        Commands::Target { folder } => target::run(folder.as_deref()),
        Commands::Update { folder } => update::run(folder.as_deref()),
        Commands::Compile { restart } => compile::run(restart),
        Commands::Run => run::run(),
        Commands::Restart => restart::run(),
        Commands::Dev => dev::run(),
        Commands::Clean { yes } => clean::run(yes),
        Commands::Ipc { action } => match action {
            IpcAction::Call { name, args } => call::run(&name, &args),
        },
    }
}

// the old name for `amane new`, kept for a while so existing setups don't break
fn startup(example: bool) -> ExitCode {
    eprintln!(
        "warning: `amane startup` is deprecated and will be removed, use `amane new` instead"
    );

    new::run(None, example)
}
