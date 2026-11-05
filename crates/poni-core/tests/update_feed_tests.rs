use poni_core::update::{parse_file, parse_payload};
use std::fs;
use std::path::Path;

#[test]
fn update_feed_file_is_valid_and_trusted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let feed_path = root.join("site").join("latest.json");
    if !feed_path.exists() {
        return;
    }
    let content = fs::read_to_string(&feed_path).expect("read site/latest.json");
    let signed = parse_file(&content).expect("parse site/latest.json signed envelope");
    let release = parse_payload(&signed.payload).expect("parse site/latest.json release payload");

    assert!(!release.version.is_empty(), "version must not be empty");
    assert!(!release.notes.is_empty(), "notes must not be empty");
    assert!(release.windows.is_some(), "windows download must be present");
    assert!(release.mac.is_some(), "mac download must be present");

    // Guard: site/latest.json MUST match the current workspace version so that
    // releases never go out with an outdated update feed.
    assert_eq!(
        release.version,
        env!("CARGO_PKG_VERSION"),
        "site/latest.json version must match Cargo package version"
    );
}
