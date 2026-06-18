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

/// Local branches of the repo at `repo_path`, for the onboarding range picker.
/// The front-end calls this via `invoke('list_branches', { repoPath })` once a
/// folder is chosen, then fills the base/target dropdowns from the result.
#[derive(serde::Serialize)]
struct BranchList {
    branches: Vec<String>,
    current: Option<String>,
    default: Option<String>,
}

#[tauri::command]
fn list_branches(repo_path: String) -> Result<BranchList, String> {
    let b = engine::list_branches(&repo_path).map_err(|e| e.to_string())?;
    Ok(BranchList {
        branches: b.branches,
        current: b.current,
        default: b.default,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![load_review, list_branches])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
