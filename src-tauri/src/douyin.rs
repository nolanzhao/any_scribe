use futures_util::StreamExt;
use std::path::{Path, PathBuf};

use crate::audio;

/// Mobile Safari User-Agent for Douyin requests.
const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
    AppleWebKit/605.1.15 (KHTML, like Gecko) \
    Version/17.0 Mobile/15E148 Safari/604.1";

/// Check if the text contains a Douyin URL.
pub fn is_douyin_url(text: &str) -> bool {
    text.contains("douyin.com")
}

/// Build an HTTP client configured for Douyin.
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(MOBILE_UA)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// Fetch the Douyin page HTML (short links auto-redirect).
async fn fetch_page(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Referer", "https://www.douyin.com/")
        .send()
        .await
        .map_err(|e| format!("获取页面失败: {e}"))?;

    resp.text()
        .await
        .map_err(|e| format!("读取页面内容失败: {e}"))
}

/// Extract `window._ROUTER_DATA = {...}` JSON from HTML using brace-depth matching.
fn extract_router_data(html: &str) -> Option<serde_json::Value> {
    let marker = "window._ROUTER_DATA";
    let marker_pos = html.find(marker)?;

    let after_marker = &html[marker_pos + marker.len()..];
    let brace_start = after_marker.find('{')?;
    let json_start = marker_pos + marker.len() + brace_start;

    let bytes = html.as_bytes();
    let mut depth = 0i32;
    let mut end_pos = json_start;

    for (i, &b) in bytes[json_start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end_pos = json_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return None;
    }

    let raw = &html[json_start..end_pos];
    let raw = raw.replace("\\u002F", "/");
    serde_json::from_str(&raw).ok()
}

/// Find page data from `_ROUTER_DATA.loaderData`.
fn find_page_data(router_data: &serde_json::Value) -> Option<serde_json::Value> {
    let loader = router_data.get("loaderData")?.as_object()?;

    // Priority: key containing "(id)"
    for (key, val) in loader {
        if key.contains("(id)") && val.is_object() {
            return Some(val.clone());
        }
    }

    // Fallback: first non-empty object
    for val in loader.values() {
        if let Some(obj) = val.as_object() {
            if !obj.is_empty() {
                return Some(val.clone());
            }
        }
    }

    None
}

/// Parse aweme info from page data. Returns (title, video_url).
fn parse_aweme(page_data: &serde_json::Value) -> (String, Option<String>) {
    let item_list = match page_data
        .get("videoInfoRes")
        .and_then(|v| v.get("item_list"))
        .and_then(|v| v.as_array())
    {
        Some(list) if !list.is_empty() => list,
        _ => return ("untitled".to_string(), None),
    };

    let aweme = &item_list[0];

    let title = aweme
        .get("desc")
        .and_then(|v| v.as_str())
        .unwrap_or("untitled")
        .to_string();

    // Check if it's an image note (not a video)
    let aweme_type = aweme.get("aweme_type").and_then(|v| v.as_i64()).unwrap_or(0);
    let has_images = aweme
        .get("images")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    if matches!(aweme_type, 2 | 68) || has_images {
        return (title, None);
    }

    // Extract video play URL
    let video_url = aweme
        .get("video")
        .and_then(|v| v.get("play_addr"))
        .and_then(|v| v.get("url_list"))
        .and_then(|v| v.as_array())
        .and_then(|list| list.first())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    (title, video_url)
}

/// Download video bytes from Douyin CDN with streaming size limit (100 MB).
async fn download_video_bytes(
    client: &reqwest::Client,
    video_url: &str,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(video_url)
        .header("Referer", "https://www.douyin.com/")
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("下载视频失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("下载视频失败，HTTP {}", resp.status()));
    }

    let mut bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取视频数据失败: {e}"))?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > 100 * 1024 * 1024 {
            return Err("视频文件过大（仅支持 100MB 以下）".to_string());
        }
    }
    Ok(bytes)
}

/// Full download pipeline: parse URL → fetch page → extract video → download.
/// Returns (title, path_to_mp4).
pub async fn download(text: &str, output_dir: &Path) -> Result<(String, PathBuf), String> {
    let url = audio::extract_url_from_text(text).ok_or("未找到有效的抖音链接")?;

    if !url.contains("douyin.com") {
        return Err("不是有效的抖音链接".to_string());
    }

    let client = build_client();

    // Fetch page
    let html = fetch_page(&client, &url).await?;

    // Parse _ROUTER_DATA
    let router_data = extract_router_data(&html)
        .ok_or("未找到视频数据，可能页面结构已变更")?;

    let page_data = find_page_data(&router_data)
        .ok_or("未找到视频信息")?;

    let (title, video_url) = parse_aweme(&page_data);
    let video_url = video_url.ok_or("该内容不是视频类型，仅支持视频")?;

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

    // Download video
    let bytes = download_video_bytes(&client, &video_url).await?;
    if bytes.is_empty() {
        return Err("下载的视频为空".to_string());
    }

    let file_path = dir.join(format!("{safe_title}.mp4"));
    std::fs::write(&file_path, &bytes)
        .map_err(|e| format!("保存视频失败: {e}"))?;

    Ok((title, file_path))
}
