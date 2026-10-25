mod analysis;
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
mod runtime;
mod security;
mod ssh_config;
mod ssh_test;
mod sshexec;
mod terminal;
mod tools;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let banco = db::init(app.handle())?;
            // Nenhuma execução sobrevive ao fechamento do aplicativo: antes de
            // mostrar o banco, o que estava `'running'` vira interrompido. A
            // conexão volta para a fila no fim deste bloco, antes de o estado
            // passar a ser dono da alça.
            {
                let conn = banco.lock()?;
                pipeline::recover_open_runs(&conn);
                // O mesmo para uma varredura de código cortada no meio: ela não leu o
                // fim de nada e não pode aparecer no histórico como se tivesse lido.
                pipeline::recover_open_analyses(&conn);
            }
            app.manage(db::AppState::new(banco));
            app.manage(terminal::TerminalState::new());
            app.manage(files_transfer::TransferState::new());
            app.manage(auth::AuthBroker::new());
            app.manage(deploy::DeployState::new());
            app.manage(github::GitHubState::new());
            app.manage(analysis::Estado::novo());
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
