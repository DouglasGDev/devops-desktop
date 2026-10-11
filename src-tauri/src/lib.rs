mod audit;
mod auth;
mod commands;
mod db;
mod deploy;
mod files;
mod files_transfer;
mod git;
mod git_probe;
mod github;
mod infra;
mod models;
mod pipeline;
mod proc;
mod ssh_config;
mod ssh_test;
mod sshexec;
mod security;
mod terminal;
mod tools;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = db::init(app.handle())?;
            // Nenhuma execução sobrevive ao fechamento do aplicativo: antes de
            // mostrar o banco, o que estava `'running'` vira interrompido.
            pipeline::recover_open_runs(&conn);
            app.manage(db::AppState::new(conn));
            app.manage(terminal::TerminalState::new());
            app.manage(files_transfer::TransferState::new());
            app.manage(auth::AuthBroker::new());
            app.manage(deploy::DeployState::new());
            app.manage(github::GitHubState::new());
            Ok(())
        });

    commands::register_all(builder)
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o DevOps Desktop")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(term) = app.try_state::<terminal::TerminalState>() {
                    terminal::shutdown(&term);
                }
                if let Some(tr) = app.try_state::<files_transfer::TransferState>() {
                    files_transfer::shutdown(&tr);
                }
            }
        });
}
