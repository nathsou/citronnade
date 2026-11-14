use std::{env, fs, path::PathBuf};

fn main() {
    // The linker script goes into OUT_DIR, which is added to the link search
    // path of everything that depends on this crate; cargo-citronnade passes
    // `-C link-arg=-Tcitronnade.ld`.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("citronnade.ld"), include_bytes!("citronnade.ld")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=citronnade.ld");
}
