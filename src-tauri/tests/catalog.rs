use std::{fs, io::Write, path::PathBuf};
use waw_image_viewer::{catalog::Catalog, iwi};
use zip::write::SimpleFileOptions;

fn archive(path: &std::path::Path, files: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn cache_duplicates_exports_and_changed_archives() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("game");
    let cache = fixture.path().join("cache");
    let exports = fixture.path().join("exports");
    fs::create_dir_all(root.join("main")).unwrap();
    fs::create_dir_all(&exports).unwrap();
    let research = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.local/research/image-audit");
    let references: serde_json::Value =
        serde_json::from_slice(&fs::read(research.join("decoded.json")).unwrap()).unwrap();
    let cube = references
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["flags"].as_u64().unwrap() & 4 != 0)
        .unwrap();
    let bytes =
        fs::read(research.join(format!("samples/{}.iwi", cube["sha256"].as_str().unwrap())))
            .unwrap();
    archive(&root.join("main/a.iwd"), &[("images/cube.iwi", &bytes)]);
    archive(&root.join("main/b.iwd"), &[("images/cube.iwi", &bytes)]);
    let mut catalog = Catalog::new(root.clone(), cache.clone());
    let index = catalog.scan(false).unwrap();
    assert!(index.warnings.is_empty());
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries[0].sources.len(), 2);
    let id = index.entries[0].id.clone();
    let thumb = catalog.thumbnail(&id).unwrap();
    assert_eq!(catalog.thumbnail(&id).unwrap(), thumb);
    let mut warm = Catalog::new(root.clone(), cache.clone());
    assert_eq!(warm.scan(false).unwrap().entries[0].id, id);
    let iwi_path = exports.join("cube.iwi");
    catalog.export(&id, &iwi_path, "iwi", false).unwrap();
    assert_eq!(fs::read(&iwi_path).unwrap(), bytes);
    assert!(catalog.export(&id, &iwi_path, "iwi", false).is_err());
    assert_eq!(fs::read(&iwi_path).unwrap(), bytes);
    let pngs = catalog.export(&id, &exports, "png", false).unwrap();
    let levels = iwi::decode(&bytes).unwrap();
    assert_eq!(pngs.len(), 6);
    for (face, path) in pngs.iter().enumerate() {
        assert_eq!(
            image::open(path).unwrap().into_rgba8().into_raw(),
            levels[0].faces[face]
        );
    }
    assert!(catalog.export(&id, &exports, "png", false).is_err());
    catalog.export(&id, &exports, "png", true).unwrap();
    catalog.clear_cache().unwrap();
    assert!(fs::read_dir(cache.join("thumbnails"))
        .unwrap()
        .next()
        .is_none());
    assert!(pngs.iter().all(|path| std::path::Path::new(path).exists()));
    archive(&root.join("main/a.iwd"), &[("images/cube.iwi", b"broken")]);
    assert!(catalog.read(&id).is_err());
    let changed = catalog.scan(true).unwrap();
    assert_eq!(changed.warnings.len(), 1);
    assert_eq!(changed.entries[0].sources.len(), 1);
}

#[test]
fn scan_installed_archives_read_only() {
    let root = PathBuf::from(r"D:\SteamLibrary\steamapps\common\Call of Duty World at War");
    let cache = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::new(root, cache.path().to_path_buf());
    let index = catalog.scan(false).unwrap();
    assert!(index.warnings.is_empty(), "{:?}", index.warnings);
    let source_count: usize = index.entries.iter().map(|entry| entry.sources.len()).sum();
    assert_eq!(source_count, 9331);
    let unique: std::collections::HashSet<_> =
        index.entries.iter().map(|entry| &entry.hash).collect();
    assert_eq!(unique.len(), 8500);
    let warm = catalog.scan(false).unwrap();
    assert_eq!(warm.entries.len(), index.entries.len());
    println!(
        "Indexed {} versions, {} sources and {} distinct images.",
        index.entries.len(),
        source_count,
        unique.len()
    );
}
