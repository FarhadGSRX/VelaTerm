//! The embedded frontend bundle (`dist`) served by browser remote access.
//!
//! The desktop build already embeds `dist` once, uncompressed, for its own window through Tauri. Remote access
//! serves that same copy, so velaterm.exe carries a single bundle instead of Tauri's plus a second, compressed
//! rust-embed copy. Large compressed blobs inside an executable are what antivirus heuristics read as a packed
//! loader (see docs/info/vela-server被火绒报毒原因分析_20261011_0955.md). Builds without the window
//! (vela-server, the Electron sidecar) have no Tauri context and keep rust-embed.

/// One file of the bundle and the SHA-256 that its ETag derives from.
pub(crate) struct Asset {
    pub data: Vec<u8>,
    pub sha256: [u8; 32],
}

/// Frontend build output at `src-tauri/../dist`, read from disk in debug and embedded in release.
#[cfg(not(feature = "gui"))]
#[derive(rust_embed::Embed)]
#[folder = "../dist"]
struct Bundle;

#[cfg(not(feature = "gui"))]
pub(crate) fn get(path: &str) -> Option<Asset> {
    let file = Bundle::get(path)?;
    Some(Asset { sha256: file.metadata.sha256_hash(), data: file.data.into_owned() })
}

#[cfg(feature = "gui")]
pub(crate) fn get(path: &str) -> Option<Asset> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static HASHES: OnceLock<Mutex<HashMap<String, [u8; 32]>>> = OnceLock::new();

    let key = tauri::utils::assets::AssetKey::from(path);
    let Some(data) = tauri_bundle().get(&key) else {
        return disk_fallback(path);
    };
    // The bundle is static, so a file's hash never changes; compute it once per path.
    let hashes = HASHES.get_or_init(Default::default);
    let cached = hashes.lock().ok().and_then(|map| map.get(path).copied());
    let sha256 = cached.unwrap_or_else(|| {
        let digest = sha256(&data);
        if let Ok(mut map) = hashes.lock() {
            map.insert(path.to_string(), digest);
        }
        digest
    });
    Some(Asset { data: data.into_owned(), sha256 })
}

/// Tauri's embedded asset provider, taken out of a context built from the one shared macro expansion
/// (`crate::tauri_context`). The headless `--serve` mode has no running Tauri app, so the provider cannot
/// come from an `AppHandle`.
#[cfg(feature = "gui")]
fn tauri_bundle() -> &'static dyn tauri::Assets<tauri::Wry> {
    use std::sync::OnceLock;

    static BUNDLE: OnceLock<Box<dyn tauri::Assets<tauri::Wry>>> = OnceLock::new();
    BUNDLE.get_or_init(|| crate::tauri_context().set_assets(Box::new(NoAssets))).as_ref()
}

/// Placeholder left in the discarded context after its provider is taken.
#[cfg(feature = "gui")]
struct NoAssets;

#[cfg(feature = "gui")]
impl tauri::Assets<tauri::Wry> for NoAssets {
    fn get(&self, _key: &tauri::utils::assets::AssetKey) -> Option<std::borrow::Cow<'_, [u8]>> {
        None
    }

    fn iter(&self) -> Box<tauri::utils::assets::AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }

    fn csp_hashes(
        &self,
        _html_path: &tauri::utils::assets::AssetKey,
    ) -> Box<dyn Iterator<Item = tauri::utils::assets::CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

/// Development builds pointed at a dev server embed no frontend, so read `dist` from disk the way rust-embed
/// does in debug. Release builds always embed the bundle and never read the disk.
#[cfg(feature = "gui")]
fn disk_fallback(path: &str) -> Option<Asset> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let relative = std::path::Path::new(path);
    if !relative.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist").join(relative);
    let data = std::fs::read(file).ok()?;
    Some(Asset { sha256: sha256(&data), data })
}

#[cfg(feature = "gui")]
fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[test]
    fn disk_fallback_rejects_paths_outside_dist() {
        for path in ["../Cargo.toml", "/etc/passwd", "assets/../../Cargo.toml", ""] {
            assert!(disk_fallback(path).is_none(), "{path}");
        }
    }

    #[test]
    fn a_missing_file_is_absent() {
        assert!(get("assets/definitely-not-a-real-file.js").is_none());
    }
}
