use crate::{Cancel, File, Listing, Pending, state::State};
use anyhow::{Result, bail};
use futures_util::{StreamExt, stream};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    sync::LazyLock,
};
use url::Url;
static PAGE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^/(?:model/[A-Za-z0-9._-]+/\d+/[^/]+|post/\d+/\d+/[A-Za-z0-9._-]+/[^/]+)/?$",
    )
    .unwrap()
});
static MEDIA_HOST: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^(?:img|vid|video)\d*\.pornerome\.com$").unwrap());

pub fn normalize(input: &str) -> Result<String> {
    let mut url = Url::parse(input)?;
    if url.scheme() != "https"
        || url.host_str() != Some("pornerome.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !PAGE.is_match(url.path())
    {
        bail!("Introduce un perfil o post HTTPS de Pornerome")
    }
    let pages: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "page")
        .map(|(_, v)| v.into_owned())
        .collect();
    if pages.len() > 1 {
        bail!("Paginación duplicada")
    }
    let page = pages.first().map(|s| s.parse::<u32>()).transpose()?;
    if page == Some(0) {
        bail!("Página inválida")
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    url.set_fragment(None);
    url.set_query(None);
    if path.starts_with("/model/")
        && let Some(page) = page
        && page > 1
    {
        url.set_query(Some(&format!("page={page}")));
    }
    Ok(url.to_string())
}
pub fn profile_url(input: &str) -> Result<String> {
    let normalized = normalize(input)?;
    let url = Url::parse(&normalized)?;
    let p: Vec<_> = url.path_segments().unwrap().collect();
    Ok(if p[0] == "model" {
        format!("https://pornerome.com/model/{}/{}/{}", p[1], p[2], p[3])
    } else {
        format!("https://pornerome.com/model/{}/{}/{}", p[3], p[2], p[4])
    })
}
pub fn model_name(input: &str) -> Result<String> {
    let normalized = profile_url(input)?;
    let url = Url::parse(&normalized)?;
    Ok(legacy_name(
        url.path_segments().unwrap().next_back().unwrap(),
    ))
}
pub fn legacy_name(input: &str) -> String {
    let value: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let mut value = value.trim_matches(['.', ' ']).to_owned();
    if value.is_empty() {
        value = "modelo".into()
    }
    if value.eq_ignore_ascii_case("pornerome-dl") && input.starts_with('.') {
        return "_pornerome-dl".into();
    }
    crate::safe_name(&value)
}
pub fn validate_media(url: &Url, post: &str) -> Result<()> {
    normalize(post)?;
    let valid_host = url
        .host_str()
        .is_some_and(|h| h == "pornerome.com" || MEDIA_HOST.is_match(h));
    let ext = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if url.scheme() != "https"
        || !valid_host
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !url.path().starts_with("/storage/")
        || ![
            "jpg", "jpeg", "png", "gif", "webp", "avif", "mp4", "m4v", "webm", "mov", "mkv",
        ]
        .contains(&ext.as_str())
    {
        bail!("Medio de Pornerome no reconocido")
    }
    Ok(())
}
pub fn validate_redirect(url: &Url, original: &Url, post: &str) -> Result<()> {
    validate_media(original, post)?;
    validate_media(url, post)?;
    if url.path() != original.path() {
        bail!("La redirección cambia el archivo de Pornerome")
    }
    Ok(())
}
pub fn file(post: &str, media: &str) -> Result<File> {
    let post = normalize(post)?;
    let url = Url::parse(media)?;
    validate_media(&url, &post)?;
    let hash = format!("{:x}", Sha256::digest(url.path().as_bytes()));
    let ext = url.path().rsplit('.').next().unwrap().to_ascii_lowercase();
    Ok(File {
        post: post.clone(),
        url: url.to_string(),
        path: format!("{}/{}.{}", model_name(&post)?, &hash[..20], ext),
        ..Default::default()
    })
}
#[derive(Default)]
pub struct Page {
    pub posts: Vec<String>,
    pub pages: Vec<String>,
    pub media: Vec<String>,
}
pub fn parse(html: &str, input: &str) -> Result<Page> {
    let input = normalize(input)?;
    let base = Url::parse(&input)?;
    let profile = profile_url(&input)?;
    let doc = Html::parse_document(html);
    let mut result = Page::default();
    if base.path().starts_with("/post/") {
        let body = doc
            .select(&Selector::parse(".post-body").unwrap())
            .next()
            .ok_or_else(|| anyhow::anyhow!("Página sin post-body; revisa el acceso"))?;
        for node in body.select(&Selector::parse("img,video,source,a[href]").unwrap()) {
            let tag = node.value().name();
            if tag == "img"
                && node
                    .ancestors()
                    .filter_map(scraper::ElementRef::wrap)
                    .any(|n| n.value().name() == "video")
            {
                continue;
            }
            let candidates = if tag == "a" {
                vec![node.attr("href")]
            } else {
                vec![node.attr("data-src"), node.attr("src")]
            };
            for value in candidates.into_iter().flatten() {
                if let Ok(url) = base.join(value)
                    && validate_media(&url, &input).is_ok()
                {
                    result.media.push(url.to_string());
                    break;
                }
            }
        }
        if result.media.is_empty() {
            bail!("No se encontraron archivos originales en post-body")
        }
    } else {
        for (selector, posts) in [
            (".posts-list a[href]", true),
            (".pagination a[href]", false),
        ] {
            for node in doc.select(&Selector::parse(selector).unwrap()) {
                if let Some(href) = node.attr("href")
                    && let Ok(url) = base.join(href)
                    && let Ok(url) = normalize(url.as_str())
                    && profile_url(&url)? == profile
                {
                    let path = Url::parse(&url)?.path().to_owned();
                    if posts && path.starts_with("/post/") {
                        result.posts.push(url)
                    } else if !posts && path.starts_with("/model/") {
                        result.pages.push(url)
                    }
                }
            }
        }
        if result.posts.is_empty() {
            bail!("Perfil sin posts reconocidos; no se interpreta como descargado")
        }
    }
    for values in [&mut result.posts, &mut result.pages, &mut result.media] {
        let mut seen = HashSet::new();
        values.retain(|s| seen.insert(s.clone()));
    }
    Ok(result)
}
fn merge(old: Vec<File>, new: Vec<File>) -> Result<Vec<File>> {
    let mut values = Vec::new();
    let mut positions = HashMap::new();
    for f in old.into_iter().chain(new) {
        let key = Url::parse(&f.url)?.path().to_owned();
        if let Some(&index) = positions.get(&key) {
            values[index] = f;
        } else {
            positions.insert(key, values.len());
            values.push(f);
        }
    }
    Ok(values)
}
pub async fn resolve<F, Fut>(
    state: &State,
    input: &str,
    refresh: bool,
    cancel: &Cancel,
    get: &F,
) -> Result<Listing>
where
    F: Fn(String) -> Fut + Sync,
    Fut: Future<Output = Result<String>>,
{
    let input = normalize(input)?;
    let profile = profile_url(&input)?;
    let previous = state.listing(&input)?;
    if let Some(saved) = &previous {
        for f in &saved.files {
            if profile_url(&f.post)? != profile {
                bail!("El listado guardado mezcla perfiles")
            };
            validate_media(&Url::parse(&f.url)?, &f.post)?;
            state.target(&f.path)?;
        }
        if !refresh && !saved.files.is_empty() && saved.pending.is_empty() {
            return Ok(saved.clone());
        }
    }
    let mut listing = Listing {
        title: model_name(&input)?,
        ..Default::default()
    };
    let tasks = if !refresh {
        previous
            .as_ref()
            .filter(|s| !s.files.is_empty())
            .map(|s| s.pending.iter().map(|p| p.url.clone()).collect())
            .unwrap_or_else(|| vec![input.clone()])
    } else {
        vec![input.clone()]
    };
    let mut posts = Vec::new();
    let mut pages = VecDeque::from(tasks);
    let mut visited = HashSet::new();
    while let Some(url) = pages.pop_front() {
        cancel.check()?;
        let url = normalize(&url)?;
        if profile_url(&url)? != profile {
            bail!("El pendiente pertenece a otro perfil")
        };
        if !visited.insert(url.clone()) {
            continue;
        }
        if visited.len() > 1000 {
            bail!("Se alcanzaron 1000 páginas")
        }
        if Url::parse(&url)?.path().starts_with("/post/") {
            posts.push(url);
            continue;
        }
        match get(url.clone()).await.and_then(|html| parse(&html, &url)) {
            Ok(page) => {
                posts.extend(page.posts);
                pages.extend(page.pages);
            }
            Err(error) => {
                cancel.check()?;
                listing.pending.push(Pending {
                    url,
                    error: error.to_string(),
                });
            }
        }
    }
    let mut seen = HashSet::new();
    posts.retain(|p| seen.insert(p.clone()));
    let mut results = stream::iter(
        posts
            .into_iter()
            .enumerate()
            .map(|(index, post)| async move {
                let result: Result<Vec<String>> = async {
                    cancel.check()?;
                    if !refresh
                        && let Some(media) = state.post(&post)?
                        && !media.is_empty()
                    {
                        for url in &media {
                            validate_media(&Url::parse(url)?, &post)?;
                        }
                        return Ok(media);
                    }
                    let media = parse(&get(post.clone()).await?, &post)?.media;
                    cancel.check()?;
                    state.save_post(&post, &media)?;
                    Ok(media)
                }
                .await;
                (index, post, result)
            }),
    )
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;
    results.sort_by_key(|r| r.0);
    for (_, post, result) in results {
        cancel.check()?;
        match result {
            Ok(media) => {
                for url in media {
                    listing.files.push(file(&post, &url)?);
                }
            }
            Err(error) => listing.pending.push(Pending {
                url: post,
                error: error.to_string(),
            }),
        }
    }
    listing.files = merge(Vec::new(), listing.files)?;
    if listing.files.is_empty()
        && (previous.is_none() || (refresh && listing.pending.iter().any(|p| p.url == input)))
    {
        bail!("No se pudo leer Pornerome; se conserva el listado anterior")
    }
    if (!refresh || !listing.pending.is_empty())
        && let Some(saved) = previous
    {
        listing.files = merge(saved.files, listing.files)?;
    }
    cancel.check()?;
    state.save_listing(&input, &listing)?;
    Ok(listing)
}
