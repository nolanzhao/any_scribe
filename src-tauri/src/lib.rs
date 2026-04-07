mod audio;
mod commands;
mod douyin;
mod model_manager;

mod transcriber;
mod xiaohongshu;
mod xiaoyuzhou;


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(commands::CancelState(std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(false),
        )))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::check_model_status,
            commands::download_model,
            commands::check_ffmpeg,
            commands::download_ffmpeg,

            commands::transcribe_video,
            commands::process_url,
            commands::cancel_transcription,
            commands::save_file,
            commands::open_containing_folder,
            commands::open_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
