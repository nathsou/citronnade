//! Packing a directory into a RomFS image.

use std::path::Path;

use citronnade_formats::romfs;

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue; // hidden files (.gitkeep, .DS_Store) stay out
        }
        if path.is_dir() {
            walk(base, &path, out)?;
        } else {
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            let data = std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
            out.push((rel, data));
        }
    }
    Ok(())
}

/// Builds a RomFS image from every file under `dir`.
pub fn build(dir: &Path) -> Result<Vec<u8>, String> {
    if !dir.is_dir() {
        return Err(format!("the RomFS directory {} does not exist", dir.display()));
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let entries: Vec<romfs::Entry<'_>> = files.iter().map(|(p, d)| romfs::Entry { path: p, data: d }).collect();
    let image = romfs::build(&entries);
    eprintln!("      RomFS {} ({} files, {} KiB)", dir.display(), files.len(), image.len().div_ceil(1024));
    Ok(image)
}
