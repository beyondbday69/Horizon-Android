//! A mod from a file on this computer: the player picks an `.hmod` or `.hnmod` in the Mods
//! page ("Add mod file…"), drops it on the window, or runs `mods add-file`, and the launcher
//! checks it the way it checks a download before it reaches the game folder (docs/mods.md).
//!
//! An `.hmod` is a content or sandboxed WebAssembly mod the game reads when it starts: it is
//! copied to `horizon-mods/` and listed like a registry install, off until the player turns it
//! on. An `.hnmod` is native code that changes the game's source before it is compiled: it is
//! copied to `horizon-native-mods/` and the instance's native build compiles it
//! (docs/native-mods.md). Neither runs anything here: the checks read the archive.
use super::{installed, java, Kind, SourceId};
use crate::native_mods;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// What [`install`] added.
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    /// The mod's manifest id: what the game's mod list and the build use.
    pub id: String,
    pub title: String,
    pub version: String,
    /// The file in the game folder, relative to it.
    pub file: String,
    /// A native mod needs the instance's native build before it runs.
    pub native: bool,
}

/// Why `from` isn't a mod this launcher installs.
fn not_a_mod(from: &Path) -> String {
    let name = from
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| from.display().to_string());
    format!("{name} isn't a Horizon mod: pick a .hmod or a .hnmod.")
}

/// Copies and checks the mod file `from` into the game folder `home`, and lists it. An existing
/// file of the same name is never replaced: the player removes it first (as a download does).
pub fn install(home: &Path, from: &Path) -> Result<Added, String> {
    if !from.is_file() {
        return Err(format!("{} is not a file.", from.display()));
    }
    let ext = from
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "hmod" => install_content(home, from),
        "hnmod" => install_native(home, from),
        _ => Err(not_a_mod(from)),
    }
}

/// A content or WebAssembly mod: `horizon-mods/<id>.hmod`, listed and off until turned on.
fn install_content(home: &Path, from: &Path) -> Result<Added, String> {
    let package = horizon_mods::Package::read(from)?;
    let m = package.manifest;
    let relative = format!("{}/{}.hmod", java::MODS_DIR, m.id);
    let target = home.join(&relative);
    if target.exists() {
        return Err(format!(
            "{relative} is already there: remove that mod first to replace it."
        ));
    }
    let bytes = fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    write_new(&target, &bytes)?;
    let entry = installed::Entry {
        source: SourceId::Local,
        project: m.id.clone(),
        slug: m.id.clone(),
        title: m.name.clone(),
        kind: Kind::Mod,
        version: m.version.clone(),
        number: m.version.clone(),
        file: relative.clone(),
        checksum: format!("SHA-256:{}", hex(&bytes)),
        page: String::new(),
        dependency: false,
        verdict: None,
        skipped: Vec::new(),
        namespaces: m.namespaces,
    };
    let mut list = installed::Installed::load(home);
    if list.entries.iter().any(|e| e.project == entry.project) {
        let _ = fs::remove_file(&target);
        return Err(format!("{} is already installed.", m.id));
    }
    list.entries.push(entry);
    if let Err(e) = list.save(home) {
        let _ = fs::remove_file(&target);
        return Err(e);
    }
    Ok(Added {
        id: m.id,
        title: m.name,
        version: m.version,
        file: relative,
        native: false,
    })
}

/// Native code: `horizon-native-mods/<id>.hnmod`, on, then the instance's native build compiles
/// it (docs/native-mods.md). The file is checked like a registry `.hnmod`; nothing is built or
/// run here.
fn install_native(home: &Path, from: &Path) -> Result<Added, String> {
    let bundle = horizon_mods::index::native::Bundle::read(from)?;
    let m = bundle.manifest;
    let relative = format!("{}/{}.hnmod", native_mods::INSTALLED, m.id);
    let target = home.join(&relative);
    if target.exists() {
        return Err(format!(
            "{relative} is already there: remove that mod first to replace it."
        ));
    }
    let bytes = fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    // A different file name, same mod: the game would load two of it.
    if home
        .join(native_mods::INSTALLED)
        .read_dir()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path() != target)
        .any(|e| {
            horizon_mods::index::native::Bundle::read(&e.path())
                .is_ok_and(|other| other.manifest.id == m.id)
        })
    {
        return Err(format!("{} is already installed.", m.id));
    }
    write_new(&target, &bytes)?;
    // Native mods are always on: the build is what turns the code into the game.
    java::set_enabled(home, &m.id, true)?;
    Ok(Added {
        id: m.id.clone(),
        title: m.id,
        version: m.version,
        file: relative,
        native: true,
    })
}

/// Writes `bytes` to `target` (whose folder is made first) only when nothing is there.
fn write_new(target: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if target.exists() {
        return Err(format!(
            "{} is already there: remove it first to replace it.",
            target.display()
        ));
    }
    fs::write(target, bytes).map_err(|e| format!("{}: {e}", target.display()))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
