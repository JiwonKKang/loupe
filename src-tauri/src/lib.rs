mod engine;

/// Build the review payload for `base...target` in the repo at `repo_path`.
/// The front-end calls this via `invoke('load_review', { repoPath, base, target })`.
#[tauri::command]
fn load_review(
    repo_path: String,
    base: String,
    target: String,
) -> Result<engine::ReviewData, String> {
    engine::build_review(&repo_path, &base, &target).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![load_review])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
