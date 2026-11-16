use personal_downloaders::{Cancel, Site, organize, sites::pornerome, state::State};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
fn quiet() -> personal_downloaders::Log {
    Arc::new(|_| {})
}
const POST: &str = "https://pornerome.com/post/90000004/900005/onlyfans/fixture.creator.000";
const MEDIA: &str = "https://img6.pornerome.com/storage/6/uz/aj/test.jpg";
fn fixture(
    root: &std::path::Path,
    old_style: bool,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    personal_downloaders::File,
) {
    let f = pornerome::file(POST, MEDIA).unwrap();
    let name = f.path.rsplit('/').next().unwrap();
    let folder = if old_style {
        "onlyfans_900005_fixture.creator.000/90000004"
    } else {
        "fixture.creator.000/onlyfans_900005/90000004"
    };
    let old = root.join(folder).join(name);
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, b"test").unwrap();
    let receipt = old.with_file_name(format!("{name}.complete.json"));
    std::fs::write(
        &receipt,
        json!({"size":4,"source":"/storage/6/uz/aj/test.jpg"}).to_string(),
    )
    .unwrap();
    (old, receipt, f)
}
#[test]
fn both_legacy_layouts_are_flattened_receipts_archived_and_second_run_is_idempotent() {
    for style in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (old, receipt, f) = fixture(root.path(), style);
        let state = State::open(root.path(), Site::Pornerome).unwrap();
        let summary = organize::run_locked(&state, &Cancel::default(), &quiet()).unwrap();
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.downloaded, 1);
        assert!(!old.exists());
        assert!(!receipt.exists());
        assert_eq!(std::fs::read(root.path().join(&f.path)).unwrap(), b"test");
        assert!(state.completed(&f, &Cancel::default()).unwrap());
        assert_eq!(
            std::fs::read_dir(state.directory.join("imported-json"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            organize::run_locked(&state, &Cancel::default(), &quiet())
                .unwrap()
                .downloaded,
            0
        );
    }
}
#[test]
fn conflicting_target_and_incorrect_size_preserve_both_originals() {
    for bad_size in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (old, receipt, f) = fixture(root.path(), false);
        let state = State::open(root.path(), Site::Pornerome).unwrap();
        let target = root.path().join(&f.path);
        if bad_size {
            std::fs::write(&old, b"short").unwrap();
        } else {
            std::fs::write(&target, b"different").unwrap();
        }
        let summary = organize::run_locked(&state, &Cancel::default(), &quiet()).unwrap();
        assert_eq!(summary.failed, 1);
        assert!(old.exists());
        assert!(receipt.exists());
        if !bad_size {
            assert_eq!(std::fs::read(target).unwrap(), b"different");
        }
        assert!(!state.completed(&f, &Cancel::default()).unwrap());
    }
}
#[test]
fn interrupted_rename_recovers_only_using_the_recorded_hash() {
    let root = tempfile::tempdir().unwrap();
    let (old, receipt, f) = fixture(root.path(), false);
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .record(
            &f,
            4,
            &format!("{:x}", Sha256::digest(b"test")),
            personal_downloaders::mtime(&old).unwrap(),
            "moving",
        )
        .unwrap();
    std::fs::rename(old, root.path().join(&f.path)).unwrap();
    assert_eq!(
        organize::run_locked(&state, &Cancel::default(), &quiet())
            .unwrap()
            .failed,
        0
    );
    assert!(!receipt.exists());
    assert!(state.completed(&f, &Cancel::default()).unwrap());
}
#[test]
fn unknown_metadata_and_incomplete_data_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    std::fs::write(root.path().join("personal.json"), b"keep").unwrap();
    let part = root
        .path()
        .join("fixture.creator.000/onlyfans_900005/90000004/unfinished.jpg.part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, b"partial").unwrap();
    organize::run_locked(&state, &Cancel::default(), &quiet()).unwrap();
    assert_eq!(
        std::fs::read(root.path().join("personal.json")).unwrap(),
        b"keep"
    );
    assert!(!part.exists());
    assert_eq!(
        std::fs::read_dir(state.directory.join("legacy-parts"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn organizing_a_live_legacy_manifest_restores_its_offline_listing_and_pending_posts() {
    let root = tempfile::tempdir().unwrap();
    let (_old, _receipt, f) = fixture(root.path(), false);
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .import_legacy(Site::Pornerome, &Cancel::default())
        .unwrap();
    let dir = root
        .path()
        .join("fixture.creator.000/onlyfans_900005/manifests");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("profile.json"),
        json!([{"post":POST,"url":MEDIA,"path":f.path}]).to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("profile.pending.json"),
        json!([{"url":POST.replace("90000004","75060536"),"error":"503"}]).to_string(),
    )
    .unwrap();
    organize::run_locked(&state, &Cancel::default(), &quiet()).unwrap();
    let listing = state
        .listing("https://pornerome.com/model/onlyfans/900005/fixture.creator.000")
        .unwrap()
        .expect("offline listing survives organization");
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.pending.len(), 1);
}

#[test]
fn top_level_pending_manifest_is_restored_with_its_original_listing() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .import_legacy(Site::Pornerome, &Cancel::default())
        .unwrap();
    std::fs::write(
        root.path().join("manifest.json"),
        json!([{"post":POST,"url":MEDIA,"path":"old/path.jpg"}]).to_string(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("manifest.pending.json"),
        json!([{"url":POST.replace("90000004","75060536"),"error":"503"}]).to_string(),
    )
    .unwrap();
    organize::run_locked(&state, &Cancel::default(), &quiet()).unwrap();
    let listing = state
        .listing("https://pornerome.com/model/onlyfans/900005/fixture.creator.000")
        .unwrap()
        .unwrap();
    assert_eq!(listing.pending.len(), 1);
    assert!(!root.path().join("manifest.pending.json").exists());
}
