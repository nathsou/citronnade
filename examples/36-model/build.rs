//! Converts the OBJ model into the RomFS directory (`romfs/lemon.cmsh`,
//! generated, not committed), which `cargo citronnade build` packs into the
//! 3DSX.
fn main() {
    citronnade_assets::build_mesh("assets/lemon.obj", "romfs");
}
