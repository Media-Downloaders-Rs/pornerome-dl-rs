use crate::{
    Cancel, File, Listing, Pending,
    sites::pornerome,
    state::{State, hash_file},
};
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use url::Url;

pub(crate) fn legacy_file(relative: &str, source: &str) -> Result<File> {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() != 2 || relative.contains('\\') {
        bail!("El registro anterior no usa una ruta plana reconocida")
    }
    let post = format!("https://pornerome.com/model/legacy/1/{}", parts[0]);
    let media = format!("https://img1.pornerome.com{source}");
    let url = Url::parse(&media)?;
    if url.path() != source || url.query().is_some() || url.fragment().is_some() {
        bail!("Origen anterior inválido")
    }
    let file = pornerome::file(&post, &media)?;
    if file.path != relative {
        bail!("El nombre anterior no corresponde al origen")
    }
    Ok(file)
}
impl State {
    fn has_table(&self, name: &str) -> Result<bool> {
        Ok(self.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            [name],
            |r| r.get(0),
        )?)
    }
    pub(crate) fn import_pornerome(&self, cancel: &Cancel) -> Result<()> {
        let done: Option<String> = self
            .connection()?
            .query_row(
                "SELECT value FROM rust_meta WHERE key='pornerome_legacy_imported'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if done.is_some() {
            return Ok(());
        }
        if self.has_table("downloads")? {
            let rows = {
                let db = self.connection()?;
                let mut statement =
                    db.prepare("SELECT path,source,size,sha256,status FROM downloads")?;
                statement
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (relative, source, size, hash, status) in rows {
                cancel.check()?;
                let file = match legacy_file(&relative, &source) {
                    Ok(file) => file,
                    Err(_) => continue,
                };
                let path = self.target(&relative)?;
                if size <= 0
                    || !matches!(status.as_str(), "complete" | "moving")
                    || self.completed(&file, cancel)?
                    || !path.is_file()
                    || path.metadata()?.len() != size as u64
                {
                    continue;
                }
                if hash_file(&path, cancel)? == hash {
                    self.record(&file, size as u64, &hash, crate::mtime(&path)?, "complete")?;
                }
            }
        }
        if self.has_table("posts")? {
            self.connection()?.execute(
                "INSERT OR IGNORE INTO rust_posts SELECT url,media,saved_at FROM posts",
                [],
            )?;
        }
        if self.has_table("listings")? {
            let rows = {
                let db = self.connection()?;
                let mut statement = db.prepare("SELECT url,files,pending FROM listings")?;
                statement
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (url, files, pending) in rows {
                self.import_pornerome_listing(
                    &url,
                    serde_json::from_str(&files)?,
                    serde_json::from_str(&pending)?,
                )?;
            }
        }
        self.restore_pornerome_manifests(false, cancel)?;
        cancel.check()?;
        self.connection()?.execute(
            "INSERT OR REPLACE INTO rust_meta VALUES ('pornerome_legacy_imported','1')",
            [],
        )?;
        Ok(())
    }
    pub(crate) fn restore_pornerome_manifests(
        &self,
        merge_existing: bool,
        cancel: &Cancel,
    ) -> Result<()> {
        use std::collections::{BTreeMap, HashMap, HashSet};
        let mut rows = BTreeMap::new();
        for table in ["legacy", "rust_legacy"] {
            if !self.has_table(table)? {
                continue;
            }
            let values = {
                let db = self.connection()?;
                let mut stmt = db.prepare(&format!(
                    "SELECT path,data FROM {table} WHERE path LIKE '%manifest%.json'"
                ))?;
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            rows.extend(values);
        }
        let mut failures = HashMap::<String, Vec<Pending>>::new();
        for (path, data) in &rows {
            if let Some(stem) = path.strip_suffix(".pending.json") {
                failures.insert(format!("{stem}.json"), serde_json::from_str(data)?);
            }
        }
        let mut groups = BTreeMap::<String, (Vec<File>, Vec<Pending>)>::new();
        let input = |path: &str, post: &str| -> Result<String> {
            if path
                .rsplit('/')
                .next()
                .is_some_and(|p| p.starts_with("post-"))
            {
                pornerome::normalize(post)
            } else {
                pornerome::profile_url(post)
            }
        };
        for (path, data) in rows {
            cancel.check()?;
            if path.ends_with(".pending.json") {
                continue;
            }
            let files: Vec<File> = serde_json::from_str(&data)?;
            let pending = failures.remove(&path).unwrap_or_default();
            let first = files
                .first()
                .map(|f| f.post.as_str())
                .or_else(|| pending.first().map(|p| p.url.as_str()));
            if let Some(first) = first {
                let group = groups.entry(input(&path, first)?).or_default();
                group.0.extend(files);
                group.1.extend(pending);
            }
        }
        for (path, pending) in failures {
            if let Some(first) = pending.first() {
                groups
                    .entry(input(&path, &first.url)?)
                    .or_default()
                    .1
                    .extend(pending);
            }
        }
        for (url, (mut files, mut pending)) in groups {
            cancel.check()?;
            if let Some(saved) = self.listing(&url)? {
                if !merge_existing {
                    continue;
                }
                files.extend(saved.files);
                pending.extend(saved.pending);
            }
            let profile = pornerome::profile_url(&url)?;
            for f in &files {
                if pornerome::profile_url(&f.post)? != profile {
                    bail!("El manifiesto mezcla perfiles")
                }
            }
            let mut seen = HashSet::new();
            pending.retain(|p| seen.insert(p.url.clone()));
            self.write_pornerome_listing(&url, files, pending)?;
        }
        Ok(())
    }
    fn import_pornerome_listing(
        &self,
        input: &str,
        files: Vec<File>,
        pending: Vec<Pending>,
    ) -> Result<()> {
        let url = pornerome::normalize(input)?;
        if self.listing(&url)?.is_some() {
            return Ok(());
        }
        self.write_pornerome_listing(&url, files, pending)
    }
    fn write_pornerome_listing(
        &self,
        input: &str,
        files: Vec<File>,
        pending: Vec<Pending>,
    ) -> Result<()> {
        let url = pornerome::normalize(input)?;
        let profile = pornerome::profile_url(&url)?;
        let mut normalized = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for file in files {
            if pornerome::profile_url(&file.post)? != profile {
                bail!("Un manifiesto anterior mezcla perfiles")
            }
            let file = pornerome::file(&file.post, &file.url)?;
            self.target(&file.path)?;
            if seen.insert(Url::parse(&file.url)?.path().to_owned()) {
                normalized.push(file)
            }
        }
        for failure in &pending {
            if pornerome::profile_url(&failure.url)? != profile {
                bail!("Pendiente anterior de otro perfil")
            }
        }
        if !normalized.is_empty() || !pending.is_empty() {
            self.save_listing(
                &url,
                &Listing {
                    title: pornerome::model_name(&url)?,
                    files: normalized,
                    pending,
                },
            )?;
        }
        Ok(())
    }
    pub(crate) fn archive_pornerome(&self, path: &str, data: &Value) -> Result<()> {
        self.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS rust_legacy (path TEXT PRIMARY KEY,data TEXT NOT NULL,saved_at REAL NOT NULL)")?;
        self.connection()?.execute(
            "INSERT OR REPLACE INTO rust_legacy VALUES (?,?,?)",
            params![path, serde_json::to_string(data)?, crate::now()],
        )?;
        Ok(())
    }
}
