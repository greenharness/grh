//! grh: install, audit and remove the Green Harness parts (ADR-0050).

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use grh::exit::Exit;
use grh::{audit, catalog, dispatch, extension, install, status, uninstall};

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

/// The argv that asks clap for the help `args` wanted, when an unknown
/// argument stopped the parse and `-h` or `--help` stands before any `--`
/// (cli.help.help-and-version: `grh --bogus --help` is someone asking how to
/// use us). It keeps the subcommand path the words name and drops everything
/// else, so clap prints the same help as `grh <path> --help`. A part's own
/// arguments never get here: `grh decisions --bogus --help` parses as that
/// part's command and is passed through.
fn help_argv(args: &[OsString]) -> Option<Vec<OsString>> {
    use clap::CommandFactory;
    let mut command = Cli::command();
    let mut argv = vec![args.first()?.clone()];
    let mut wants_help = false;
    for arg in args.iter().skip(1) {
        let Some(word) = arg.to_str() else { continue };
        match word {
            "--" => break,
            "-h" | "--help" => wants_help = true,
            _ if word.starts_with('-') => {}
            _ => {
                if let Some(sub) = command.find_subcommand(word).cloned() {
                    argv.push(arg.clone());
                    command = sub;
                }
            }
        }
    }
    wants_help.then(|| {
        argv.push("--help".into());
        argv
    })
}

/// `Cli::parse`, except that help wins over an unknown argument.
fn parse_cli() -> Cli {
    let args: Vec<OsString> = std::env::args_os().collect();
    match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) if e.kind() == clap::error::ErrorKind::UnknownArgument => {
            match help_argv(&args).map(Cli::try_parse_from) {
                Some(Err(help)) if help.kind() == clap::error::ErrorKind::DisplayHelp => {
                    help.exit()
                }
                _ => e.exit(),
            }
        }
        Err(e) => e.exit(),
    }
}

fn main() -> ExitCode {
    let cli = parse_cli();
    let result: Result<(), Exit> = match cli.command {
        Command::Part(argv) => match catalog::Catalog::load() {
            Ok(cat) => Err(dispatch::run(&cat, &argv)),
            Err(e) => Err(e),
        },
        Command::Install {
            names,
            repo,
            replace,
            dry_run,
            yes,
            extension,
            quiet,
        } => {
            if let Some(dir) = extension {
                if !names.is_empty() {
                    Err(Exit::Usage(
                        "--extension installs one extension; name no parts with it".into(),
                    ))
                } else {
                    extension::run(&extension::Opts {
                        dir,
                        replace,
                        dry_run,
                        yes,
                        quiet,
                    })
                }
            } else {
                catalog::Catalog::load().and_then(|cat| {
                    install::run(
                        &cat,
                        &install::Opts {
                            names,
                            repo,
                            replace,
                            dry_run,
                            yes,
                            quiet,
                        },
                    )
                })
            }
        }
        Command::Uninstall {
            names,
            repo,
            keep_deps,
            dry_run,
            yes,
            quiet,
        } => catalog::Catalog::load().and_then(|cat| {
            uninstall::run(
                &cat,
                &uninstall::Opts {
                    names,
                    repo,
                    keep_deps,
                    dry_run,
                    yes,
                    quiet,
                },
            )
        }),
        Command::Status { json } => catalog::Catalog::load().and_then(|cat| {
            let format = if json {
                status::Format::Json
            } else {
                status::Format::Text
            };
            status::run(&cat, format)
        }),
        Command::Audit { dir, json } => catalog::Catalog::load().and_then(|cat| {
            let format = if json {
                status::Format::Json
            } else {
                status::Format::Text
            };
            audit::run(&cat, dir.as_deref(), format)
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => e.report(),
    }
}
