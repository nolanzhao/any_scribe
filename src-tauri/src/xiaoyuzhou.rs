use futures_util::StreamExt;
use std::path::{Path, PathBuf};

use crate::audio;

/// Desktop User-Agent for Xiaoyuzhou requests.
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Check if the URL is a Xiaoyuzhou episode URL.
pub fn is_xiaoyuzhou_url(text: &str) -> bool {
    text.contains("xiaoyuzhoufm.com")
}

/// Fetch page HTML.
async fn fetch_page(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Referer", "https://www.xiaoyuzhoufm.com/")
        .send()
        .await
        .map_err(|e| format!("获取页面失败: {e}"))?;

    resp.text()
        .await
        .map_err(|e| format!("读取页面内容失败: {e}"))
}

/// Extract the audio URL (media.xyzcdn.net/*.m4a) from script tags.
fn extract_audio_url(html: &str) -> Option<String> {
    let marker = "media.xyzcdn.net";
    let prefix = "https://";

    // Find "https://media.xyzcdn.net" in the HTML
    let mut search_from = 0;
    while let Some(marker_pos) = html[search_from..].find(marker) {
        let abs_marker = search_from + marker_pos;
        // Walk backward to find "https://"
        let region_start = abs_marker.saturating_sub(10);
        let before = &html[region_start..abs_marker];
        if let Some(https_offset) = before.rfind(prefix) {
            let url_start = region_start + https_offset;
            // Find the end of the URL: .m4a followed by a quote or whitespace
            let after = &html[url_start..];
            if let Some(m4a_pos) = after.find(".m4a") {
                let end = url_start + m4a_pos + 4;
                return Some(html[url_start..end].to_string());
            }
        }
        search_from = abs_marker + marker.len();
    }

    None
}

/// Extract episode title from <h1 class="title">...</h1> or <title> tag.
fn extract_title(html: &str) -> Option<String> {
    // Try <h1 ...>title</h1> first (common pattern for Xiaoyuzhou)
    if let Some(h1_start) = html.find("<h1") {
        let after_h1 = &html[h1_start..];
        if let Some(close_bracket) = after_h1.find('>') {
            let content_start = h1_start + close_bracket + 1;
            if let Some(h1_end) = html[content_start..].find("</h1>") {
                let title = html[content_start..content_start + h1_end].trim();
                // Strip HTML tags inside the title
                let title = strip_html_tags(title);
                if !title.is_empty() {
                    return Some(title);
                }
            }
        }
    }

    // Fallback: <title>...</title>
    if let Some(start) = html.find("<title>") {
        let content_start = start + 7;
        if let Some(end) = html[content_start..].find("</title>") {
            let title = html[content_start..content_start + end].trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }

    None
}

/// Strip HTML tags from a string.
fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result.trim().to_string()
}

/// Download audio file with streaming.
async fn download_audio_bytes(audio_url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(audio_url)
        .send()
        .await
        .map_err(|e| format!("下载音频失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("下载音频失败，HTTP {}", resp.status()));
    }

    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取音频数据失败: {e}"))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 500 * 1024 * 1024 {
            return Err("音频文件过大（仅支持 500MB 以下）".to_string());
        }
    }
    Ok(bytes)
}

/// Full download pipeline: fetch page → extract audio URL + title → download.
/// Returns (title, path_to_m4a).
pub async fn download(text: &str, output_dir: &Path) -> Result<(String, PathBuf), String> {
    let url = audio::extract_url_from_text(text)
        .filter(|u| u.contains("xiaoyuzhoufm.com"))
        .ok_or("未找到有效的小宇宙链接")?;

    // Fetch page
    let html = fetch_page(&url).await?;

    // Extract audio URL
    let audio_url = extract_audio_url(&html)
        .ok_or("未找到音频链接，可能页面结构已变更")?;

    // Extract title
    let title = extract_title(&html).unwrap_or_else(|| "小宇宙播客".to_string());

    // Prepare output directory
    let safe_title = audio::sanitize_dirname(&title);
    let dir = output_dir.join(&safe_title);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建输出目录失败: {e}"))?;

    // Check for existing download
    if let Some(existing) = audio::find_existing_media(&dir) {
        eprintln!("Found existing file, reusing: {}", existing.display());
        return Ok((title, existing));
    }

    // Download audio
    let bytes = download_audio_bytes(&audio_url).await?;
    if bytes.is_empty() {
        return Err("下载的音频为空".to_string());
    }

    let file_path = dir.join(format!("{safe_title}.m4a"));
    std::fs::write(&file_path, &bytes)
        .map_err(|e| format!("保存音频失败: {e}"))?;

    Ok((title, file_path))
}
