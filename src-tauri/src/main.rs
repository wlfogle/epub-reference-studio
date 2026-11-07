use epub_reference_core::{
    book, ollama, online,
    workspace::{Job, Workspace},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    workspace: Workspace,
    cancel: Arc<AtomicBool>,
}

#[tauri::command]
async fn choose(app: tauri::AppHandle, kind: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file();
        let file = match kind.as_str() {
            "directory" => dialog.blocking_pick_folder(),
            "reference" => dialog
                .add_filter("Reference", &["epub", "txt"])
                .blocking_pick_file(),
            _ => dialog.add_filter("EPUB", &["epub"]).blocking_pick_file(),
        };
        file.map(|p| p.into_path().map(|p| p.to_string_lossy().to_string()))
            .transpose()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn operation(
    app: tauri::AppHandle,
    action: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace = app.state::<AppState>().workspace.clone();
    let cancel = app.state::<AppState>().cancel.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow_result::ResultValue {
        let text = |key: &str| -> Result<&str, String> {
            args[key]
                .as_str()
                .ok_or_else(|| format!("Missing argument: {key}"))
        };
        let value = match action.as_str() {
            "initialize" => serde_json::json!({
                "library": workspace.library().map_err(error)?,
                "jobs": workspace.jobs().map_err(error)?,
                "workspace": workspace.path(),
                "ollama": workspace.ai_config().map_err(error)?,
            }),
            "scan" => {
                cancel.store(false, Ordering::Relaxed);
                let library = workspace
                    .scan(&PathBuf::from(text("path")?), &cancel, |n| {
                        let _ = app.emit("scan-progress", n);
                    })
                    .map_err(error)?;
                serde_json::to_value(library).map_err(|e| e.to_string())?
            }
            "analyze" => value(workspace.analyze(&PathBuf::from(text("path")?)))?,
            "models" => {
                let config = serde_json::from_value(args["config"].clone()).map_err(error)?;
                serde_json::to_value(ollama::models(&config).map_err(error)?).map_err(error)?
            }
            "save_ai" => {
                let config = serde_json::from_value(args["config"].clone()).map_err(error)?;
                serde_json::to_value(workspace.save_ai_config(config).map_err(error)?)
                    .map_err(error)?
            }
            "assist" => value(workspace.assist(text("id")?, text("mode")?))?,
            "job" => value(workspace.job(text("id")?))?,
            "search" => value(workspace.search(
                text("id")?,
                text("query")?,
                args["refresh"].as_bool().unwrap_or(false),
            ))?,
            "download" => value(workspace.download(text("id")?, text("candidate")?))?,
            "import" => {
                value(workspace.import_reference(text("id")?, &PathBuf::from(text("path")?)))?
            }
            "compare" => {
                value(workspace.compare(text("id")?, args["confirmed"].as_bool().unwrap_or(false)))?
            }
            "select" => {
                let selected =
                    serde_json::from_value(args["selected"].clone()).map_err(|e| e.to_string())?;
                value(workspace.select(text("id")?, selected))?
            }
            "export" => serde_json::to_value(
                workspace
                    .export(text("id")?, &PathBuf::from(text("path")?))
                    .map_err(error)?,
            )
            .map_err(|e| e.to_string())?,
            _ => return Err("Unknown operation".into()),
        };
        Ok(value)
    })
    .await
    .map_err(|e| e.to_string())?
}

mod anyhow_result {
    pub type ResultValue = Result<serde_json::Value, String>;
}

fn error(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}
fn value(j: Result<Job, impl std::fmt::Display>) -> Result<serde_json::Value, String> {
    serde_json::to_value(j.map_err(error)?).map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_scan(state: tauri::State<AppState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn open_source(url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(error)?;
    online::validate_url(&parsed).map_err(error)?;
    std::process::Command::new("xdg-open")
        .arg(parsed.as_str())
        .spawn()
        .map_err(error)?;
    Ok(())
}

fn main() {
    // The audit CLI intentionally shares the same parser as the desktop.
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--inspect") {
        let result = args
            .get(2)
            .ok_or_else(|| "Supply an EPUB path".to_owned())
            .and_then(|path| book::read(&PathBuf::from(path)).map_err(error));
        match result {
            Ok(b) => println!(
                "{}",
                serde_json::json!({
                    "metadata": b.metadata, "sha256": b.sha256,
                    "characters": b.text.chars().count(), "findings": b.findings,
                })
            ),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let open = args
        .windows(2)
        .find(|pair| pair[0] == "--open")
        .map(|pair| PathBuf::from(&pair[1]));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let root = std::env::var_os("EPUB_STUDIO_WORKSPACE")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            let workspace = Workspace::new(root)?;
            if let Some(path) = &open {
                workspace.analyze(path)?;
            }
            app.manage(AppState {
                workspace,
                cancel: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            choose,
            operation,
            cancel_scan,
            open_source
        ])
        .run(tauri::generate_context!())
        .expect("Could not start EPUB Reference Studio");
}
