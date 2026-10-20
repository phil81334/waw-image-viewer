use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use waw_image_viewer::iwi;

#[test]
fn compare_every_audited_image_and_mip() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.local");
    let references: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("verification/reference-pixels.json")).unwrap())
            .unwrap();
    let mut errors = Vec::new();
    let mut compared = 0;
    for reference in references.as_array().unwrap() {
        let hash = reference["sha256"].as_str().unwrap();
        let name = reference["name"].as_str().unwrap();
        let bytes =
            fs::read(root.join(format!("research/image-audit/samples/{hash}.iwi"))).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        match iwi::decode(&bytes) {
            Err(error) => errors.push(format!("{name}: {error}")),
            Ok(levels) => {
                for check in reference["checks"].as_array().unwrap() {
                    let mip = check["mip"].as_u64().unwrap() as usize;
                    let face = check["face"].as_u64().unwrap() as usize;
                    let actual = format!("{:x}", Sha256::digest(&levels[mip].faces[face]));
                    if actual != check["sha256"].as_str().unwrap() {
                        errors.push(format!("{name} mip {mip} face {face}"));
                    }
                    compared += 1;
                }
            }
        }
    }
    fs::write(
        root.join("verification/rust-decoder-results.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"compared":compared,"errors":errors}))
            .unwrap(),
    )
    .unwrap();
    assert!(
        errors.is_empty(),
        "{} differences; first: {:?}",
        errors.len(),
        errors.first()
    );
    println!("Matched {compared} face/mip images.");
}

#[test]
fn rejects_malformed_images() {
    for length in 0..28 {
        assert!(iwi::decode(&vec![0; length]).is_err());
    }
    let mut data = vec![0; 28];
    data[..4].copy_from_slice(b"IWi\x06");
    assert!(iwi::decode(&data).is_err());
    data[4] = 13;
    data[6] = 4;
    data[8] = 4;
    data[10] = 1;
    assert!(iwi::decode(&data).is_err());
}
