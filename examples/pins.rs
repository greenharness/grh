//! Pin a release's manifest into `manifests/` and re-check every pin of the
//! shipped catalog. A thin argv and exit wrapper over `grh::pins`.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use grh::manifest::FILE_NAME;
use grh::pins::{check, check_exit_code, pin, Existing};

const USAGE: &str = "usage: pins pin [--replace] <name> <repo> <tag> [asset]
       pins check
       pins -h

pin    download the release asset (default greenharness.toml), check it as the
       catalog does, write manifests/<name>.toml and print the [[pinned]] block.
       It refuses to overwrite a copy with other bytes unless --replace.
check  download every [[pinned]] asset of catalog.toml and compare its sha256
       with the pin and the copy; one line per pin, or `0 pinned manifests`.

GRH_RELEASE_BASE replaces https://github.com; GH_TOKEN or GITHUB_TOKEN is
sent to the first host only.

Exit codes (this example's own, not grh's table):
  0  done
  1  mismatch, missing asset, refusal, not found, or any other failure
  2  usage
  3  every failing pin was unreachable (1 wins when a pin also mismatched
     or is missing)";

fn usage_error(message: &str) -> ExitCode {
    eprintln!("pins: {message}\n{USAGE}");
    ExitCode::from(2)
}

/// Writes to stdout; a closed pipe ends the run quietly with `code`, the
/// command's own exit code.
fn emit(out: &str, code: ExitCode) -> Result<(), ExitCode> {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Err(code),
        Err(e) => {
            eprintln!("pins: cannot write to stdout: {e}");
            Err(ExitCode::from(1))
        }
    }
}

fn run_pin(args: &[String]) -> ExitCode {
    let mut replace = false;
    let mut operands = Vec::new();
    let mut options_ended = false;
    for arg in args {
        match arg.as_str() {
            "--" if !options_ended => options_ended = true,
            "--replace" if !options_ended => replace = true,
            flag if !options_ended && flag.starts_with('-') && flag != "-" => {
                return usage_error(&format!("unknown option {flag}"));
            }
            operand => operands.push(operand),
        }
    }
    let (name, repo, tag, asset) = match operands.as_slice() {
        [name, repo, tag] => (name, repo, tag, FILE_NAME),
        [name, repo, tag, asset] => (name, repo, tag, *asset),
        _ => return usage_error("pin takes <name> <repo> <tag> [asset]"),
    };
    let existing = if replace {
        Existing::Replace
    } else {
        Existing::Keep
    };
    let copies = Path::new(env!("CARGO_MANIFEST_DIR")).join("manifests");
    match pin(&copies, name, repo, tag, asset, existing) {
        Ok(block) => match emit(&block, ExitCode::SUCCESS) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        },
        Err(error) => {
            eprintln!("pins: {error}");
            ExitCode::from(if error.is_unreachable() { 3 } else { 1 })
        }
    }
}

fn run_check(args: &[String]) -> ExitCode {
    if let Some(extra) = args.first() {
        return usage_error(&format!("check takes no argument, got {extra}"));
    }
    // The catalog grh would load: `GRH_CATALOG` and the `manifests/` beside
    // it when set, else this repository's own.
    let catalog = std::env::var_os("GRH_CATALOG")
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog.toml"));
    let root = catalog.parent().unwrap_or(Path::new(".")).to_path_buf();
    let source = match std::fs::read_to_string(&catalog) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("pins: cannot read {}: {e}", catalog.display());
            return ExitCode::from(1);
        }
    };
    let results = match check(&source, &root.join("manifests")) {
        Ok(results) => results,
        Err(exit) => {
            eprintln!("pins: {}", grh::ops::text(&exit));
            return ExitCode::from(1);
        }
    };
    let mut out = String::new();
    if results.is_empty() {
        out.push_str("0 pinned manifests\n");
    }
    for result in &results {
        out.push_str(&format!("{result}\n"));
    }
    let code = ExitCode::from(check_exit_code(&results));
    match emit(&out, code) {
        Ok(()) => code,
        Err(ended) => ended,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return match emit(&format!("{USAGE}\n"), ExitCode::SUCCESS) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        };
    }
    match args.split_first() {
        Some((verb, rest)) if verb == "pin" => run_pin(rest),
        Some((verb, rest)) if verb == "check" => run_check(rest),
        Some((other, _)) if other.starts_with('-') => {
            usage_error(&format!("unknown option {other}"))
        }
        Some((other, _)) => usage_error(&format!("unknown command {other}")),
        None => usage_error("a command is required"),
    }
}
