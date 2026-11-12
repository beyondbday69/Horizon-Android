use super::*;
use std::io::Write;
use std::path::PathBuf;

fn temp_home(name: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("horizon-local-mod-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    home
}

/// A content mod the manifest and archive checks accept: one `data/` file and no code.
fn content_mod(dir: &Path, id: &str, version: &str) -> PathBuf {
    let manifest = format!(
        "id = \"{id}\"\nname = \"Sample {id}\"\nversion = \"{version}\"\napi = \"0.2\"\n\
         capabilities = [\"registries\"]\n"
    );
    let bytes = horizon_mods::Package::write_files(
        &manifest,
        None,
        &[("data/sample/horizon/block/rock.json", b"{}")],
    )
    .unwrap();
    let path = dir.join(format!("{id}.hmod"));
    fs::write(&path, bytes).unwrap();
    path
}

/// A native bundle: the manifest and the mixin's own identity agree.
fn native_mod(dir: &Path, id: &str) -> PathBuf {
    let revision = "a".repeat(40);
    let mixins = "b".repeat(40);
    let manifest = format!(
        "{{\"schema\":1,\"id\":\"{id}\",\"version\":\"1.0.0\",\"game\":\"26.3\",\
         \"horizon\":\"{revision}\",\"mixins\":\"{mixins}\"}}"
    );
    let mixin = format!(
        "[mod]\nid = \"{id}\"\nversion = \"1.0.0\"\n\n\
         [compatibility]\nhorizon_revision = \"{revision}\"\n"
    );
    let path = dir.join(format!("{id}.hnmod"));
    let file = fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("horizon-native.json", options).unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    zip.start_file(format!("mixins/{id}/horizon-mixin.toml"), options)
        .unwrap();
    zip.write_all(mixin.as_bytes()).unwrap();
    zip.finish().unwrap();
    path
}

#[test]
fn a_content_mod_is_copied_and_listed_off() {
    let dir = temp_home("content");
    let home = dir.join("game");
    let from = content_mod(&dir, "sample", "1.2.0");

    let added = install(&home, &from).unwrap();
    assert_eq!(added.id, "sample");
    assert_eq!(added.title, "Sample sample");
    assert_eq!(added.version, "1.2.0");
    assert_eq!(added.file, "horizon-mods/sample.hmod");
    assert!(!added.native);
    assert!(home.join(&added.file).is_file());

    let list = installed::Installed::load(&home);
    let entry = list.find(SourceId::Local, "sample").unwrap();
    assert_eq!(entry.file, added.file);
    assert_eq!(entry.version, "1.2.0");
    assert!(entry.checksum.starts_with("SHA-256:"));
    // As a registry install, it is off until the player turns it on.
    assert!(!java::is_enabled(&home, "sample"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_file_is_never_replaced() {
    let dir = temp_home("exists");
    let home = dir.join("game");
    let from = content_mod(&dir, "sample", "1.0.0");
    install(&home, &from).unwrap();

    let err = install(&home, &from).unwrap_err();
    assert!(err.contains("already there"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_native_mod_is_installed_on_and_waits_for_a_build() {
    let dir = temp_home("native");
    let home = dir.join("game");
    let from = native_mod(&dir, "sample_native");

    let added = install(&home, &from).unwrap();
    assert_eq!(added.id, "sample_native");
    assert!(added.native);
    assert_eq!(added.file, "horizon-native-mods/sample_native.hnmod");
    assert!(home.join(&added.file).is_file());
    assert!(java::is_enabled(&home, "sample_native"));
    // The Mods page lists it from the folder, without a browser entry.
    let untracked = crate::native_status::untracked(&home);
    assert_eq!(untracked.len(), 1);
    assert_eq!(untracked[0].id, "sample_native");
    assert!(installed::Installed::load(&home).entries.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn another_kind_of_file_is_refused() {
    let dir = temp_home("other");
    let home = dir.join("game");
    let from = dir.join("notes.txt");
    fs::write(&from, b"not a mod").unwrap();
    let err = install(&home, &from).unwrap_err();
    assert!(err.contains(".hmod or a .hnmod"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}
