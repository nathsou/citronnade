//! Package metadata (`[package.metadata.citronnade]`) and packaging an ELF
//! into a 3DSX.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use citronnade_formats::{dsx, png, smdh};

use crate::json::{self, Value};

/// What the SMDH and the 3DSX need to know about a package.
#[derive(Debug, Clone, Default)]
pub struct PackageInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub icon: Option<PathBuf>,
    pub romfs: Option<PathBuf>,
    /// What `cargo citronnade test` checks in the program's output.
    pub test: Option<TestSpec>,
}

/// A console test: lines the output must contain (in order) and must not
/// contain, and how long the program may run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestSpec {
    pub expect: Vec<String>,
    pub reject: Vec<String>,
    pub timeout_secs: u64,
}

/// `[package.metadata.citronnade.test]`, or else an emulator check that
/// needs no input and exits by itself.
fn test_spec(meta: Option<&Value>) -> Option<TestSpec> {
    let strings = |v: Option<&Value>, key: &str| -> Vec<String> {
        v.and_then(|t| t.get(key)).map(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_str).map(String::from).collect()
    };
    let meta = meta?;
    if let Some(t) = meta.get("test") {
        let timeout = t.get("timeout").and_then(Value::as_f64).unwrap_or(30.0) as u64;
        return Some(TestSpec { expect: strings(Some(t), "expect"), reject: strings(Some(t), "reject"), timeout_secs: timeout });
    }
    let emu = meta.get("emulator")?;
    let interactive = !emu.get("input").map(Value::as_array).unwrap_or(&[]).is_empty();
    let exits = emu.get("exits").and_then(Value::as_bool).unwrap_or(false);
    let needs_host = emu.get("host_servers").is_some() || emu.get("sd_files").is_some();
    if interactive || !exits || needs_host {
        return None;
    }
    let frames = emu.get("frames").and_then(Value::as_f64).unwrap_or(600.0);
    Some(TestSpec { expect: strings(Some(emu), "expect"), reject: strings(Some(emu), "reject"), timeout_secs: (frames / 60.0) as u64 + 20 })
}

/// Package ID -> metadata, from `cargo metadata`.
pub fn load(cargo: &Path) -> Result<HashMap<String, PackageInfo>, String> {
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let v = json::parse(&String::from_utf8_lossy(&out.stdout))?;
    let mut map = HashMap::new();
    for p in v.get("packages").map(Value::as_array).unwrap_or(&[]) {
        let id = p.str("id").unwrap_or_default().to_string();
        let manifest = PathBuf::from(p.str("manifest_path").unwrap_or_default());
        let dir = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
        let meta = p.get("metadata").and_then(|m| m.get("citronnade"));
        let path = |key: &str| meta.and_then(|m| m.str(key)).map(|s| dir.join(s));
        let authors: Vec<&str> = p.get("authors").map(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_str).collect();
        // "Name <email>" -> "Name"
        let author = (!authors.is_empty()).then(|| {
            authors.iter().map(|a| a.split('<').next().unwrap_or(a).trim()).collect::<Vec<_>>().join(", ")
        });
        map.insert(
            id,
            PackageInfo {
                name: p.str("name").unwrap_or_default().to_string(),
                test: test_spec(meta),
                title: meta.and_then(|m| m.str("title")).map(String::from),
                description: p.str("description").map(String::from),
                author: meta.and_then(|m| m.str("author")).map(String::from).or(author),
                icon: path("icon"),
                romfs: path("romfs"),
            },
        );
    }
    Ok(map)
}

/// Builds the SMDH for a package.
pub fn make_smdh(info: &PackageInfo, binary: &str) -> Result<Vec<u8>, String> {
    let icon = match &info.icon {
        Some(path) => {
            let data = std::fs::read(path).map_err(|e| format!("could not read icon {}: {e}", path.display()))?;
            let img = png::decode(&data).map_err(|e| format!("{}: {e}", path.display()))?;
            if img.width == 48 && img.height == 48 { img.rgba } else { img.resized(48, 48).rgba }
        }
        None => crate::icon::default_icon(),
    };
    let title = info.title.clone().unwrap_or_else(|| binary.to_string());
    let description = info.description.clone().unwrap_or_else(|| "Built with Citronnade".into());
    let author = info.author.clone().unwrap_or_else(|| "Unknown".into());
    Ok(smdh::Smdh {
        short_description: &title,
        long_description: &description,
        publisher: &author,
        icon: &icon,
        flags: smdh::DEFAULT_FLAGS,
    }
    .to_bytes())
}

/// Converts an ELF to a 3DSX next to it. Returns the 3DSX path.
pub fn package(elf: &Path, info: &PackageInfo, romfs: Option<&[u8]>) -> Result<(PathBuf, dsx::Layout), String> {
    let data = std::fs::read(elf).map_err(|e| format!("could not read {}: {e}", elf.display()))?;
    let binary = elf.file_stem().and_then(|s| s.to_str()).unwrap_or("program");
    let smdh = make_smdh(info, binary)?;
    let (out, layout) = dsx::convert(&data, &dsx::Extras { smdh: Some(&smdh), romfs })
        .map_err(|e| format!("{}: {e}", elf.display()))?;
    let path = elf.with_extension("3dsx");
    std::fs::write(&path, out).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    std::fs::write(elf.with_extension("smdh"), smdh).map_err(|e| format!("could not write the SMDH: {e}"))?;
    Ok((path, layout))
}
