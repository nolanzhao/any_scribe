use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Emitter};

pub struct CancelState(pub Arc<AtomicBool>);

#[tauri::command]
pub fn cancel_transcription(state: tauri::State<'_, CancelState>) {
    state.0.store(true, Ordering::Relaxed);
}

use crate::audio;
use crate::douyin;
use crate::model_manager;
use crate::transcriber;
use crate::xiaohongshu;
use crate::xiaoyuzhou;

// ─── Source detection ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Source {
    Douyin,
    Xiaohongshu,
    Xiaoyuzhou,
}

fn detect_source(text: &str) -> Result<Source, String> {
    if douyin::is_douyin_url(text) {
        return Ok(Source::Douyin);
    }
    if xiaohongshu::is_xhs_url(text) {
        return Ok(Source::Xiaohongshu);
    }
    if xiaoyuzhou::is_xiaoyuzhou_url(text) {
        return Ok(Source::Xiaoyuzhou);
    }
    Err("不支持的链接类型，请检查链接是否正确".to_string())
}

fn source_dir_name(source: Source) -> &'static str {
    match source {
        Source::Douyin => "Douyin",
        Source::Xiaohongshu => "Xiaohongshu",
        Source::Xiaoyuzhou => "Xiaoyuzhou",
    }
}

fn source_label(source: Source) -> &'static str {
    match source {
        Source::Douyin => "抖音",
        Source::Xiaohongshu => "小红书",
        Source::Xiaoyuzhou => "小宇宙",
    }
}

// ─── Dependency checks ─────────────────────────────────────────────

#[tauri::command]
pub fn check_model_status() -> model_manager::ModelInfo {
    if let Some(existing) = model_manager::find_existing_model() {
        let size_mb = std::fs::metadata(&existing)
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        return model_manager::ModelInfo {
            exists: true,
            path: existing.to_string_lossy().to_string(),
            size_mb,
            name: "Whisper Large V3 Turbo".to_string(),
        };
    }
    model_manager::check_model()
}

#[tauri::command]
pub async fn download_model(app: AppHandle) -> Result<String, String> {
    model_manager::download_model(&app).await
}

#[tauri::command]
pub async fn download_ffmpeg(app: AppHandle) -> Result<String, String> {
    model_manager::download_ffmpeg(&app).await
}

#[tauri::command]
pub fn check_ffmpeg() -> bool {
    audio::is_ffmpeg_available()
}

// ─── Result structs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TranscribeResult {
    pub title: Option<String>,
    pub segments: Vec<transcriber::Segment>,
    pub srt_path: String,
    pub txt_path: String,
    pub media_path: Option<String>,
    pub duration: f64,
}

// ─── Local video transcription ─────────────────────────────────────

#[tauri::command]
pub async fn transcribe_video(
    app: AppHandle,
    state: tauri::State<'_, CancelState>,
    video_path: String,
    language: Option<String>,
) -> Result<TranscribeResult, String> {
    state.0.store(false, Ordering::Relaxed);
    let cancel_flag = state.0.clone();

    if !Path::new(&video_path).exists() {
        return Err(format!("文件不存在: {video_path}"));
    }
    if !audio::is_supported_video(&video_path) {
        return Err("不支持的视频格式".to_string());
    }

    let model_path = model_manager::find_existing_model()
        .ok_or("未找到 Whisper 模型，请先下载")?;

    let duration = audio::get_video_duration(&video_path).unwrap_or(0.0);

    let video_stem = Path::new(&video_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("transcription");
    let video_dir = Path::new(&video_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let srt_path = video_dir.join(format!("{video_stem}.srt"));
    let txt_path = video_dir.join(format!("{video_stem}.txt"));
    let srt_str = srt_path.to_string_lossy().to_string();
    let txt_str = txt_path.to_string_lossy().to_string();

    // Extract audio to temp WAV
    let temp_dir = std::env::temp_dir().join("anyscribe");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {e}"))?;
    let wav_path = temp_dir.join("audio.wav");
    let wav_str = wav_path.to_string_lossy().to_string();

    emit_progress(&app, "extracting", 0.0, "正在从视频中提取音频...");

    let vp = video_path.clone();
    let wp = wav_str.clone();
    tokio::task::spawn_blocking(move || audio::extract_audio(&vp, &wp))
        .await
        .map_err(|e| format!("提取音频失败: {e}"))??;

    if cancel_flag.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&wav_path);
        return Err("已取消".to_string());
    }

    let segments = run_transcription(
        &app,
        cancel_flag,
        &model_path.to_string_lossy(),
        &wav_str,
        &srt_str,
        &txt_str,
        language,
    )
    .await?;

    let _ = std::fs::remove_file(&wav_path);

    Ok(TranscribeResult {
        title: None,
        segments,
        srt_path: srt_str,
        txt_path: txt_str,
        media_path: None,
        duration,
    })
}

