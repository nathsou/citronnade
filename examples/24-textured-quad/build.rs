//! Converts the PNG tile to a tiled RGBA5551 texture at build time.
fn main() {
    citronnade_assets::build_texture("assets/tile.png", citronnade_assets::TexFormat::Rgba5551);
}
