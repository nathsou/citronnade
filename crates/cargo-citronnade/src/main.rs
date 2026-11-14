//! `cargo citronnade`: build, package and run Nintendo 3DS homebrew.

mod building;
mod console;
mod icon;
mod json;
mod link;
mod new;
mod package;
mod qr;
mod romfs;
mod serve;
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
                     (--cia: also as an installable .cia, and its .cxi)
  run                Build, then send the .3dsx to the Homebrew Launcher (3dslink)
  link <file.3dsx>   Send an existing .3dsx to the Homebrew Launcher
  serve              Build each binary as a .cia, serve it over HTTP on this
                     network and show a QR code for FBI (Remote Install >
                     Scan QR Code)
  qr <text> [--svg <file>]  Show a QR code of the text (a URL for FBI, say)
  test               Build, send each package with a console test, and check
                     its output (see [package.metadata.citronnade.test])
  3dsx <file.elf>    Convert an ELF into a .3dsx (with the default SMDH)
  new <path>         Create a project
  <other>            Run any other cargo command (check, clippy, doc, ...) for the 3DS

Options for run, link and test:
  -a, --address <ip>  The console's address (default: discover it)
  -r, --retries <n>   Discovery attempts (default: 10)
  -s, --server        (run, link) Show the program's output here until it exits
  -v, --verbose       (test) Show each program's output
  -- <args>...        Arguments for the program on the console

Options for serve:
  --port <n>          The HTTP port (default: 8000)
  --address <ip>      This computer's address on the network (default: found)

Options for new:
  --path <dir>        Depend on a local Citronnade checkout instead of git

Other options are passed to cargo (for example --release, -p hello, --example x).";

struct LinkOptions {
    address: Option<Ipv4Addr>,
    retries: u32,
    program_args: Vec<String>,
    server: bool,
    verbose: bool,
}

/// Splits our own options from cargo's.
fn take_link_options(args: &[String]) -> Result<(LinkOptions, Vec<String>), String> {
    let mut opts = LinkOptions { address: None, retries: 10, program_args: Vec::new(), server: false, verbose: false };
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
            "-s" | "--server" => opts.server = true,
            "-v" | "--verbose" => opts.verbose = true,
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

fn console_address(opts: &LinkOptions) -> Result<Ipv4Addr, String> {
    match opts.address {
        Some(a) => Ok(a),
        None => link::discover(opts.retries),
    }
}

fn send(opts: &LinkOptions, file: &Path) -> Result<(), String> {
    let address = console_address(opts)?;
    if !opts.server {
        return link::send(address, file, &opts.program_args);
    }
    // Listen first, then send with the argument that makes the program
    // connect back, then show its output until it exits.
    let listener = console::listen()?;
    let mut args = opts.program_args.clone();
    args.push(console::STDIO_ARG.to_string());
    link::send(address, file, &args)?;
    eprintln!("     Waiting for the program's output (Ctrl-C stops)");
    let (_, _) = console::collect(
        &listener,
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(u64::MAX / 4),
        &mut std::io::stdout(),
    )?;
    eprintln!("     The program closed its output");
    Ok(())
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
        "qr" => {
            let text = rest.first().ok_or("qr needs the text to encode")?;
            let code = qr::QrCode::encode(text.as_bytes()).ok_or("the text is too long for a QR code (213 bytes at most)")?;
            print!("{}", code.to_terminal());
            if let Some(i) = rest.iter().position(|a| a == "--svg") {
                let path = rest.get(i + 1).ok_or("--svg needs a file")?;
                std::fs::write(path, code.to_svg()).map_err(|e| format!("could not write {path}: {e}"))?;
            }
            Ok(())
        }
        "serve" => {
            let mut port = 8000u16;
            let mut address = None;
            let mut cargo_args = vec!["--cia".to_string()];
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--port" => {
                        let v = rest.get(i + 1).ok_or("--port needs a value")?;
                        port = v.parse().map_err(|_| format!("`{v}` is not a port"))?;
                        i += 1;
                    }
                    "--address" => {
                        let v = rest.get(i + 1).ok_or("--address needs a value")?;
                        address = Some(v.parse().map_err(|_| format!("`{v}` is not an IPv4 address"))?);
                        i += 1;
                    }
                    other => cargo_args.push(other.to_string()),
                }
                i += 1;
            }
            let tc = Toolchain::find()?;
            let built = building::build(&tc, "build", &cargo_args)?;
            let cias: Vec<PathBuf> = built.iter().map(|b| b.dsx.with_extension("cia")).collect();
            serve::serve(&cias, address, port)
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
        "test" => {
            let (opts, cargo_args) = take_link_options(rest)?;
            let tc = Toolchain::find()?;
            let built = building::build(&tc, "build", &cargo_args)?;
            let address = console_address(&opts)?;
            let sender = |file: &Path, args: &[String]| link::send(address, file, args);
            let (mut passed, mut failed, mut skipped) = (0, 0, 0);
            for b in &built {
                let Some(spec) = &b.info.test else {
                    skipped += 1;
                    continue;
                };
                if console::run_test(&b.info.name, &b.dsx, spec, &sender, opts.verbose) {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            println!("{passed} passed, {failed} failed, {skipped} without a console test");
            if failed == 0 { Ok(()) } else { Err(format!("{failed} console test(s) failed")) }
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
