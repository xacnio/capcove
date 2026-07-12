//! Convenience-only machine translation for the in-app Terms/Privacy viewer.
//! Proxied through the backend — the frontend's CSP has no connect-src for external hosts.
use anyhow::{anyhow, Result};

/// Translates `text` via Google's public, keyless translate endpoint.
#[tauri::command]
pub async fn translate_text(text: String, target: String) -> Result<String, String> {
    do_translate(&text, &target).await.map_err(|e| e.to_string())
}

async fn do_translate(text: &str, target: &str) -> Result<String> {
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    let resp = reqwest::Client::new()
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[("client", "gtx"), ("sl", "en"), ("tl", target), ("dt", "t"), ("q", text)])
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let segments = resp[0]
        .as_array()
        .ok_or_else(|| anyhow!("unexpected translate response shape"))?;
    Ok(segments
        .iter()
        .filter_map(|seg| seg[0].as_str())
        .collect::<String>())
}
