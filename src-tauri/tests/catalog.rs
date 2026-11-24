use std::{fs, io::Write, path::PathBuf};
use waw_image_viewer::{
    catalog::{Catalog, Source},
    iwi,
};
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
    let mut bytes = bitmap(30);
    bytes[5] |= 4; // Six cubemap faces with one pixel each.
    for red in [60, 90, 120, 150, 180] {
        bytes.extend_from_slice(&[0, 0, red, 255]);
    }
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
#[ignore = "requires WAW_IMAGE_VIEWER_TEST_ROOT to point at a WaW installation"]
fn scan_installed_sources_read_only() {
    let root = PathBuf::from(std::env::var_os("WAW_IMAGE_VIEWER_TEST_ROOT").unwrap());
    let cache = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::new(root, cache.path().to_path_buf());
    let index = catalog.scan(false).unwrap();
    assert!(index.warnings.is_empty(), "{:?}", index.warnings);
    let archive_count = index
        .entries
        .iter()
        .flat_map(|entry| &entry.sources)
        .filter(|source| matches!(source, Source::Archive { .. }))
        .count();
    let loose_count = index
        .entries
        .iter()
        .flat_map(|entry| &entry.sources)
        .filter(|source| matches!(source, Source::Loose { .. }))
        .count();
    assert!(archive_count > 0);
    println!(
        "Indexed {} versions from {archive_count} IWD entries and {loose_count} loose files.",
        index.entries.len()
    );
    let warm = catalog.scan(false).unwrap();
    assert_eq!(warm.entries.len(), index.entries.len());
}

fn bitmap(red: u8) -> Vec<u8> {
    let mut bytes = vec![0; 28];
    bytes[..4].copy_from_slice(b"IWi\x06");
    bytes[4] = 1; // BGRA bitmap
    bytes[5] = 2; // No mipmaps
    bytes[6] = 1;
    bytes[8] = 1;
    bytes[10] = 1;
    bytes.extend_from_slice(&[0, 0, red, 255]);
    bytes
}

#[test]
fn loose_folder_is_optional_and_extensions_are_the_only_listing_gate() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("game");
    let cache = fixture.path().join("cache");
    fs::create_dir_all(root.join("main")).unwrap();
    archive(
        &root.join("main/base.iwd"),
        &[("images/stock.iwi", &bitmap(50))],
    );
    let mut catalog = Catalog::new(root.clone(), cache.clone());
    let initial = catalog.scan(false).unwrap();
    assert!(initial.warnings.is_empty());
    assert_eq!(initial.entries.len(), 1);

    let folder = root.join("raw/images");
    fs::create_dir_all(folder.join("custom")).unwrap();
    fs::create_dir(folder.join("folder.iwi")).unwrap();
    let image = bitmap(100);
    fs::write(folder.join("custom/mod.IWI"), &image).unwrap();
    fs::write(folder.join("bad.iwi"), b"broken").unwrap();
    fs::write(folder.join("ignore.png"), &image).unwrap();

    let index = catalog.scan(false).unwrap();
    assert!(index.warnings.is_empty());
    assert_eq!(index.entries.len(), 3);
    let entry = index
        .entries
        .iter()
        .find(|entry| entry.name == "images/custom/mod.IWI")
        .unwrap();
    assert!(matches!(&entry.sources[..], [Source::Loose { entry }] if entry == "custom/mod.IWI"));
    assert_eq!(catalog.pixels(&entry.id, 0).unwrap(), [100, 0, 0, 255]);
    assert!(catalog
        .thumbnail(&entry.id)
        .unwrap()
        .starts_with("data:image/png;base64,"));
    let png = fixture.path().join("mod.png");
    catalog.export(&entry.id, &png, "png", false).unwrap();
    assert_eq!(
        image::open(png).unwrap().into_rgba8().into_raw(),
        [100, 0, 0, 255]
    );

    let bad = index
        .entries
        .iter()
        .find(|entry| entry.name == "images/bad.iwi")
        .unwrap();
    assert!(bad.header.is_none());
    assert!(catalog.pixels(&bad.id, 0).is_err());
    let exported = fixture.path().join("original.iwi");
    catalog.export(&bad.id, &exported, "iwi", false).unwrap();
    assert_eq!(fs::read(exported).unwrap(), b"broken");

    let mut reopened = Catalog::new(root.clone(), cache);
    assert_eq!(reopened.scan(false).unwrap().entries.len(), 3);
    fs::remove_file(folder.join("bad.iwi")).unwrap();
    assert_eq!(reopened.scan(false).unwrap().entries.len(), 2);
    fs::remove_file(folder.join("custom/mod.IWI")).unwrap();
    assert_eq!(reopened.scan(false).unwrap().entries.len(), 1);
}

#[test]
fn loose_versions_group_with_matching_iwd_images_and_refresh_on_edit() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("game");
    let cache = fixture.path().join("cache");
    let folder = root.join("raw/images");
    fs::create_dir_all(root.join("main")).unwrap();
    fs::create_dir_all(&folder).unwrap();
    let bytes = bitmap(20);
    archive(
        &root.join("main/base.iwd"),
        &[("images/shared.iwi", &bytes)],
    );
    let loose = folder.join("shared.iwi");
    fs::write(&loose, &bytes).unwrap();

    let mut catalog = Catalog::new(root, cache);
    let index = catalog.scan(false).unwrap();
    assert_eq!(index.entries.len(), 1);
    assert!(matches!(
        &index.entries[0].sources[..],
        [Source::Archive { .. }, Source::Loose { .. }]
    ));

    let mut changed = bitmap(90);
    changed.push(0); // Also change the size to exercise invalidation on any filesystem.
    fs::write(&loose, changed).unwrap();
    let index = catalog.scan(false).unwrap();
    assert_eq!(index.entries.len(), 2);
    let version = index
        .entries
        .iter()
        .find(|entry| matches!(&entry.sources[..], [Source::Loose { .. }]))
        .unwrap();
    fs::write(&loose, bitmap(90)).unwrap();
    assert!(catalog.read(&version.id).unwrap_err().contains("changed"));
    let index = catalog.scan(false).unwrap();
    let version = index
        .entries
        .iter()
        .find(|entry| matches!(&entry.sources[..], [Source::Loose { .. }]))
        .unwrap();
    assert_eq!(catalog.pixels(&version.id, 0).unwrap(), [90, 0, 0, 255]);
}

#[test]
fn loose_images_work_without_main_archives() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("game");
    let folder = root.join("raw/images");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("only.iwi"), bitmap(110)).unwrap();

    let mut catalog = Catalog::new(root, fixture.path().join("cache"));
    let index = catalog.scan(false).unwrap();
    assert!(index.warnings.is_empty());
    assert_eq!(index.entries.len(), 1);
    assert_eq!(
        catalog.pixels(&index.entries[0].id, 0).unwrap(),
        [110, 0, 0, 255]
    );
}
