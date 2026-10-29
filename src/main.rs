//! grh: install, audit and remove the Green Harness parts (ADR-0050).

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use grh::exit::Exit;
use grh::{catalog, dispatch};

#[derive(Debug, Parser)]
#[command(
    name = "grh",
    version,
    about = "Install, audit and remove the Green Harness parts",
    after_help = "Parts: decisions gate guardrails router conform attest workflow design\n\
                  Profiles: solo (gate guardrails), team (+ decisions conform), full (+ router)\n\
                  Any other first word runs that part: grh decisions resolve <key>"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install parts or profiles, with what they need.
    Install {
        /// Parts and profiles (solo, team, full).
        names: Vec<String>,
        /// Also arm the current repository (git hooks, the decisions hook, attest's workflow).
        #[arg(long)]
        repo: bool,
        /// Back up and overwrite a file grh did not place.
        #[arg(long)]
        replace: bool,
        /// Print the plan and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Go ahead without asking (needed when stdin is not a terminal).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Install a local extension from its manifest directory.
        #[arg(long, value_name = "DIR")]
        extension: Option<std::path::PathBuf>,
        /// Only errors and the summary.
        #[arg(long, short = 'q')]
        quiet: bool,
    },
    /// Remove what grh placed: everything, or the named parts.
    Uninstall {
        names: Vec<String>,
        /// Only the current repository's entries.
        #[arg(long)]
        repo: bool,
        /// Refuse to remove a part another installed part needs.
        #[arg(long)]
        keep_deps: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long, short = 'q')]
        quiet: bool,
    },
    /// Read a repository and recommend parts and a profile. Changes nothing.
    Audit {
        dir: Option<std::path::PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Each part: installed or not, its version, placed or adopted.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Run a part's own command: grh <part> [args…].
    #[command(external_subcommand)]
    Part(Vec<OsString>),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: Result<(), Exit> = match cli.command {
        Command::Part(argv) => Err(dispatch::run(&catalog::Catalog::load(), &argv)),
        Command::Install { .. }
        | Command::Uninstall { .. }
        | Command::Audit { .. }
        | Command::Status { .. } => Err(Exit::Usage("not implemented yet".into())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => e.report(),
    }
}