// ─── Unified URL processing ───────────────────────────────────────

#[tauri::command]
pub async fn process_url(
    app: AppHandle,
    state: tauri::State<'_, CancelState>,
    url: String,
    language: Option<String>,
) -> Result<TranscribeResult, String> {
    state.0.store(false, Ordering::Relaxed);
    let cancel_flag = state.0.clone();

    let source = detect_source(&url)?;
    let label = source_label(source);
    let output_base = audio::output_base_dir().join(source_dir_name(source));

    emit_progress(&app, "fetching", 0.0, &format!("正在解析{label}链接..."));

    // 1. Download media (platform-specific)
    let (title, media_path) = match source {
        Source::Douyin => douyin::download(&url, &output_base).await?,
        Source::Xiaohongshu => xiaohongshu::download(&url, &output_base).await?,
        Source::Xiaoyuzhou => xiaoyuzhou::download(&url, &output_base).await?,

    };

    if cancel_flag.load(Ordering::Relaxed) {
        return Err("已取消".to_string());
    }

    // 2. Convert to WAV
    emit_progress(&app, "converting", 0.05, "正在转换音频格式...");

    let media_str = media_path.to_string_lossy().to_string();
    let duration = audio::get_video_duration(&media_str).unwrap_or(0.0);

    let safe_title = audio::sanitize_dirname(&title);
    let output_dir = media_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let wav_path = output_dir.join("audio.wav");
    let srt_path = output_dir.join(format!("{safe_title}.srt"));
    let txt_path = output_dir.join(format!("{safe_title}.txt"));

    let wav_str = wav_path.to_string_lossy().to_string();
    let srt_str = srt_path.to_string_lossy().to_string();
    let txt_str = txt_path.to_string_lossy().to_string();

    tokio::task::spawn_blocking({
        let ms = media_str.clone();
        let ws = wav_str.clone();
        move || audio::extract_audio(&ms, &ws)
    })
    .await
    .map_err(|e| format!("音频转换失败: {e}"))??;

    if cancel_flag.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&wav_path);
        return Err("已取消".to_string());
    }

    // 3. Transcribe
    let model_path = model_manager::find_existing_model()
        .ok_or("未找到 Whisper 模型，请先下载")?;

    let segments = run_transcription(
        &app,
        cancel_flag,
        &model_path.to_string_lossy(),
        &wav_str,
        &srt_str,
        &txt_str,
        language,
    )
    .await?;

    let _ = std::fs::remove_file(&wav_path);

    Ok(TranscribeResult {
        title: Some(title),
        segments,
        srt_path: srt_str,
        txt_path: txt_str,
        media_path: Some(media_str),
        duration,
    })
}

// ─── Shared transcription runner ───────────────────────────────────

async fn run_transcription(
    app: &AppHandle,
    cancel_flag: Arc<AtomicBool>,
    model_path: &str,
    wav_path: &str,
    srt_path: &str,
    txt_path: &str,
    language: Option<String>,
) -> Result<Vec<transcriber::Segment>, String> {
    let model = model_path.to_string();
    let wav = wav_path.to_string();
    let srt = srt_path.to_string();
    let txt = txt_path.to_string();
    let app_clone = app.clone();
    let cancel = cancel_flag;

    tokio::task::spawn_blocking(move || {
        transcriber::transcribe_streaming(
            &app_clone,
            cancel,
            &model,
            &wav,
            &srt,
            &txt,
            language.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("转录失败: {e}"))?
}

// ─── File operations ───────────────────────────────────────────────

#[tauri::command]
pub async fn save_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, &content).map_err(|e| format!("保存文件失败: {e}"))
}

#[tauri::command]
pub fn open_containing_folder(path: String) -> Result<(), String> {
    Command::new("open")
        .args(["-R", &path])
        .spawn()
        .map_err(|e| format!("打开文件夹失败: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn open_file(path: String) -> Result<(), String> {
    Command::new("open")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("打开文件失败: {e}"))?;
    Ok(())
}

// ─── Helpers ───────────────────────────────────────────────────────

fn emit_progress(app: &AppHandle, stage: &str, progress: f64, message: &str) {
    let _ = app.emit(
        "transcribe-progress",
        transcriber::TranscribeProgress {
            stage: stage.to_string(),
            progress,
            message: message.to_string(),
            segment: None,
        },
    );
}
