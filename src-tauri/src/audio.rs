use std::path::Path;
use std::process::Command;

/// Helper to get the correct ffmpeg command.
fn get_ffmpeg_path() -> String {
    let local = crate::model_manager::local_ffmpeg_path();
    if local.exists() {
        local.to_string_lossy().to_string()
    } else {
        "ffmpeg".to_string()
    }
}

/// Extract audio from a video file to a WAV file (16kHz, mono, 16-bit PCM).
/// This is the format whisper.cpp expects.
pub fn extract_audio(video_path: &str, output_path: &str) -> Result<(), String> {
    let ffmpeg_cmd = get_ffmpeg_path();
    let output = Command::new(&ffmpeg_cmd)
        .args([
            "-y",
            "-i",
            video_path,
            "-vn",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            output_path,
        ])
        .output()
        .map_err(|e| {
            format!("FFmpeg not found: {e}. Command was: {ffmpeg_cmd}")
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("FFmpeg failed: {stderr}"))
    }
}

/// Load a WAV file and return audio samples as Vec<f32> (normalized to [-1.0, 1.0]).
pub fn load_wav_as_f32(wav_path: &str) -> Result<Vec<f32>, String> {
    let reader =
        hound::WavReader::open(wav_path).map_err(|e| format!("Failed to open WAV: {e}"))?;

    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(format!(
            "Expected mono audio, got {} channels",
            spec.channels
        ));
    }

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max_val = (1 << (spec.bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max_val)
                .collect()
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .filter_map(|s| s.ok())
            .collect(),
    };

    Ok(samples)
}

/// Get video duration in seconds using ffmpeg directly (so we don't need ffprobe).
pub fn get_video_duration(video_path: &str) -> Result<f64, String> {
    let output = Command::new(get_ffmpeg_path())
        .arg("-i")
        .arg(video_path)
        .output()
        .map_err(|e| format!("ffmpeg failed: {e}"))?;

    // ffmpeg -i typically exits with code 1 when no output file is provided, which is fine here.
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    // Parse "  Duration: 00:03:45.12,"
    if let Some(duration_idx) = stderr.find("Duration: ") {
        let dur_str = &stderr[duration_idx + 10..];
        if let Some(comma_idx) = dur_str.find(',') {
            let time_str = dur_str[..comma_idx].trim();
            // time_str represents HH:MM:SS.ms
            let parts: Vec<&str> = time_str.split(':').collect();
            if parts.len() == 3 {
                let h: f64 = parts[0].parse().unwrap_or(0.0);
                let m: f64 = parts[1].parse().unwrap_or(0.0);
                let s: f64 = parts[2].parse().unwrap_or(0.0);
                return Ok(h * 3600.0 + m * 60.0 + s);
            }
        }
    }

    Err("Failed to parse duration from ffmpeg".to_string())
}

/// Check if FFmpeg is available (either local or system).
pub fn is_ffmpeg_available() -> bool {
    let local = crate::model_manager::local_ffmpeg_path();
    if local.exists() {
        return true;
    }

    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Check if the given path looks like a supported video file.
pub fn is_supported_video(path: &str) -> bool {
    let extensions = [
        "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts", "mts",
    ];
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| extensions.contains(&ext.to_lowercase().as_str()))
}

/// Sanitize a title for use as a directory/file name.
pub fn sanitize_dirname(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '_',
            _ => c,
        })
        .collect();
    // Collapse multiple underscores
    let mut result = String::with_capacity(s.len());
    let mut prev_underscore = false;
    for c in s.trim().chars() {
        if c == '_' {
            if !prev_underscore {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    let result = result.trim_matches('_').to_string();
    if result.is_empty() {
        "download".to_string()
    } else if result.chars().count() > 80 {
        result.chars().take(80).collect()
    } else {
        result
    }
}

/// Find an existing media file in a directory (for re-run detection).
pub fn find_existing_media(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let extensions = ["mp4", "mp3", "m4a", "webm"];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

/// Extract a URL from pasted share text (finds first http/https URL).
pub fn extract_url_from_text(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let url_part = &text[start..];
    let end = url_part
        .find(|c: char| c.is_whitespace())
        .unwrap_or(url_part.len());
    Some(url_part[..end].to_string())
}

/// Get the base output directory for AnyScribe downloads.
pub fn output_base_dir() -> std::path::PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("AnyScribe")
}
