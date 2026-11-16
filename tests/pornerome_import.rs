use personal_downloaders::{Cancel, Site, sites::pornerome, state::State};
use rusqlite::{Connection, params};
use serde_json::json;
use sha2::{Digest, Sha256};
const PROFILE: &str = "https://pornerome.com/model/onlyfans/900005/fixture.creator.000";
const POST: &str = "https://pornerome.com/post/90000004/900005/onlyfans/fixture.creator.000";
const MEDIA: &str = "https://img6.pornerome.com/storage/6/uz/aj/test.jpg";

fn fixture(root: &std::path::Path, bytes: &[u8]) -> (Connection, personal_downloaders::File) {
    let f = pornerome::file(POST, MEDIA).unwrap();
    let target = root.join(&f.path);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, bytes).unwrap();
    let meta = root.join(".pornerome-dl");
    std::fs::create_dir_all(&meta).unwrap();
    let db = Connection::open(meta.join("state.sqlite3")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE downloads(path_key TEXT PRIMARY KEY,path TEXT,source TEXT,size INTEGER,sha256 TEXT,mtime_ns INTEGER,status TEXT,updated_at REAL); CREATE TABLE posts(url TEXT PRIMARY KEY,saved_at REAL,media TEXT); CREATE TABLE listings(url TEXT PRIMARY KEY,saved_at REAL,files TEXT,pending TEXT); CREATE TABLE legacy(path TEXT PRIMARY KEY,data TEXT,imported_at REAL);").unwrap();
    db.execute(
        "INSERT INTO downloads VALUES (?,?,?,?,?,?,?,?)",
        params![
            f.path.to_lowercase(),
            f.path,
            "/storage/6/uz/aj/test.jpg",
            4,
            format!("{:x}", Sha256::digest(b"test")),
            personal_downloaders::mtime(&target).unwrap(),
            "complete",
            personal_downloaders::now()
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO posts VALUES (?,?,?)",
        params![
            POST,
            personal_downloaders::now(),
            json!([MEDIA]).to_string()
        ],
    )
    .unwrap();
    db.execute("INSERT INTO legacy VALUES (?,?,?)",params!["fixture.creator.000/onlyfans_900005/manifests/profile.json",json!([{"post":POST,"url":MEDIA,"path":format!("fixture.creator.000/onlyfans_900005/90000004/{}",target.file_name().unwrap().to_str().unwrap())}]).to_string(),personal_downloaders::now()]).unwrap();
    (db, f)
}
#[test]
fn wal_backup_records_posts_and_archived_manifest_survive_import() {
    let root = tempfile::tempdir().unwrap();
    let (db, f) = fixture(root.path(), b"test");
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .import_legacy(Site::Pornerome, &Cancel::default())
        .unwrap();
    assert!(state.completed(&f, &Cancel::default()).unwrap());
    let mut alias = f.clone();
    alias.url = alias.url.replace("img6", "img9");
    assert!(state.completed(&alias, &Cancel::default()).unwrap());
    assert_eq!(
        state.listing(PROFILE).unwrap().unwrap().files[0].path,
        f.path
    );
    assert_eq!(state.post(POST).unwrap().unwrap(), vec![MEDIA]);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM legacy", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let backup = std::fs::read_dir(state.directory.join("rust-backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        Connection::open(backup)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM downloads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn mismatched_legacy_hash_is_preserved_but_not_imported_as_complete() {
    let root = tempfile::tempdir().unwrap();
    let (_db, f) = fixture(root.path(), b"xxxx");
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .import_legacy(Site::Pornerome, &Cancel::default())
        .unwrap();
    assert!(!state.completed(&f, &Cancel::default()).unwrap());
    assert_eq!(std::fs::read(root.path().join(&f.path)).unwrap(), b"xxxx");
}

#[test]
fn archived_pending_posts_are_not_hidden_by_a_successful_manifest() {
    let root = tempfile::tempdir().unwrap();
    let (db, _) = fixture(root.path(), b"test");
    let pending = POST.replace("90000004", "75060536");
    db.execute(
        "INSERT INTO legacy VALUES (?,?,?)",
        params![
            "fixture.creator.000/onlyfans_900005/manifests/profile.pending.json",
            json!([{"url":pending,"error":"HTTP 503"}]).to_string(),
            personal_downloaders::now()
        ],
    )
    .unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .import_legacy(Site::Pornerome, &Cancel::default())
        .unwrap();
    let listing = state.listing(PROFILE).unwrap().unwrap();
    assert_eq!(listing.pending.len(), 1);
    assert_eq!(listing.pending[0].url, pending);
}
