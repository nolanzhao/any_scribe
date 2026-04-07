use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::audio;

/// A single transcription segment with timestamps.
#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Progress event payload sent to frontend.
#[derive(Debug, Clone, Serialize)]
pub struct TranscribeProgress {
    pub stage: String,
    pub progress: f64,
    pub message: String,
    pub segment: Option<Segment>,
}

/// Format seconds to SRT timestamp string: HH:MM:SS,mmm
pub fn format_srt_timestamp(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let hrs = total_ms / 3_600_000;
    let mins = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1_000;
    let ms = total_ms % 1_000;
    format!("{hrs:02}:{mins:02}:{secs:02},{ms:03}")
}

/// Format seconds to display timestamp: HH:MM:SS.mmm
pub fn format_display_timestamp(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let hrs = total_ms / 3_600_000;
    let mins = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1_000;
    let ms = total_ms % 1_000;
    format!("{hrs:02}:{mins:02}:{secs:02}.{ms:03}")
}

/// Chunk size in samples (30 seconds at 16kHz).
const CHUNK_SAMPLES: usize = 30 * 16000;
/// Overlap in samples (1 second).
const OVERLAP_SAMPLES: usize = 16000;

/// Run whisper transcription with chunked processing for real-time output.
/// Emits segments as they are produced and writes to files incrementally.
pub fn transcribe_streaming(
    app: &AppHandle,
    cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    model_path: &str,
    wav_path: &str,
    srt_path: &str,
    txt_path: &str,
    language: Option<&str>,
) -> Result<Vec<Segment>, String> {
    // Emit: loading model
    emit_progress(app, "loading", 0.0, "正在加载语音识别模型...", None);

    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("Failed to load whisper model: {e}"))?;

    // Load audio
    emit_progress(app, "loading_audio", 0.05, "正在加载音频数据...", None);
    let audio_data = audio::load_wav_as_f32(wav_path)?;
    let total_samples = audio_data.len();
    let total_duration = total_samples as f64 / 16000.0;

    // Open output files for incremental writing
    let mut srt_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(srt_path)
        .map_err(|e| format!("Failed to create SRT file: {e}"))?;

    let mut txt_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(txt_path)
        .map_err(|e| format!("Failed to create TXT file: {e}"))?;

    // Process audio in chunks for real-time output
    let mut all_segments: Vec<Segment> = Vec::new();
    let mut chunk_start: usize = 0;
    let mut segment_index: usize = 0;

    // When auto-detecting, lock the language after the first chunk to keep it stable.
    // e.g. if chunk 1 detects Chinese, all subsequent chunks also use Chinese.
    let mut locked_language: Option<String> = language.map(|l| l.to_string());

    while chunk_start < total_samples {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("转录已取消".to_string());
        }

        let chunk_end = (chunk_start + CHUNK_SAMPLES).min(total_samples);
        let chunk = &audio_data[chunk_start..chunk_end];
        let chunk_offset_seconds = chunk_start as f64 / 16000.0;

        let chunk_progress = chunk_start as f64 / total_samples as f64;
        let overall_progress = 0.1 + 0.9 * chunk_progress;
        emit_progress(
            app,
            "transcribing",
            overall_progress,
            &format!(
                "正在转录 {:.0}/{:.0}s ...",
                chunk_offset_seconds, total_duration
            ),
            None,
        );

        // Configure whisper params
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });

        if let Some(ref lang) = locked_language {
            params.set_language(Some(lang));
        }
        params.set_translate(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_token_timestamps(true);

        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {e}"))?;

        state
            .full(params, chunk)
            .map_err(|e| format!("Transcription failed: {e}"))?;

        // After the first chunk, lock the detected language for all subsequent chunks
        if locked_language.is_none() {
            let detected_lang_id = state.full_lang_id_from_state();
            if detected_lang_id >= 0 {
                let lang_str = whisper_rs::get_lang_str(detected_lang_id)
                    .unwrap_or("en")
                    .to_string();
                emit_progress(
                    app,
                    "transcribing",
                    overall_progress,
                    &format!("检测到语言: {}，后续将保持一致", lang_str),
                    None,
                );
                locked_language = Some(lang_str);
            }
        }

        // Collect segments from this chunk
        let num_segments = state.full_n_segments();

        for i in 0..num_segments {
            let seg = state
                .get_segment(i)
                .ok_or_else(|| format!("Failed to get segment {i}"))?;

            // Timestamps are in centiseconds, add chunk offset
            let start = seg.start_timestamp() as f64 / 100.0 + chunk_offset_seconds;
            let end = seg.end_timestamp() as f64 / 100.0 + chunk_offset_seconds;
            let text = seg
                .to_str_lossy()
                .map_err(|e| format!("Failed to get segment text: {e}"))?
                .trim()
                .to_string();

            if text.is_empty() {
                continue;
            }

            segment_index += 1;
            let segment = Segment {
                start,
                end,
                text: text.clone(),
            };

            // Write to SRT file immediately
            let _ = writeln!(
                srt_file,
                "{}\n{} --> {}\n{}\n",
                segment_index,
                format_srt_timestamp(start),
                format_srt_timestamp(end),
                text
            );
            let _ = srt_file.flush();

            // Write to TXT file immediately
            let _ = writeln!(
                txt_file,
                "[{} --> {}] {}",
                format_display_timestamp(start),
                format_display_timestamp(end),
                text
            );
            let _ = txt_file.flush();

            // Emit segment to frontend in real-time
            let seg_progress =
                0.1 + 0.9 * ((chunk_start as f64 + (i as f64 / num_segments as f64) * (chunk_end - chunk_start) as f64) / total_samples as f64);
            emit_progress(
                app,
                "transcribing",
                seg_progress,
                &format!("已转录 {} 段", segment_index),
                Some(segment.clone()),
            );

            all_segments.push(segment);
        }

        // Advance to next chunk (with overlap to avoid cutting words)
        chunk_start = if chunk_end >= total_samples {
            total_samples // done
        } else {
            chunk_end - OVERLAP_SAMPLES
        };
    }

    // Emit: done
    emit_progress(
        app,
        "done",
        1.0,
        &format!("转录完成！共 {} 段", all_segments.len()),
        None,
    );

    Ok(all_segments)
}

fn emit_progress(
    app: &AppHandle,
    stage: &str,
    progress: f64,
    message: &str,
    segment: Option<Segment>,
) {
    let _ = app.emit(
        "transcribe-progress",
        TranscribeProgress {
            stage: stage.to_string(),
            progress,
            message: message.to_string(),
            segment,
        },
    );
}


