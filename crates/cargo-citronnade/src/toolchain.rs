//! Finding the pieces of the Rust toolchain a 3DS build needs.

use std::path::PathBuf;
use std::process::Command;

pub const TARGET: &str = "armv6k-nintendo-3ds";

pub struct Toolchain {
    pub cargo: PathBuf,
    pub rust_lld: PathBuf,
}

fn rustc() -> String {
    std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into())
}

fn output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("could not run {:?}: {e}", cmd.get_program()))?;
    if !out.status.success() {
        return Err(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Toolchain {
    pub fn find() -> Result<Toolchain, String> {
        let cargo = std::env::var_os("CARGO").map(PathBuf::from).unwrap_or_else(|| "cargo".into());
        let sysroot = PathBuf::from(output(Command::new(rustc()).args(["--print", "sysroot"]))?);
        let version = output(Command::new(rustc()).arg("-vV"))?;
        let host = version
            .lines()
            .find_map(|l| l.strip_prefix("host: "))
            .ok_or("could not determine the host triple from `rustc -vV`")?;
        if !version.contains("-nightly") && !version.contains("-dev") {
            return Err(format!(
                "a nightly toolchain is required (to build `core` for {TARGET}); found:\n  {}\n\
                 Add a rust-toolchain.toml with `channel = \"nightly\"`, or run `rustup override set nightly`.",
                version.lines().next().unwrap_or("")
            ));
        }
        let exe = if cfg!(windows) { ".exe" } else { "" };
        let rust_lld = sysroot.join("lib/rustlib").join(host).join("bin").join(format!("rust-lld{exe}"));
        if !rust_lld.exists() {
            return Err(format!("rust-lld was not found at {}", rust_lld.display()));
        }
        let src = sysroot.join("lib/rustlib/src/rust/library/core");
        if !src.exists() {
            return Err("the rust-src component is missing: run `rustup component add rust-src`".into());
        }
        Ok(Toolchain { cargo, rust_lld })
    }

    /// The flags every 3DS build needs, for `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`.
    pub fn rustflags() -> &'static str {
        "-C linker-flavor=ld.lld -C link-arg=-Tcitronnade.ld -C link-arg=--emit-relocs"
    }

    /// A cargo command for the 3DS target with `core`/`alloc` built from source.
    pub fn cargo_command(&self, subcommand: &str) -> Command {
        let mut cmd = Command::new(&self.cargo);
        cmd.arg(subcommand)
            .arg("--target")
            .arg(TARGET)
            .arg("-Zbuild-std=core,alloc")
            .arg("-Zbuild-std-features=compiler-builtins-mem");
        cmd.env("CARGO_TARGET_ARMV6K_NINTENDO_3DS_LINKER", &self.rust_lld);
        let mut flags = Self::rustflags().to_string();
        if let Ok(extra) = std::env::var("CARGO_TARGET_ARMV6K_NINTENDO_3DS_RUSTFLAGS") {
            flags = format!("{flags} {extra}");
        }
        cmd.env("CARGO_TARGET_ARMV6K_NINTENDO_3DS_RUSTFLAGS", &flags);
        // RUSTFLAGS would override the target-specific flags entirely.
        if let Ok(global) = std::env::var("RUSTFLAGS") {
            cmd.env("RUSTFLAGS", format!("{global} {flags}"));
        }
        cmd
    }
}
