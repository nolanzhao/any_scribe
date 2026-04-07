use futures_util::StreamExt;
use std::path::{Path, PathBuf};

use crate::audio;

/// Mobile Safari User-Agent for Xiaohongshu requests.
const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
    AppleWebKit/605.1.15 (KHTML, like Gecko) \
    Version/17.0 Mobile/15E148 Safari/604.1";

/// Check if the text contains a Xiaohongshu URL.
pub fn is_xhs_url(text: &str) -> bool {
    text.contains("xhslink.com") || text.contains("xiaohongshu.com")
}

/// Build an HTTP client configured for Xiaohongshu.
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(MOBILE_UA)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// Fetch page HTML, following redirects (short links resolve automatically).
async fn fetch_page(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|e| format!("获取页面失败: {e}"))?;

    resp.text()
        .await
        .map_err(|e| format!("读取页面内容失败: {e}"))
}

/// Extract `window.__INITIAL_STATE__` JSON from HTML using brace-depth matching.
fn extract_initial_state(html: &str) -> Option<serde_json::Value> {
    let marker = "window.__INITIAL_STATE__";
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
    let raw = raw.replace("undefined", "null");
    serde_json::from_str(&raw).ok()
}

/// Navigate the JSON structure to find the note object.
fn find_note_data(state: &serde_json::Value) -> Option<serde_json::Value> {
    // Path 1: mobile — state.noteData.data.noteData
    if let Some(note) = state
        .get("noteData")
        .and_then(|nd| nd.get("data"))
        .and_then(|d| d.get("noteData"))
    {
        if note.is_object() {
            return Some(note.clone());
        }
    }

    // Path 2: desktop — state.note.noteDetailMap.{id}.note
    if let Some(map) = state
        .get("note")
        .and_then(|n| n.get("noteDetailMap"))
        .and_then(|m| m.as_object())
    {
        for val in map.values() {
            if let Some(note) = val.get("note") {
                if note.is_object() {
                    return Some(note.clone());
                }
            }
        }
    }

    None
}

/// Parse note info from state. Returns (title, video_url).
fn parse_note(state: &serde_json::Value) -> Result<(String, Option<String>), String> {
    let note = find_note_data(state).ok_or("未找到笔记数据")?;

    let title = note
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| note.get("desc").and_then(|v| v.as_str()))
        .unwrap_or("untitled")
        .to_string();

    // Only handle video notes
    let note_type = note.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if note_type != "video" {
        return Ok((title, None));
    }

    // Extract video URL: try stream codecs h264 > h265 > av1 > h266
    let video_url = extract_video_url(&note);
    Ok((title, video_url))
}

/// Extract video URL from note, trying h264 > h265 > av1 > h266, then fallback.
fn extract_video_url(note: &serde_json::Value) -> Option<String> {
    let stream = note.get("video")?.get("media")?.get("stream")?;

    for codec in &["h264", "h265", "av1", "h266"] {
        if let Some(codec_list) = stream.get(*codec).and_then(|v| v.as_array()) {
            for item in codec_list {
                if let Some(master_url) = item.get("masterUrl").and_then(|v| v.as_str()) {
                    if !master_url.is_empty() {
                        return Some(master_url.to_string());
                    }
                }
            }
        }
    }

    // Fallback: consumer.originVideoKey
    let origin_key = note
        .get("video")?
        .get("consumer")?
        .get("originVideoKey")?
        .as_str()?;
    if !origin_key.is_empty() {
        return Some(format!("https://sns-video-bd.xhscdn.com/{origin_key}"));
    }

    None
}

/// Download video bytes with streaming size limit (100 MB).
async fn download_video_bytes(
    client: &reqwest::Client,
    video_url: &str,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(video_url)
        .header("Referer", "https://www.xiaohongshu.com/")
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
    let url = audio::extract_url_from_text(text).ok_or("未找到有效的小红书链接")?;

    if !url.contains("xhslink.com") && !url.contains("xiaohongshu.com") {
        return Err("不是有效的小红书链接".to_string());
    }

    let client = build_client();

    // Fetch page (short links auto-redirect via client policy)
    let html = fetch_page(&client, &url).await?;

    // Parse __INITIAL_STATE__
    let state = extract_initial_state(&html)
        .ok_or("未找到页面数据，可能需要登录或页面结构已变更")?;

    let (title, video_url) = parse_note(&state)?;
    let video_url = video_url.ok_or("该笔记不是视频类型，仅支持视频笔记")?;

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
