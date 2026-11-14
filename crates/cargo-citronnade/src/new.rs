//! `cargo citronnade new`: a project template.

use std::path::Path;

pub fn new_project(path: &Path, citronnade_path: Option<&Path>) -> Result<(), String> {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("bad project path")?
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "-");
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("`{name}` is not a valid package name (it must not start with a digit)"));
    }
    let dependency = match citronnade_path {
        Some(p) => format!("citronnade = {{ path = {:?} }}", p.join("crates/citronnade")),
        None => "citronnade = { git = \"https://github.com/nathsou/citronnade\" }".to_string(),
    };
    let write = |rel: &str, content: String| -> Result<(), String> {
        let p = path.join(rel);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&p, content).map_err(|e| format!("could not write {}: {e}", p.display()))
    };
    write(
        "Cargo.toml",
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
description = "A Nintendo 3DS homebrew program"

[package.metadata.citronnade]
title = "{name}"
# icon = "icon.png"   # 48x48 PNG
# romfs = "romfs"     # directory packed into the 3DSX

[dependencies]
{dependency}

[profile.dev]
panic = "abort"
opt-level = 1

[profile.release]
panic = "abort"
lto = true
"#
        ),
    )?;
    write(
        "rust-toolchain.toml",
        "[toolchain]\nchannel = \"nightly-2026-11-13\"\ncomponents = [\"rust-src\"]\nprofile = \"minimal\"\n".into(),
    )?;
    write(".gitignore", "/target\n".into())?;
    write(
        "src/main.rs",
        r#"#![no_std]
#![no_main]

use citronnade::log;

#[citronnade::main]
fn main() {
    log!("Hello from the 3DS!");
}
"#
        .into(),
    )?;
    eprintln!("     Created `{name}` in {}", path.display());
    eprintln!("     Build it with `cargo citronnade build`, send it with `cargo citronnade run`.");
    Ok(())
}
