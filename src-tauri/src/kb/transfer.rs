//! Stage browser imports in the selected vault, then publish complete files without replacing destinations.
use super::*;
use base64::Engine;
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Serialize, Deserialize)]
struct ImportFile {
    index: usize,
    path: String,
    size: u64,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    destination: String,
    files: Vec<ImportFile>,
    skipped: usize,
}
pub(super) fn staging(v: &Vault, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).map_err(|_| "kb_invalid")?;
    let parent = root(v)?.join(".vkb-import");
    if parent
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("kb_invalid_path".into());
    }
    let stage = parent.join(id);
    if stage
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("kb_invalid_path".into());
    }
    Ok(stage)
}
fn manifest(v: &Vault, args: &Value) -> Result<(PathBuf, Manifest)> {
    let stage = staging(v, required(args, "importId")?)?;
    let meta: Manifest = serde_json::from_slice(&files::read_bytes(
        &stage.join("manifest.json"),
        8 * 1024 * 1024,
    )?)
    .map_err(|_| "kb_invalid")?;
    path(v, &meta.destination, true)?;
    Ok((stage, meta))
}
pub fn begin(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let destination = args["destination"].as_str().unwrap_or("");
    if !path(v, destination, true)?.is_dir() {
        return Err("kb_not_found".into());
    }
    let inputs = args["files"].as_array().ok_or("kb_invalid")?;
    if inputs.is_empty() || inputs.len() > 5000 {
        return Err("kb_too_large".into());
    }
    let mut files = vec![];
    let mut names = std::collections::HashSet::new();
    let mut skipped = vec![];
    let mut total = 0u64;
    for (index, input) in inputs.iter().enumerate() {
        let raw = required(input, "path")?;
        if raw.split('/').any(|s| s.starts_with('.') && s != "..") {
            skipped.push((raw.into(), input["size"].as_u64().unwrap_or(0)));
            continue;
        }
        relative(raw, false)?;
        let size = input["size"].as_u64().ok_or("kb_invalid")?;
        total = total.checked_add(size).ok_or("kb_too_large")?;
        if size > MAX_ASSET || total > 1024 * 1024 * 1024 {
            return Err("kb_too_large".into());
        }
        let dest = if destination.is_empty() {
            raw.into()
        } else {
            format!("{destination}/{raw}")
        };
        if path(v, &dest, false)?.exists() || !names.insert(dest.to_lowercase()) {
            return Err("kb_exists".into());
        }
        files.push(ImportFile {
            index,
            path: raw.into(),
            size,
        });
    }
    if files.is_empty() {
        return Err("kb_empty_import".into());
    }
    // Reject file/directory collisions within the manifest before creating staging data.
    for file in &files {
        let mut parent = Path::new(&file.path).parent();
        while let Some(p) = parent.filter(|p| !p.as_os_str().is_empty()) {
            if files.iter().any(|f| Path::new(&f.path) == p) {
                return Err("kb_exists".into());
            }
            parent = p.parent();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let stage = staging(v, &id)?;
    std::fs::create_dir_all(&stage).map_err(err)?;
    let indices: Vec<_> = files.iter().map(|f| f.index).collect();
    let planned: Vec<(String, u64)> = files.iter().map(|f| (f.path.clone(), f.size)).collect();
    let skipped_count = skipped.len();
    let meta = Manifest {
        destination: destination.into(),
        files,
        skipped: skipped_count,
    };
    let result = (|| {
        std::fs::write(
            stage.join("manifest.json"),
            serde_json::to_vec(&meta).map_err(err)?,
        )
        .map_err(err)?;
        for file in &meta.files {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(stage.join(file.index.to_string()))
                .map_err(err)?;
        }
        let conn = app.db().conn.lock().map_err(err)?;
        super::imports::create(&conn, &v.id, &id, destination, &planned, &skipped, total)?;
        Ok(json!({"importId":id,"indices":indices,"skipped":skipped_count,"bytes":total}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}
pub fn chunk(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let (stage, meta) = manifest(v, args)?;
    let index = args["index"].as_u64().ok_or("kb_invalid")? as usize;
    let file = meta
        .files
        .iter()
        .find(|f| f.index == index)
        .ok_or("kb_invalid")?;
    let offset = args["offset"].as_u64().ok_or("kb_invalid")?;
    let encoded = args["base64"].as_str().ok_or("kb_invalid")?;
    if encoded.len() > 1_398_104 {
        return Err("kb_too_large".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "kb_invalid")?;
    if bytes.len() > 1024 * 1024 || offset + bytes.len() as u64 > file.size {
        return Err("kb_too_large".into());
    }
    let p = stage.join(index.to_string());
    if p.symlink_metadata().map_err(err)?.file_type().is_symlink() {
        return Err("kb_invalid_path".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .map_err(err)?;
    if output.metadata().map_err(err)?.len() != offset {
        return Err("kb_conflict".into());
    }
    output.write_all(&bytes).map_err(err)?;
    super::imports::touch(app, &v.id, required(args, "importId")?);
    Ok(json!({"offset":offset+bytes.len() as u64}))
}
/// Publish a fully staged batch. Any failure closes the batch as failed so the history does not
/// list a record that never reached a result.
pub fn commit(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let import_id = required(args, "importId")?;
    let result = publish(app, v, import_id, args);
    if let Err(error) = &result {
        let code = error.split(':').next().unwrap_or("kb_io");
        let _ = super::imports::finish(app, &v.id, import_id, Some(code));
    }
    result
}
fn publish(app: &AppCtx, v: &Vault, import_id: &str, args: &Value) -> Result<Value> {
    let (stage, meta) = manifest(v, args)?;
    let mut ready = vec![];
    for file in &meta.files {
        let rel = if meta.destination.is_empty() {
            file.path.clone()
        } else {
            format!("{}/{}", meta.destination, file.path)
        };
        let dest = path(v, &rel, false)?;
        if dest.exists() {
            return Err("kb_exists".into());
        }
        let source = stage.join(file.index.to_string());
        let stat = source.symlink_metadata().map_err(err)?;
        if !stat.is_file() || stat.file_type().is_symlink() || stat.len() != file.size {
            return Err("kb_incomplete_import".into());
        }
        ready.push((source, dest, rel));
    }
    let mut created = Vec::new();
    let mut directories = Vec::new();
    let result = (|| {
        for (source, dest, _) in &ready {
            let mut missing = Vec::new();
            let mut parent = dest.parent();
            while let Some(p) = parent.filter(|p| !p.exists()) {
                missing.push(p.to_path_buf());
                parent = p.parent();
            }
            for p in missing.into_iter().rev() {
                std::fs::create_dir(&p).map_err(err)?;
                directories.push(p);
            }
            // Staging lives on the same filesystem. create_new is the fallback for volumes without hard links.
            if let Err(link_error) = std::fs::hard_link(source, dest) {
                if dest.exists() {
                    return Err(if link_error.kind() == std::io::ErrorKind::AlreadyExists {
                        "kb_exists".into()
                    } else {
                        err(link_error)
                    });
                }
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(dest)
                    .map_err(err)?;
                created.push(dest.clone());
                std::io::copy(&mut std::fs::File::open(source).map_err(err)?, &mut output)
                    .map_err(err)?;
                output.sync_all().map_err(err)?;
            } else {
                created.push(dest.clone());
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        for p in created.into_iter().rev() {
            let _ = std::fs::remove_file(p);
        }
        for p in directories.into_iter().rev() {
            let _ = std::fs::remove_dir(p);
        }
        return Err(error);
    }
    std::fs::remove_dir_all(stage).map_err(err)?;
    super::imports::complete(app, &v.id, import_id, ready.len())?;
    let paths: Vec<String> = ready.iter().map(|(_, _, p)| p.clone()).collect();
    let absolute_paths: Vec<String> = ready
        .iter()
        .map(|(_, dest, _)| dest.to_string_lossy().into_owned())
        .collect();
    Ok(
        json!({"imported":ready.len(),"skipped":meta.skipped,"paths":paths,"absolutePaths":absolute_paths}),
    )
}
/// Cancel a staged batch, or fail it when the client reports an error code.
pub fn abort(app: &AppCtx, v: &Vault, args: &Value) -> Result<Value> {
    let import_id = required(args, "importId")?;
    let stage = staging(v, import_id)?;
    if stage.exists() {
        std::fs::remove_dir_all(stage).map_err(err)?;
    }
    super::imports::finish(
        app,
        &v.id,
        import_id,
        args["error"].as_str().filter(|s| !s.is_empty()),
    )?;
    Ok(json!(null))
}
pub fn asset(v: &Vault, args: &Value) -> Result<Value> {
    let p = path(v, required(args, "path")?, false)?;
    let mut file = std::fs::File::open(&p).map_err(|_| "kb_not_found")?;
    let metadata = file.metadata().map_err(err)?;
    let stamp = format!(
        "{}:{}",
        metadata.len(),
        metadata
            .modified()
            .map_err(err)?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    if args["stamp"]
        .as_str()
        .is_some_and(|expected| expected != stamp)
    {
        return Err("kb_conflict".into());
    }
    let size = metadata.len();
    if size > MAX_ASSET {
        return Err("kb_too_large".into());
    }
    let offset = args["offset"].as_u64().unwrap_or(0);
    if offset > size {
        return Err("kb_invalid".into());
    }
    file.seek(SeekFrom::Start(offset)).map_err(err)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if file.metadata().map_err(err)?.modified().map_err(err)? != metadata.modified().map_err(err)? {
        return Err("kb_conflict".into());
    }
    let extension = p
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    };
    Ok(
        json!({"base64":base64::engine::general_purpose::STANDARD.encode(&bytes),"size":size,"mime":mime,"stamp":stamp,"offset":offset+bytes.len() as u64}),
    )
}
