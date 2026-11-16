//! Explicit offline conversion of the two retired Pornerome layouts.
use crate::{
    Cancel, Log, Site,
    engine::Summary,
    pornerome_state::legacy_file,
    sites::pornerome,
    state::{State, hash_file},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn require_legacy_idle() -> Result<()> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let script = format!(
            r#"$ErrorActionPreference='Stop'; $root='D:\repos\pornerome-dl'; $active=@(Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne {} -and ( $_.Name -match '^pornerome.*\.exe$' -or ($_.Name -match '^python(w)?\.exe$' -and (($_.ExecutablePath -and $_.ExecutablePath.StartsWith($root+'\',[StringComparison]::OrdinalIgnoreCase)) -or $_.CommandLine -match 'pornerome[^\s"/\\]*\.py' -or $_.CommandLine -like ('*'+$root+'\*')))) }}); if ($active.Count -ne 0) {{exit 2}}"#,
            std::process::id()
        );
        let status = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            bail!(
                "Cierra Pornerome anterior y las otras ventanas de Pornerome antes de organizar; no se ha movido nada"
            )
        }
    }
    Ok(())
}
pub fn organize(root: &Path, cancel: Cancel, log: Log) -> Result<Summary> {
    require_legacy_idle()?;
    let state = State::open(root, Site::Pornerome)?;
    state.import_legacy(Site::Pornerome, &cancel)?;
    run_locked(&state, &cancel, &log)
}
fn relative(root: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/"))
}
fn scan(
    root: &Path,
    directory: &Path,
    metadata: &Path,
    files: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<()> {
    if depth > 40 {
        bail!("La estructura anterior supera 40 niveles")
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path == metadata {
            continue;
        }
        let kind = entry.file_type()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if entry.metadata()?.file_attributes() & 0x400 != 0 {
                bail!("No se reorganizan carpetas con enlaces")
            }
        }
        if kind.is_symlink() {
            bail!("No se reorganizan enlaces")
        }
        crate::target(root, &relative(root, &path)?)?;
        if kind.is_dir() {
            scan(root, &path, metadata, files, depth + 1)?
        } else if kind.is_file() {
            files.push(path)
        }
    }
    Ok(())
}
fn legacy_target(relative: &str) -> Option<String> {
    let p: Vec<_> = relative.split('/').collect();
    let name = if p.len() == 4
        && p[2].bytes().all(|b| b.is_ascii_digit())
        && regex::Regex::new(r"^[A-Za-z0-9._-]+_\d+$")
            .ok()?
            .is_match(p[1])
    {
        p[0].to_owned()
    } else if p.len() == 3 && p[1].bytes().all(|b| b.is_ascii_digit()) {
        regex::Regex::new(r"^[A-Za-z0-9.-]+_\d+_(.+)$")
            .ok()?
            .captures(p[0])?[1]
            .to_owned()
    } else {
        return None;
    };
    Some(format!("{}/{}", pornerome::legacy_name(&name), p.last()?))
}
fn archive_path(state: &State, relative: &str, folder: &str, extension: &str) -> Result<PathBuf> {
    let hash = format!("{:x}", Sha256::digest(relative.as_bytes()));
    let technical = format!(".pornerome-dl/{folder}/{hash}.{extension}");
    let target = crate::target(&state.root, &technical)?;
    fs::create_dir_all(target.parent().unwrap())?;
    if target.exists() {
        bail!("Ya existe un respaldo de {relative}; se conserva el original")
    }
    Ok(target)
}
fn archive_json(state: &State, path: &Path, value: &Value) -> Result<()> {
    let relative = relative(&state.root, path)?;
    let archive = archive_path(state, &relative, "imported-json", "json")?;
    state.archive_pornerome(&relative, value)?;
    fs::rename(path, archive)?;
    Ok(())
}
pub fn run_locked(state: &State, cancel: &Cancel, log: &Log) -> Result<Summary> {
    let mut files = Vec::new();
    scan(&state.root, &state.root, &state.directory, &mut files, 0)?;
    let mut result = Summary::default();
    let mut empty_candidates = Vec::new();
    for receipt in files
        .iter()
        .filter(|p| p.to_string_lossy().ends_with(".complete.json"))
    {
        cancel.check()?;
        let attempt = (|| -> Result<()> {
            let receipt_name = receipt
                .file_name()
                .unwrap()
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Nombre no UTF-8"))?;
            let old = receipt.with_file_name(receipt_name.trim_end_matches(".complete.json"));
            let old_relative = relative(&state.root, &old)?;
            let Some(target_relative) = legacy_target(&old_relative) else {
                return Ok(());
            };
            let value: Value = serde_json::from_slice(&fs::read(receipt)?)?;
            let source = value["source"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Constancia sin origen"))?;
            let size = value["size"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| anyhow::anyhow!("Tamaño inválido"))?;
            let mut file = legacy_file(&target_relative, source)?;
            file.expected_size = Some(size);
            let target = state.target(&target_relative)?;
            crate::target(&state.root, &old_relative)?;
            if old.is_file() {
                if old.metadata()?.len() != size {
                    bail!("Tamaño diferente; se conserva el original")
                }
                let hash = hash_file(&old, cancel)?;
                if target.exists() && hash_file(&target, cancel)? != hash {
                    bail!("Destino diferente; se conservan ambos archivos")
                }
                state.record(&file, size, &hash, crate::mtime(&old)?, "moving")?;
                fs::create_dir_all(target.parent().unwrap())?;
                cancel.check()?;
                if target.exists() {
                    let backup = archive_path(
                        state,
                        &old_relative,
                        "legacy-duplicates",
                        old.extension().and_then(|s| s.to_str()).unwrap_or("bin"),
                    )?;
                    fs::rename(&old, backup)?;
                } else {
                    fs::rename(&old, &target)?;
                }
                state.record(&file, size, &hash, crate::mtime(&target)?, "complete")?;
                result.downloaded += 1;
            } else if !state.completed(&file, cancel)? {
                bail!("Falta el original y no existe una finalización verificable")
            }
            archive_json(state, receipt, &value)?;
            let mut parent = receipt.parent();
            while let Some(dir) = parent {
                if dir == state.root || dir == target.parent().unwrap() {
                    break;
                }
                empty_candidates.push(dir.to_path_buf());
                parent = dir.parent();
            }
            Ok(())
        })();
        if let Err(error) = attempt {
            cancel.check()?;
            result.failed += 1;
            (log)(format!(
                "Pendiente {}: {error}",
                relative(&state.root, receipt)?
            ));
        }
    }
    for path in files.iter().filter(|p| {
        p.extension().is_some_and(|s| s == "json")
            && !p.to_string_lossy().ends_with(".complete.json")
    }) {
        cancel.check()?;
        let post_cache = path.parent().is_some_and(|p| {
            p.file_name().is_some_and(|n| n == "posts")
                && p.parent()
                    .is_some_and(|p| p.file_name().is_some_and(|n| n == ".state"))
        });
        let manifest = path
            .parent()
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "manifests"))
            || path == &state.root.join("manifest.json")
            || path == &state.root.join("manifest.pending.json");
        if !post_cache && !manifest {
            continue;
        }
        let attempt = (|| -> Result<()> {
            let value: Value = serde_json::from_slice(&fs::read(path)?)?;
            if post_cache {
                let url = pornerome::normalize(value["url"].as_str().unwrap_or(""))?;
                let media: Vec<String> = serde_json::from_value(value["media"].clone())?;
                for item in &media {
                    pornerome::validate_media(&url::Url::parse(item)?, &url)?;
                }
                let saved = value["saved_at"]
                    .as_f64()
                    .ok_or_else(|| anyhow::anyhow!("Caché sin fecha"))?;
                state.connection()?.execute(
                    "INSERT OR IGNORE INTO rust_posts VALUES (?,?,?)",
                    rusqlite::params![url, serde_json::to_string(&media)?, saved],
                )?;
            } else if path.to_string_lossy().ends_with(".pending.json") {
                let pending: Vec<crate::Pending> = serde_json::from_value(value.clone())?;
                for failure in pending {
                    pornerome::normalize(&failure.url)?;
                }
            } else {
                let files: Vec<crate::File> = serde_json::from_value(value.clone())?;
                for file in files {
                    pornerome::file(&file.post, &file.url)?;
                }
            }
            archive_json(state, path, &value)?;
            result.skipped += 1;
            Ok(())
        })();
        if let Err(error) = attempt {
            cancel.check()?;
            result.failed += 1;
            (log)(format!(
                "Pendiente {}: {error}",
                relative(&state.root, path)?
            ));
        }
    }
    if result.skipped > 0 {
        state.restore_pornerome_manifests(true, cancel)?;
    }
    for path in files
        .iter()
        .filter(|p| p.extension().is_some_and(|s| s == "part"))
    {
        cancel.check()?;
        let rel = relative(&state.root, path)?;
        if legacy_target(&rel).is_none() {
            continue;
        }
        let target = archive_path(state, &rel, "legacy-parts", "part")?;
        state.archive_pornerome(&rel, &json!({"partial":relative(&state.root,&target)?}))?;
        fs::rename(path, target)?;
    }
    empty_candidates.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    empty_candidates.dedup();
    for dir in empty_candidates {
        cancel.check()?;
        if crate::target(&state.root, &relative(&state.root, &dir)?).is_ok() {
            let _ = fs::remove_dir(dir);
        }
    }
    (log)(format!(
        "Organización: {} reubicados, {} registros archivados, {} pendientes",
        result.downloaded, result.skipped, result.failed
    ));
    Ok(result)
}
