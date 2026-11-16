use personal_downloaders::{
    Cancel, Site,
    sites::{self, pornerome},
    state::{State, source_key},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use url::Url;
const PROFILE: &str = "https://pornerome.com/model/onlyfans/900005/fixture.creator.000";
const POST: &str = "https://pornerome.com/post/90000004/900005/onlyfans/fixture.creator.000";
const MEDIA: &str = "https://img6.pornerome.com/storage/6/uz/aj/test.jpg";

#[test]
fn urls_and_media_are_scoped_to_the_profile_and_storage_hosts() {
    assert_eq!(
        sites::inputs(Site::Pornerome, &format!("{PROFILE}/ {PROFILE}")).unwrap(),
        vec![PROFILE]
    );
    assert!(pornerome::normalize(&PROFILE.replace("https://", "https://user:secret@")).is_err());
    assert!(
        pornerome::validate_media(
            &Url::parse("https://evil.test/storage/test.jpg").unwrap(),
            POST
        )
        .is_err()
    );
    let original = Url::parse(MEDIA).unwrap();
    assert!(
        pornerome::validate_redirect(
            &Url::parse(&MEDIA.replace("img6", "img9")).unwrap(),
            &original,
            POST
        )
        .is_ok()
    );
    assert!(
        pornerome::validate_redirect(
            &Url::parse(&MEDIA.replace("test.jpg", "other.jpg")).unwrap(),
            &original,
            POST
        )
        .is_err()
    );
    let mut f = pornerome::file(POST, MEDIA).unwrap();
    let identity = source_key(&f).unwrap();
    f.url = f.url.replace("img6", "img9");
    assert_eq!(source_key(&f).unwrap(), identity);
}
#[test]
fn originals_exclude_posters_recommendations_and_profile_thumbnails() {
    let html = format!(
        r#"<img src="{MEDIA}"><div class="post-body"><img data-src="{MEDIA}"><video poster="https://img1.pornerome.com/storage/poster.jpg"><source src="https://vid1.pornerome.com/storage/video.m4v"></video></div><aside><img src="https://img1.pornerome.com/storage/avatar.jpg"></aside>"#
    );
    let page = pornerome::parse(&html, POST).unwrap();
    assert_eq!(
        page.media,
        vec![MEDIA, "https://vid1.pornerome.com/storage/video.m4v"]
    );
    assert!(pornerome::parse("<html>Access denied</html>", POST).is_err());
}
#[tokio::test]
async fn pagination_deduplicates_cdn_aliases_without_fetching_other_profiles() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    let other = POST.replace("90000004", "75060536");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let listing=pornerome::resolve(&state,PROFILE,false,&Cancel::default(),&move|url:String| {
        seen.lock().unwrap().push(url.clone());let second=other.clone();async move {Ok(if url==PROFILE {format!(r#"<div class="posts-list"><a href="{POST}">post</a><a href="https://pornerome.com/post/1/7/onlyfans/other">other</a></div><div class="pagination"><a href="{PROFILE}?page=2">2</a></div>"#)}else if url.contains("?page=2") {format!("<div class='posts-list'><a href='{second}'>post</a></div>")}else{format!("<div class='post-body'><img src='{}'></div>",if url==POST {MEDIA.to_owned()}else{MEDIA.replace("img6","img9")})})}
    }).await.unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert!(listing.pending.is_empty());
}
#[tokio::test]
async fn one_failed_post_among_580_keeps_successes_and_only_retries_that_post() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let listing=pornerome::resolve(&state,PROFILE,false,&Cancel::default(),&move|url:String| {
        seen.fetch_add(1,Ordering::SeqCst);async move {
            if url==PROFILE {return Ok(format!("<div class='posts-list'>{}</div>",(0..580).map(|i|format!("<a href='https://pornerome.com/post/{}/900005/onlyfans/fixture.creator.000'>post</a>",80000000+i)).collect::<String>()))}
            let id=Url::parse(&url).unwrap().path_segments().unwrap().nth(1).unwrap().parse::<u64>().unwrap();
            if id==80000102 {anyhow::bail!("HTTP 503")}
            Ok(format!("<div class='post-body'><img src='https://img1.pornerome.com/storage/test/{id}.jpg'></div>"))
        }
    }).await.unwrap();
    assert_eq!(listing.files.len(), 579);
    assert_eq!(listing.pending.len(), 1);
    assert_eq!(count.load(Ordering::SeqCst), 581);
    let retries = AtomicUsize::new(0);
    let recovered=pornerome::resolve(&state,PROFILE,false,&Cancel::default(),&|url:String| {
        retries.fetch_add(1,Ordering::SeqCst);async move {assert!(url.contains("80000102"));Ok("<div class='post-body'><img src='https://img1.pornerome.com/storage/test/80000102.jpg'></div>".into())}
    }).await.unwrap();
    assert_eq!(recovered.files.len(), 580);
    assert!(recovered.pending.is_empty());
    assert_eq!(retries.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn failed_refresh_does_not_replace_a_previous_complete_listing() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path(), Site::Pornerome).unwrap();
    state
        .save_listing(
            PROFILE,
            &personal_downloaders::Listing {
                title: "fixture.creator.000".into(),
                files: vec![pornerome::file(POST, MEDIA).unwrap()],
                pending: vec![],
            },
        )
        .unwrap();
    assert!(
        pornerome::resolve(
            &state,
            PROFILE,
            true,
            &Cancel::default(),
            &|_: String| async { anyhow::bail!("blocked") }
        )
        .await
        .is_err()
    );
    assert_eq!(state.listing(PROFILE).unwrap().unwrap().files.len(), 1);
}
