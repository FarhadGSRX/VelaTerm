//! Font families for the device displaying the UI. Remote servers expose only bundled text fonts.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontCatalog {
    pub families: Vec<String>,
    pub system_fonts_available: bool,
}

/// Full text families shipped with the frontend; symbol-only fallback subsets are not selectable.
pub fn bundled_font_catalog() -> FontCatalog {
    FontCatalog {
        families: vec!["JetBrains Mono".into()],
        system_fonts_available: false,
    }
}

fn merge_system_families(families: Vec<String>) -> FontCatalog {
    let mut catalog = bundled_font_catalog();
    catalog.system_fonts_available = true;
    catalog
        .families
        .extend(families.into_iter().filter_map(|family| {
            let family = family.trim();
            (!family.is_empty() && !family.starts_with('.')).then(|| family.to_owned())
        }));
    catalog
        .families
        .sort_by_cached_key(|family| family.to_lowercase());
    catalog
        .families
        .dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
    catalog
}

#[cfg(feature = "gui")]
pub fn system_font_catalog() -> FontCatalog {
    // CoreText on macOS, DirectWrite on Windows, and Fontconfig on Linux. No browser measurements.
    match font_kit::source::SystemSource::new().all_families() {
        Ok(families) if !families.is_empty() => merge_system_families(families),
        _ => bundled_font_catalog(),
    }
}

#[cfg(feature = "gui")]
#[tauri::command]
async fn catalog() -> FontCatalog {
    // Always runs on the local native host, including windows connected to a remote server.
    tauri::async_runtime::spawn_blocking(system_font_catalog)
        .await
        .unwrap_or_else(|_| bundled_font_catalog())
}

#[cfg(feature = "gui")]
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("local-fonts")
        .invoke_handler(tauri::generate_handler![catalog])
        .build()
}

/// Electron reads the same native catalog without starting a GUI, server, or database.
#[cfg(feature = "gui")]
pub fn print_font_catalog() {
    println!(
        "{}",
        serde_json::to_string(&system_font_catalog()).expect("font catalog serialization")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_installed_families_without_inventing_presets() {
        let catalog = merge_system_families(vec![
            " User Font ".into(),
            "jetbrains mono".into(),
            "User Font".into(),
            ".Hidden Font".into(),
            "".into(),
            "中文字体".into(),
        ]);
        assert!(catalog.system_fonts_available);
        assert_eq!(
            catalog.families,
            ["JetBrains Mono", "User Font", "中文字体"]
        );
    }

    #[test]
    fn browser_catalog_does_not_claim_system_enumeration() {
        let catalog = bundled_font_catalog();
        assert!(!catalog.system_fonts_available);
        assert_eq!(catalog.families, ["JetBrains Mono"]);
    }

    #[cfg(feature = "native-menu-tests")]
    #[test]
    fn remote_font_command_requires_a_window_scoped_permission() {
        use tauri::Manager;
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        let mut native_context = crate::tauri_context();
        std::mem::swap(
            context.runtime_authority_mut(),
            native_context.runtime_authority_mut(),
        );
        let app = tauri::test::mock_builder()
            .plugin(plugin())
            .build(context)
            .unwrap();
        let url: url::Url = "http://127.0.0.1:32718".parse().unwrap();
        let remote = tauri::WebviewWindowBuilder::new(
            &app,
            "font-remote-test",
            tauri::WebviewUrl::External(url.clone()),
        )
        .build()
        .unwrap();
        let invoke = || {
            tauri::test::get_ipc_response(
                &remote,
                tauri::webview::InvokeRequest {
                    cmd: "plugin:local-fonts|catalog".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: url.clone(),
                    body: Default::default(),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            )
        };
        assert!(invoke().is_err());
        app.add_capability(
            tauri::ipc::CapabilityBuilder::new("font-remote-test")
                .window("font-remote-test")
                .remote("http://127.0.0.1:*".to_string())
                .permission("local-fonts:allow-catalog"),
        )
        .unwrap();
        let catalog = invoke()
            .unwrap()
            .deserialize::<serde_json::Value>()
            .unwrap();
        assert!(catalog["families"]
            .as_array()
            .unwrap()
            .iter()
            .any(|family| family == "JetBrains Mono"));
    }
}
