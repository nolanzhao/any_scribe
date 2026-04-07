use futures_util::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

/// Download progress event payload.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub speed_mbps: f64,
    pub message: String,
}

/// Information about the whisper model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub exists: bool,
    pub path: String,
    pub size_mb: f64,
    pub name: String,
}

const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";
const MODEL_FILENAME: &str = "ggml-large-v3-turbo.bin";

/// Get the models directory path.
pub fn models_dir() -> PathBuf {
    let data_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    data_dir.join("AnyScribe").join("models")
}

/// Get the default model file path.
pub fn default_model_path() -> PathBuf {
    models_dir().join(MODEL_FILENAME)
}

/// Check if the model already exists locally.
pub fn check_model() -> ModelInfo {
    let model_path = default_model_path();
    let exists = model_path.exists();
    let size_mb = if exists {
        std::fs::metadata(&model_path)
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0)
    } else {
        0.0
    };

    ModelInfo {
        exists,
        path: model_path.to_string_lossy().to_string(),
        size_mb,
        name: "Whisper Large V3 Turbo".to_string(),
    }
}

/// Also check user's local whisper cache directories for existing models.
pub fn find_existing_model() -> Option<PathBuf> {
    // 1. Check our own directory first
    let our_model = default_model_path();
    if our_model.exists() {
        return Some(our_model);
    }

    // 2. Check common whisper.cpp model locations
    if let Some(home) = dirs::home_dir() {
        let common_paths = [
            home.join(".cache/whisper/ggml-large-v3-turbo.bin"),
            home.join("whisper.cpp/models/ggml-large-v3-turbo.bin"),
            home.join(".local/share/whisper/ggml-large-v3-turbo.bin"),
        ];

        for path in &common_paths {
            if path.exists() {
                return Some(path.clone());
            }
        }

        // Also check old VideoScribe path for backward compat
        let legacy = home.join("Library/Application Support/VideoScribe/models/ggml-large-v3-turbo.bin");
        if legacy.exists() {
            return Some(legacy);
        }

        // Also check for any turbo model variant
        let common_dirs = [
            home.join(".cache/whisper"),
            home.join("whisper.cpp/models"),
            home.join(".local/share/whisper"),
        ];

        for dir in &common_dirs {
            if dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        if name_str.contains("turbo") && name_str.ends_with(".bin") {
                            return Some(entry.path());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Download the whisper model from HuggingFace with progress reporting.
pub async fn download_model(app: &AppHandle) -> Result<String, String> {
    let model_path = default_model_path();

    // Create directory
    let models_dir = models_dir();
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {e}"))?;

    // Start download
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded: 0,
            total: 0,
            speed_mbps: 0.0,
            message: "正在连接下载服务器...".to_string(),
        },
    );

    let client = reqwest::Client::new();
    let response = client
        .get(MODEL_URL)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);

    // Write to a temp file first, then rename (atomic)
    let temp_path = model_path.with_extension("bin.downloading");
    let mut file = tokio::fs::File::create(&temp_path)
        .await
        .map_err(|e| format!("Failed to create temp file: {e}"))?;

    let mut downloaded: u64 = 0;
    let start_time = std::time::Instant::now();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {e}"))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Failed to write chunk: {e}"))?;

        downloaded += chunk.len() as u64;

        // Calculate speed
        let elapsed = start_time.elapsed().as_secs_f64();
        let speed_mbps = if elapsed > 0.0 {
            (downloaded as f64 / 1_048_576.0) / elapsed
        } else {
            0.0
        };

        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                downloaded,
                total: total_size,
                speed_mbps,
                message: format!(
                    "正在下载模型... {:.1}/{:.1} MB ({:.1} MB/s)",
                    downloaded as f64 / 1_048_576.0,
                    total_size as f64 / 1_048_576.0,
                    speed_mbps
                ),
            },
        );
    }

    // Flush and rename
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Failed to flush file: {e}"))?;
    drop(file);

    std::fs::rename(&temp_path, &model_path)
        .map_err(|e| format!("Failed to rename temp file: {e}"))?;

    Ok(model_path.to_string_lossy().to_string())
}

/// Get the local FFmpeg binary path.
pub fn local_ffmpeg_path() -> PathBuf {
    let data_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    data_dir.join("AnyScribe").join("bin").join("ffmpeg")
}

/// Download the FFmpeg binary from evermeet.cx with progress reporting.
pub async fn download_ffmpeg(app: &AppHandle) -> Result<String, String> {
    let ffmpeg_path = local_ffmpeg_path();
    let bin_dir = ffmpeg_path.parent().unwrap();
    
    std::fs::create_dir_all(bin_dir)
        .map_err(|e| format!("Failed to create bin directory: {e}"))?;

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded: 0,
            total: 0,
            speed_mbps: 0.0,
            message: "准备下载 FFmpeg (环境依赖)...".to_string(),
        },
    );

    let client = reqwest::Client::new();
    let response = client
        .get("https://evermeet.cx/ffmpeg/getrelease/zip")
        .send()
        .await
        .map_err(|e| format!("Download request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let zip_path = bin_dir.join("ffmpeg.zip");
    
    let mut file = tokio::fs::File::create(&zip_path)
        .await
        .map_err(|e| format!("Failed to create zip file: {e}"))?;

    let mut downloaded: u64 = 0;
    let start_time = std::time::Instant::now();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {e}"))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Failed to write chunk: {e}"))?;

        downloaded += chunk.len() as u64;

        let elapsed = start_time.elapsed().as_secs_f64();
        let speed_mbps = if elapsed > 0.0 {
            (downloaded as f64 / 1_048_576.0) / elapsed
        } else {
            0.0
        };

        let message = if total_size > 0 {
            format!(
                "正在下载 FFmpeg... {:.1}/{:.1} MB ({:.1} MB/s)",
                downloaded as f64 / 1_048_576.0,
                total_size as f64 / 1_048_576.0,
                speed_mbps
            )
        } else {
            format!(
                "正在下载 FFmpeg... {:.1} MB ({:.1} MB/s)",
                downloaded as f64 / 1_048_576.0,
                speed_mbps
            )
        };

        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                downloaded,
                total: total_size,
                speed_mbps,
                message,
            },
        );
    }

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Failed to flush zip file: {e}"))?;
    drop(file);

    // Extract the zip
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            downloaded: total_size,
            total: total_size,
            speed_mbps: 0.0,
            message: "正在解压 FFmpeg...".to_string(),
        },
    );

    let status = std::process::Command::new("unzip")
        .arg("-o")
        .arg("-j") // flat extraction in case it has folders
        .arg(&zip_path)
        .arg("ffmpeg") // only pull out the ffmpeg binary
        .arg("-d")
        .arg(bin_dir)
        .status()
        .map_err(|e| format!("Failed to execute unzip: {e}"))?;

    if !status.success() {
        return Err("Failed to extract FFmpeg binary".to_string());
    }

    // Clean up the zip file
    let _ = std::fs::remove_file(&zip_path);

    // Set executable permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = std::fs::metadata(&ffmpeg_path).map(|m| m.permissions()) {
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&ffmpeg_path, perms);
        }
    }

    Ok(ffmpeg_path.to_string_lossy().to_string())
}


