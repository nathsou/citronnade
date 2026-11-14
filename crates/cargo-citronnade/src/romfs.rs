//! RomFS images (filled in by v0.4).

use std::path::Path;

pub fn build(dir: &Path) -> Result<Vec<u8>, String> {
    Err(format!("RomFS packing is not supported yet (requested for {})", dir.display()))
}
