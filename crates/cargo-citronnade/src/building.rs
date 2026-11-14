//! `cargo citronnade build`: build for the 3DS and package each binary.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;

use crate::json;
use crate::package::{self, PackageInfo};
use crate::toolchain::Toolchain;

pub struct Built {
    pub dsx: PathBuf,
    /// The package's metadata.
    pub info: PackageInfo,
}

/// Runs `cargo <subcommand>` for the 3DS with `args`, then packages every
/// executable it produced. Returns the packaged programs.
/// With `--cia` among `args`, each program is also packaged as an
/// installable title (`.cia`, and the `.cxi` inside it).
pub fn build(tc: &Toolchain, subcommand: &str, args: &[String]) -> Result<Vec<Built>, String> {
    let cia = args.iter().any(|a| a == "--cia");
    let args: Vec<String> = args.iter().filter(|a| *a != "--cia").cloned().collect();
    let packages = package::load(&tc.cargo)?;
    let mut cmd = tc.cargo_command(subcommand);
    cmd.arg("--message-format=json-render-diagnostics").args(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = cmd.spawn().map_err(|e| format!("could not run cargo: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let mut executables: Vec<(PathBuf, String)> = Vec::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if !line.starts_with('{') {
            println!("{line}");
            continue;
        }
        let Ok(msg) = json::parse(&line) else { continue };
        if msg.str("reason") == Some("compiler-artifact") {
            if let Some(exe) = msg.str("executable") {
                executables.push((PathBuf::from(exe), msg.str("package_id").unwrap_or_default().to_string()));
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("the build failed".into());
    }

    let mut built = Vec::new();
    for (elf, id) in executables {
        let fallback = PackageInfo::default();
        let info = packages.get(&id).unwrap_or(&fallback);
        let romfs = match &info.romfs {
            Some(dir) => Some(crate::romfs::build(dir)?),
            None => None,
        };
        let (dsx, layout) = package::package(&elf, info, romfs.as_deref())?;
        eprintln!(
            "    Packaged {} (code {} KiB, rodata {} KiB, data {} KiB, bss {} KiB, {} relocations)",
            dsx.display(),
            layout.code.div_ceil(1024),
            layout.rodata.div_ceil(1024),
            layout.data.div_ceil(1024),
            layout.bss.div_ceil(1024),
            layout.absolute_relocations + layout.relative_relocations
        );
        if cia {
            let out = package::package_cia(&elf, info, romfs.as_deref())?;
            eprintln!(
                "    Packaged {} (title {:016X}, {} KiB; {} for emulators)",
                out.cia.display(),
                out.title_id,
                out.size.div_ceil(1024),
                out.cxi.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        built.push(Built { dsx, info: info.clone() });
    }
    Ok(built)
}
