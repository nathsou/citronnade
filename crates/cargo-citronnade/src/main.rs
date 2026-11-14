//! `cargo citronnade`: build, package and run Nintendo 3DS homebrew.

mod building;
mod icon;
mod json;
mod link;
mod new;
mod package;
mod romfs;
mod toolchain;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use toolchain::Toolchain;

const USAGE: &str = "\
Build, package and run Nintendo 3DS homebrew written with Citronnade.

Usage: cargo citronnade <command> [options] [cargo options]

Commands:
  build              Build for the 3DS and package each binary as a .3dsx
  run                Build, then send the .3dsx to the Homebrew Launcher (3dslink)
  link <file.3dsx>   Send an existing .3dsx to the Homebrew Launcher
  3dsx <file.elf>    Convert an ELF into a .3dsx (with the default SMDH)
  new <path>         Create a project
  <other>            Run any other cargo command (check, clippy, doc, ...) for the 3DS

Options for run and link:
  -a, --address <ip>  The console's address (default: discover it)
  -r, --retries <n>   Discovery attempts (default: 10)
  -- <args>...        Arguments for the program on the console

Options for new:
  --path <dir>        Depend on a local Citronnade checkout instead of git

Other options are passed to cargo (for example --release, -p hello, --example x).";

struct LinkOptions {
    address: Option<Ipv4Addr>,
    retries: u32,
    program_args: Vec<String>,
}

/// Splits our own options from cargo's.
fn take_link_options(args: &[String]) -> Result<(LinkOptions, Vec<String>), String> {
    let mut opts = LinkOptions { address: None, retries: 10, program_args: Vec::new() };
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-a" | "--address" => {
                let v = args.get(i + 1).ok_or("--address needs a value")?;
                opts.address = Some(v.parse().map_err(|_| format!("`{v}` is not an IPv4 address"))?);
                i += 1;
            }
            "-r" | "--retries" => {
                let v = args.get(i + 1).ok_or("--retries needs a value")?;
                opts.retries = v.parse().map_err(|_| format!("`{v}` is not a number"))?;
                i += 1;
            }
            "--" => {
                opts.program_args = args[i + 1..].to_vec();
                break;
            }
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    Ok((opts, rest))
}

fn send(opts: &LinkOptions, file: &Path) -> Result<(), String> {
    let address = match opts.address {
        Some(a) => a,
        None => link::discover(opts.retries)?,
    };
    link::send(address, file, &opts.program_args)
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        println!("{USAGE}");
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("cargo-citronnade {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "build" => {
            let tc = Toolchain::find()?;
            building::build(&tc, "build", rest).map(|_| ())
        }
        "run" => {
            let (opts, cargo_args) = take_link_options(rest)?;
            let tc = Toolchain::find()?;
            let built = building::build(&tc, "build", &cargo_args)?;
            match built.as_slice() {
                [one] => send(&opts, &one.dsx),
                [] => Err("nothing to run: the build produced no binary".into()),
                many => Err(format!(
                    "the build produced {} binaries; pick one with --bin, --example or -p",
                    many.len()
                )),
            }
        }
        "link" => {
            let (opts, files) = take_link_options(rest)?;
            let file = files.first().ok_or("link needs a .3dsx file")?;
            send(&opts, Path::new(file))
        }
        "3dsx" => {
            let elf = PathBuf::from(rest.first().ok_or("3dsx needs an ELF file")?);
            let info = package::PackageInfo::default();
            let (out, layout) = package::package(&elf, &info, None)?;
            eprintln!("    Packaged {} ({:?})", out.display(), layout);
            Ok(())
        }
        "new" => {
            let mut path = None;
            let mut local = None;
            let mut i = 0;
            while i < rest.len() {
                if rest[i] == "--path" {
                    local = Some(PathBuf::from(rest.get(i + 1).ok_or("--path needs a directory")?));
                    i += 1;
                } else {
                    path = Some(PathBuf::from(&rest[i]));
                }
                i += 1;
            }
            let path = path.ok_or("new needs a path")?;
            new::new_project(&path, local.as_deref())
        }
        other => {
            // Any other cargo command, for the 3DS target.
            let tc = Toolchain::find()?;
            let status = tc
                .cargo_command(other)
                .args(rest)
                .status()
                .map_err(|e| format!("could not run cargo: {e}"))?;
            if status.success() { Ok(()) } else { Err(format!("cargo {other} failed")) }
        }
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `cargo citronnade ...` runs us as `cargo-citronnade citronnade ...`.
    if args.first().map(String::as_str) == Some("citronnade") {
        args.remove(0);
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
