use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{types::ToSql, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::{open_path, open_url};

use crate::{
    analysis, audit,
    auth::{self, AskPass, AuthBroker},
    db::{self, AppState},
    deploy,
    files,
    files_transfer::{self, TransferInfo, TransferState},
    git, git_probe, github, infra,
    models::{
        AnalysisFinding, AnalysisInput, AnalysisRun, AuditEntry, ConnectionTestResult,
        DefinitionRevision, DeployProfile, DeployProfileInput, DeployRun, EditorInstalado,
        ImportSshHost, Project, PipelineNode, PipelineNodeInput, RemoteEntry, Routine,
        RoutineInput, SearchProjectRow, SearchResults, SearchServerRow, Server, ServerInput,
        SshDiscoveryReport, TerminalHistoryEntry, TerminalSessionInfo, ToolVersions,
    },
    pipeline, security, ssh_config, ssh_test, sshexec,
    terminal::{self, TerminalState},
    tools,
};

type DbHandle = Arc<Mutex<Connection>>;

fn lock_db(db: &DbHandle) -> Result<MutexGuard<'_, Connection>, String> {
    db.lock().map_err(|_| "o banco de dados local está indisponível".to_string())
}

fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let conn = lock_db(&state.db)?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| format!("falha ao ler configurações: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("falha ao ler configurações: {e}"))?;
    let mut out = HashMap::new();
    for row in rows {
        let (k, v) = row.map_err(|e| format!("falha ao ler configuração: {e}"))?;
        out.insert(k, v);
    }
    Ok(out)
}

#[tauri::command]
async fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("chave de configuração vazia".into());
    }
    if key.len() > 100 {
        return Err("chave de configuração muito longa".into());
    }
    if value.len() > 10_000 {
        return Err("valor de configuração muito longo".into());
    }
    let conn = lock_db(&state.db)?;
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        rusqlite::params![key, value, db::now_rfc3339()],
    )
    .map_err(|e| format!("falha ao salvar configuração: {e}"))?;
    // Registra apenas a chave alterada — o valor pode ser sensível no futuro.
    audit::log(
        &conn,
        "info",
        "settings",
        "Configuração alterada",
        None,
        None,
        None,
        Some(&format!("chave: {key}")),
    )
}

#[tauri::command]
fn get_app_data_dir(app: AppHandle) -> Result<String, String> {
    db::pasta_de_dados(&app).map(|p| p.display().to_string())
}

#[tauri::command]
async fn get_tool_versions(state: State<'_, AppState>) -> Result<ToolVersions, String> {
    let db = state.db.clone();
    let versions = tauri::async_runtime::spawn_blocking(tools::detect)
        .await
        .map_err(|e| format!("falha interna na detecção de ferramentas: {e}"))?;
    if versions.ssh.is_none() || versions.git.is_none() {
        let conn = lock_db(&db)?;
        audit::log(
            &conn,
            "warning",
            "app",
            "Ferramenta essencial ausente",
            None,
            None,
            None,
            Some(&format!("ssh: {:?} · git: {:?}", versions.ssh, versions.git)),
        )?;
    }
    Ok(versions)
}

#[tauri::command]
async fn list_servers(state: State<'_, AppState>) -> Result<Vec<Server>, String> {
    let conn = lock_db(&state.db)?;
    db::list_servers(&conn)
}

#[tauri::command]
async fn add_server(state: State<'_, AppState>, input: ServerInput) -> Result<Server, String> {
    input.validate()?;
    let conn = lock_db(&state.db)?;
    let server = db::insert_server(&conn, &input, "manual")?;
    audit::log(
        &conn,
        "success",
        "server",
        "Servidor cadastrado",
        Some(&server.name),
        None,
        None,
        Some(&format!("{}@{}:{}", server.username, server.host, server.port)),
    )?;
    Ok(server)
}

#[tauri::command]
async fn update_server(state: State<'_, AppState>, id: i64, input: ServerInput) -> Result<Server, String> {
    input.validate()?;
    let conn = lock_db(&state.db)?;
    let old = db::get_server(&conn, id)?;
    let server = db::update_server_row(&conn, id, &input)?;
    audit::log(
        &conn,
        "info",
        "server",
        "Servidor atualizado",
        Some(&server.name),
        None,
        None,
        Some(&format!(
            "antes: {}@{}:{} · depois: {}@{}:{}",
            old.username, old.host, old.port, server.username, server.host, server.port
        )),
    )?;
    Ok(server)
}

#[tauri::command]
async fn delete_server(state: State<'_, AppState>, id: i64) -> Result<usize, String> {
    let conn = lock_db(&state.db)?;
    let server = db::get_server(&conn, id)?;
    let modelos = db::delete_server_row(&conn, id)?;
    audit::log(
        &conn,
        "info",
        "server",
        "Servidor removido",
        Some(&server.name),
        None,
        None,
        Some(&format!(
            "apenas o cadastro local; ~/.ssh não foi alterado{}",
            if modelos > 0 {
                format!(" · {modelos} modelo(s) de deploy que publicavam nele também saíram")
            } else {
                String::new()
            }
        )),
    )?;
    Ok(modelos)
}

#[tauri::command]
async fn test_connection(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
) -> Result<ConnectionTestResult, String> {
    let db = state.db.clone();
    let server = server_snapshot(&db, server_id)?;
    let name = server.name.clone();

    let result = with_askpass(
        app,
        (*passwords).clone(),
        server,
        "Teste de conexão",
        |server, pass| {
            Ok(ssh_test::test(
                server.ssh_alias.as_deref(),
                &server.host,
                server.port,
                &server.username,
                server.identity_file.as_deref(),
                pass,
            ))
        },
    )
    .await?;

    let conn = lock_db(&db)?;
    db::record_connection_status(&conn, server_id, result.ok, &result.message)?;
    audit::log(
        &conn,
        if result.ok { "success" } else { "error" },
        "connection",
        "Teste de conexão",
        Some(&name),
        None,
        result.exit_code,
        Some(&result.message),
    )?;
    Ok(result)
}

/// §12: métricas do servidor, sob pedido da pessoa usuária. A coleta roda apenas
/// comandos de leitura e o desfecho — bom ou ruim — é registrado na auditoria.
#[tauri::command]
async fn server_metrics(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
) -> Result<infra::ServerMetrics, String> {
    let db = state.db.clone();
    let server = server_snapshot(&db, server_id)?;
    let name = server.name.clone();
    let collected = with_askpass(
        app,
        (*passwords).clone(),
        server,
        "Coleta de métricas",
        infra::collect,
    )
    .await;
    let conn = lock_db(&db)?;
    match &collected {
        Ok(metrics) => audit::log(
            &conn,
            "success",
            "connection",
            "Métricas coletadas",
            Some(&name),
            None,
            None,
            Some(&format!(
                "{} · {} disco(s) · {} serviço(s) · {} alerta(s)",
                metrics.hostname.as_deref().unwrap_or("sem hostname"),
                metrics.disks.len(),
                metrics.services.len(),
                metrics.warnings.len()
            )),
        )?,
        Err(e) => audit::log(
            &conn,
            "error",
            "connection",
            "Falha ao coletar métricas",
            Some(&name),
            None,
            None,
            Some(e),
        )?,
    }
    collected
}

/// Responde ao prompt de senha exibido na interface. A senha fica só na memória
/// da operação que a solicitou.
#[tauri::command]
async fn provide_password(
    passwords: State<'_, AuthBroker>,
    request_id: i64,
    password: String,
) -> Result<(), String> {
    passwords.respond(request_id, &password)
}

/// O usuário dispensou o prompt: a operação pendente falha imediatamente.
#[tauri::command]
async fn cancel_password_prompt(
    passwords: State<'_, AuthBroker>,
    request_id: i64,
) -> Result<(), String> {
    passwords.cancel(request_id)
}

#[tauri::command]
async fn discover_ssh(state: State<'_, AppState>) -> Result<SshDiscoveryReport, String> {
    let db = state.db.clone();
    let report = tauri::async_runtime::spawn_blocking(ssh_config::discover)
        .await
        .map_err(|e| format!("falha interna na descoberta SSH: {e}"))?;
    let conn = lock_db(&db)?;
    audit::log(
        &conn,
        "info",
        "app",
        "Descoberta de ~/.ssh",
        None,
        None,
        None,
        Some(&format!(
            "{} arquivo(s) de configuração · {} host(s) · {} chave(s) privada(s)",
            report.config_files.len(),
            report.hosts.len(),
            report.private_keys.len()
        )),
    )?;
    Ok(report)
}

#[tauri::command]
async fn import_ssh_hosts(state: State<'_, AppState>, entries: Vec<ImportSshHost>) -> Result<Vec<Server>, String> {
    if entries.is_empty() {
        return Err("nenhum host selecionado para importar".into());
    }
    if entries.len() > 500 {
        return Err("seleção grande demais; importe em lotes menores".into());
    }
    let conn = lock_db(&state.db)?;
    let mut created: Vec<Server> = Vec::new();
    let mut skipped = 0usize;
    for entry in &entries {
        let alias = entry.alias.trim().to_string();
        let host = entry.host.trim().to_string();
        if alias.is_empty() || host.is_empty() || !(1..=65535).contains(&entry.port) {
            skipped += 1;
            continue;
        }
        if crate::models::validate_environment(&entry.environment).is_err() {
            skipped += 1;
            continue;
        }
        let user = if entry.user.trim().is_empty() {
            std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_default()
        } else {
            entry.user.trim().to_string()
        };
        if user.is_empty() {
            skipped += 1;
            continue;
        }
        if db::server_exists(&conn, &alias, &host, &user, entry.port)? {
            skipped += 1;
            continue;
        }
        let input = ServerInput {
            name: alias.clone(),
            host,
            port: entry.port,
            username: user,
            auth_method: if entry.identity_file.is_some() { "key".into() } else { "unknown".into() },
            identity_file: entry.identity_file.clone(),
            ssh_alias: Some(alias),
            environment: entry.environment.clone(),
            group_name: None,
            notes: None,
        };
        created.push(db::insert_server(&conn, &input, "imported")?);
    }
    audit::log(
        &conn,
        if created.is_empty() { "warning" } else { "success" },
        "server",
        "Importação de ~/.ssh/config",
        None,
        None,
        None,
        Some(&format!("{} importado(s), {} ignorado(s)", created.len(), skipped)),
    )?;
    Ok(created)
}

#[tauri::command]
async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    let conn = lock_db(&state.db)?;
    db::list_projects(&conn)
}

#[tauri::command]
async fn add_project(state: State<'_, AppState>, path: String, name: Option<String>) -> Result<Project, String> {
    if path.trim().is_empty() {
        return Err("informe o caminho da pasta do projeto".into());
    }
    let db = state.db.clone();
    let raw = path.trim().to_string();
    let (canonical, probe) = tauri::async_runtime::spawn_blocking(move || {
        let canonical = git_probe::canonical_dir(&raw)?;
        let probe = git_probe::probe(&canonical);
        Ok::<_, String>((canonical, probe))
    })
    .await
    .map_err(|e| format!("falha interna ao adicionar projeto: {e}"))
    .and_then(|r| r)?;

    let display = canonical.display().to_string();
    let name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => canonical
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("Projeto")
            .to_string(),
    };

    let conn = lock_db(&db)?;
    let project = db::insert_project(
        &conn,
        &name,
        &display,
        probe.is_git_repo,
        probe.branch.as_deref(),
        probe.remote.as_deref(),
    )?;
    let git_detail = if probe.is_git_repo {
        format!(" · branch {}", probe.branch.as_deref().unwrap_or("?"))
    } else {
        String::new()
    };
    audit::log(
        &conn,
        "success",
        "project",
        "Projeto adicionado",
        None,
        Some(&name),
        None,
        Some(&format!("pasta: {display}{git_detail}")),
    )?;
    Ok(project)
}

#[tauri::command]
async fn refresh_project(state: State<'_, AppState>, id: i64) -> Result<Project, String> {
    let db = state.db.clone();
    let project = {
        let conn = lock_db(&db)?;
        db::get_project(&conn, id)?
    };
    let path = project.local_path.clone();
    let probe = tauri::async_runtime::spawn_blocking(move || git_probe::probe(Path::new(&path)))
        .await
        .map_err(|e| format!("falha interna ao atualizar projeto: {e}"))?;
    let conn = lock_db(&db)?;
    let updated = db::update_project_git(
        &conn,
        id,
        probe.is_git_repo,
        probe.branch.as_deref(),
        probe.remote.as_deref(),
    )?;
    let detail = if probe.is_git_repo {
        format!(
            "branch: {} · remote: {}",
            probe.branch.as_deref().unwrap_or("?"),
            probe.remote.as_deref().unwrap_or("—")
        )
    } else {
        "pasta sem repositório Git".to_string()
    };
    audit::log(
        &conn,
        "info",
        "git",
        "Detecção de Git do projeto",
        None,
        Some(&project.name),
        None,
        Some(&detail),
    )?;
    Ok(updated)
}

#[tauri::command]
async fn update_project(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    default_server_id: Option<i64>,
) -> Result<Project, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("informe o nome do projeto".into());
    }
    if name.len() > 200 {
        return Err("nome do projeto muito longo".into());
    }
    let conn = lock_db(&state.db)?;
    let project = db::update_project_row(&conn, id, &name, default_server_id)?;
    audit::log(
        &conn,
        "info",
        "project",
        "Projeto atualizado",
        None,
        Some(&project.name),
        None,
        Some(&format!(
            "servidor padrão: {}",
            default_server_id.map(|s| s.to_string()).unwrap_or_else(|| "nenhum".into())
        )),
    )?;
    Ok(project)
}

#[tauri::command]
async fn delete_project(state: State<'_, AppState>, id: i64) -> Result<usize, String> {
    let conn = lock_db(&state.db)?;
    let project = db::get_project(&conn, id)?;
    let modelos = db::delete_project_row(&conn, id)?;
    audit::log(
        &conn,
        "info",
        "project",
        "Projeto removido",
        None,
        Some(&project.name),
        None,
        Some(&format!(
            "apenas o cadastro local; nenhum arquivo foi apagado{}",
            if modelos > 0 {
                format!(" · {modelos} modelo(s) de deploy do projeto também saíram do cadastro")
            } else {
                String::new()
            }
        )),
    )?;
    Ok(modelos)
}

// ---------- Modelos de deploy ----------
//
// O modelo descreve *como* publicar um projeto em um servidor. Nada aqui executa
// nada: a execução vem do motor de deploy, que só aceita um modelo já salvo e
// confirmado pelo usuário. Os valores das variáveis nunca passam por aqui — a
// interface manda apenas os nomes.

#[tauri::command]
async fn list_deploy_profiles(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<Vec<DeployProfile>, String> {
    let conn = lock_db(&state.db)?;
    db::list_deploy_profiles(&conn, project_id)
}

#[tauri::command]
async fn save_deploy_profile(
    state: State<'_, AppState>,
    input: DeployProfileInput,
) -> Result<DeployProfile, String> {
    let conn = lock_db(&state.db)?;
    let profile = db::upsert_deploy_profile(&conn, &input)?;
    let project = db::get_project(&conn, profile.project_id)?;
    audit::log(
        &conn,
        "info",
        "deploy",
        if input.id.is_some() { "Modelo de deploy atualizado" } else { "Modelo de deploy criado" },
        profile.server_name.as_deref(),
        Some(&project.name),
        None,
        Some(&profile_summary(&profile)),
    )?;
    Ok(profile)
}

#[tauri::command]
async fn delete_deploy_profile(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = lock_db(&state.db)?;
    let profile = db::get_deploy_profile(&conn, id)?;
    let project = db::get_project(&conn, profile.project_id)?;
    db::delete_deploy_profile(&conn, id)?;
    audit::log(
        &conn,
        "info",
        "deploy",
        "Modelo de deploy removido",
        profile.server_name.as_deref(),
        Some(&project.name),
        None,
        Some(&format!("modelo “{}”", profile.name)),
    )
}

#[tauri::command]
async fn set_default_deploy_profile(
    state: State<'_, AppState>,
    id: i64,
) -> Result<DeployProfile, String> {
    let conn = lock_db(&state.db)?;
    let profile = db::set_default_deploy_profile(&conn, id)?;
    let project = db::get_project(&conn, profile.project_id)?;
    audit::log(
        &conn,
        "info",
        "deploy",
        "Modelo padrão definido",
        profile.server_name.as_deref(),
        Some(&project.name),
        None,
        Some(&format!("modelo “{}”", profile.name)),
    )?;
    Ok(profile)
}

/// Resumo do modelo para a auditoria: estratégia, destino e contagem de
/// variáveis — nunca os valores.
fn profile_summary(profile: &DeployProfile) -> String {
    let destino = match profile.server_name.as_deref() {
        Some(nome) => format!("{} em {nome}", profile.remote_path),
        None => format!("{} nesta máquina", profile.remote_path),
    };
    format!(
        "estratégia {}, destino {}, {} etapa(s) de comando, {} variável(is) declarada(s) (só os nomes)",
        profile.strategy,
        destino,
        profile.prepare_commands.len()
            + profile.build_commands.len()
            + profile.restart_commands.len()
            + profile.rollback_commands.len(),
        profile.secret_keys.len(),
    )
}

// Deploy (capítulo 10) — o motor roda em thread própria e fala com o servidor
// de verdade. A interface só recebe progresso e o registro final.

#[tauri::command]
async fn deploy_preflight(
    state: State<'_, AppState>,
    profile_id: i64,
) -> Result<deploy::Preflight, String> {
    deploy::preflight(&state.db, profile_id)
}

#[tauri::command]
async fn deploy_start(
    app: AppHandle,
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    deploys: State<'_, deploy::DeployState>,
    profile_id: i64,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
) -> Result<i64, String> {
    deploy::start(
        app,
        &deploys,
        state.db.clone(),
        (*passwords).clone(),
        profile_id,
        secret_values,
        allow_secrets,
    )
}

#[tauri::command]
async fn deploy_rollback(
    app: AppHandle,
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    deploys: State<'_, deploy::DeployState>,
    run_id: i64,
    secret_values: HashMap<String, String>,
) -> Result<i64, String> {
    deploy::rollback(
        app,
        &deploys,
        state.db.clone(),
        (*passwords).clone(),
        run_id,
        secret_values,
    )
}

#[tauri::command]
async fn deploy_cancel(
    state: State<'_, AppState>,
    deploys: State<'_, deploy::DeployState>,
    run_id: i64,
) -> Result<String, String> {
    let label = deploy::cancel(&deploys, run_id)?;
    let conn = lock_db(&state.db)?;
    let _ = audit::log(
        &conn,
        "warning",
        "deploy",
        "Deploy interrompido",
        None,
        None,
        None,
        Some(&format!("execução #{run_id} · {label}")),
    );
    Ok(label)
}

#[tauri::command]
async fn deploy_active(deploys: State<'_, deploy::DeployState>) -> Result<Vec<deploy::ActiveInfo>, String> {
    Ok(deploy::active(&deploys))
}

#[tauri::command]
async fn deploy_runs(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<DeployRun>, String> {
    let conn = lock_db(&state.db)?;
    db::list_runs(&conn, project_id, limit.unwrap_or(50).clamp(1, 200))
}

#[tauri::command]
async fn deploy_run(state: State<'_, AppState>, run_id: i64) -> Result<DeployRun, String> {
    let conn = lock_db(&state.db)?;
    db::get_run(&conn, run_id)
}

// Contrato central de pipeline (plano CI/CD, etapa 2). Pedir execução, interromper e
// consultar estado já são `deploy_start` / `deploy_cancel` / `deploy_run` — o que
// estas portas acrescentam é a definição versionada, a conferência dela sem executar
// nada, o histórico das versões e a nova tentativa só quando ela é segura.

#[tauri::command]
async fn pipeline_definitions(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<Vec<pipeline::DefinitionSummary>, String> {
    pipeline::definitions(&state.db, project_id)
}

#[tauri::command]
async fn pipeline_revisions(
    state: State<'_, AppState>,
    definition_id: i64,
) -> Result<Vec<DefinitionRevision>, String> {
    pipeline::revisions(&state.db, definition_id)
}

#[tauri::command]
async fn pipeline_validate(state: State<'_, AppState>, definition_id: i64) -> Result<String, String> {
    pipeline::validate(&state.db, definition_id)
}

#[tauri::command]
async fn pipeline_retry_policy(
    state: State<'_, AppState>,
    run_id: i64,
) -> Result<pipeline::RetryPolicy, String> {
    pipeline::retry_policy(&state.db, run_id)
}

#[tauri::command]
async fn pipeline_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    deploys: State<'_, deploy::DeployState>,
    run_id: i64,
    secret_values: HashMap<String, String>,
) -> Result<i64, String> {
    pipeline::retry(
        app,
        &deploys,
        state.db.clone(),
        (*passwords).clone(),
        run_id,
        secret_values,
    )
}

// O grafo desenhado na tela é a definição: salvar nós sobe a versão, e o que a
// execução percorre é exatamente o que está aqui. Nada roda fora destas portas.

#[tauri::command]
async fn pipeline_nodes(
    state: State<'_, AppState>,
    definition_id: i64,
) -> Result<Vec<PipelineNode>, String> {
    pipeline::nodes(&state.db, definition_id)
}

#[tauri::command]
async fn pipeline_save_nodes(
    state: State<'_, AppState>,
    definition_id: i64,
    nodes: Vec<PipelineNodeInput>,
) -> Result<Vec<PipelineNode>, String> {
    pipeline::save_nodes(&state.db, definition_id, &nodes)
}

#[tauri::command]
async fn pipeline_check_nodes(
    state: State<'_, AppState>,
    definition_id: i64,
    nodes: Vec<PipelineNodeInput>,
) -> Result<String, String> {
    for no in &nodes {
        no.validate()?;
    }
    let lista = nodes
        .iter()
        .map(pipeline::NoParaChecar::from)
        .collect::<Vec<_>>();
    let tem_servidor = {
        let conn = lock_db(&state.db)?;
        db::get_deploy_profile(&conn, definition_id)?.server_id.is_some()
    };
    pipeline::checar_grafo(&lista, tem_servidor)
}

/// As quinze etapas do motor, com o nome que o próprio motor dá a cada uma no
/// modo do modelo. É a lista que o construtor oferece — nada de duplicá-la na
/// interface.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EtapaDoMotor {
    pub position: i64,
    pub name: String,
}

#[tauri::command]
async fn pipeline_engine_steps(
    state: State<'_, AppState>,
    definition_id: i64,
) -> Result<Vec<EtapaDoMotor>, String> {
    let tem_servidor = {
        let conn = lock_db(&state.db)?;
        db::get_deploy_profile(&conn, definition_id)?.server_id.is_some()
    };
    Ok((1..=15)
        .map(|p| EtapaDoMotor {
            position: p,
            name: if tem_servidor {
                deploy::step_name(p)
            } else {
                deploy::step_name_local(p)
            },
        })
        .collect())
}

/// O pedido de execução de um nó, como a tela do construtor o faz.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RodarNo {
    definition_id: i64,
    node_key: String,
    with_dependencies: bool,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
}

#[tauri::command]
async fn pipeline_run_node(
    app: AppHandle,
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    deploys: State<'_, deploy::DeployState>,
    input: RodarNo,
) -> Result<i64, String> {
    deploy::run_no(
        app,
        &deploys,
        state.db.clone(),
        (*passwords).clone(),
        deploy::PedidoDeCorrida {
            profile_id: input.definition_id,
            alvo: Some(input.node_key),
            com_dependencias: input.with_dependencies,
            secret_values: input.secret_values,
            allow_secrets: input.allow_secrets,
        },
    )
}

#[tauri::command]
async fn pipeline_approve(
    deploys: State<'_, deploy::DeployState>,
    request_id: i64,
    approved: bool,
) -> Result<(), String> {
    deploys.approvals().respond(request_id, approved)
}

#[tauri::command]
async fn pipeline_approvals_pending(
    deploys: State<'_, deploy::DeployState>,
) -> Result<usize, String> {
    Ok(deploys.approvals().pending())
}

//
// Toda operação de Git parte do `project_id` cadastrado no banco: o caminho do
// repositório nunca vem do cliente, então não há como apontar a execução para
// uma pasta fora dos projetos que o usuário registrou.

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitIdentity {
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

/// Resolve o diretório de trabalho do projeto e confirma que é um repositório.
fn project_repo(db: &DbHandle, project_id: i64) -> Result<(String, PathBuf), String> {
    let (name, local_path) = {
        let conn = lock_db(db)?;
        let project = db::get_project(&conn, project_id)?;
        (project.name, project.local_path)
    };
    let dir = git_probe::canonical_dir(&local_path)?;
    if !git::is_repo(&dir) {
        return Err(format!(
            "{} não é um repositório Git — use a seção de projetos para abrir uma pasta com repositório",
            name
        ));
    }
    Ok((name, dir))
}

/// Consulta Git em thread bloqueante. Não audita: o painel atualiza o estado a
/// cada operação e logaria cada leitura inundaria o histórico.
async fn git_read<T, F>(db: DbHandle, project_id: i64, op: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T, String> + Send + 'static,
{
    let (name, dir) = project_repo(&db, project_id)?;
    tauri::async_runtime::spawn_blocking(move || op(&dir).map_err(|e| format!("{name}: {e}")))
        .await
        .map_err(|e| format!("falha interna na leitura do Git: {e}"))?
}

/// Mutação Git em thread bloqueante, com o desfecho auditado.
async fn git_write<T, F>(
    db: DbHandle,
    project_id: i64,
    operation: &'static str,
    detail: String,
    op: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T, String> + Send + 'static,
{
    let (name, dir) = project_repo(&db, project_id)?;
    let result = tauri::async_runtime::spawn_blocking(move || op(&dir))
        .await
        .map_err(|e| format!("falha interna na operação do Git: {e}"))?;
    let conn = lock_db(&db)?;
    let message = match &result {
        Ok(_) => detail,
        Err(e) => format!("{detail} — {e}"),
    };
    audit::log(
        &conn,
        if result.is_ok() { "success" } else { "error" },
        "git",
        operation,
        None,
        Some(&name),
        None,
        // redact_url retira credenciais embutidas em URLs de remoto.
        Some(&redact_url(&message)),
    )?;
    result.map_err(|e| format!("{name}: {e}"))
}

fn read_setting(db: &DbHandle, key: &str) -> Result<Option<String>, String> {
    let conn = lock_db(db)?;
    db::setting_value(&conn, key)
}

/// Primeira linha de uma mensagem, para caber no registro de auditoria.
fn subject_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return "(sem mensagem)".to_string();
    }
    let mut out: String = first.chars().take(120).collect();
    if first.chars().count() > 120 {
        out.push('…');
    }
    out
}

/// Remove credenciais que alguns usuários embutem em URLs de remoto
/// (`https://usuario:token@host/repo`) antes de mostrar ou auditar o valor.
/// Funciona tanto para uma URL isolada quanto para uma mensagem de erro do git
/// que a contenha, trataindo cada ocorrência de `esquema://`.
fn redact_url(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find("://") {
        let after = &rest[at + 3..];
        let authority_end = after
            .find(['/', '?', '#', ' ', '\n', '\t', '"', '\''])
            .unwrap_or(after.len());
        let authority = &after[..authority_end];
        out.push_str(&rest[..at]);
        out.push_str("://");
        match authority.split_once('@') {
            Some((_, host)) => {
                out.push_str("***@");
                out.push_str(host);
            }
            None => out.push_str(authority),
        }
        rest = &after[authority_end..];
    }
    out.push_str(rest);
    out
}

/// Deep link dos editores conhecidos; `None` pede o manipulador padrão do
/// sistema, que respeita a associação escolhida pelo usuário.
fn editor_target(editor: &str, path: Option<&Path>) -> Option<String> {
    let scheme = match editor {
        "vscode" => "vscode://file",
        "cursor" => "cursor://file",
        _ => return None,
    };
    let chosen = path?;
    // canonicalize() no Windows devolve caminhos verbatim (`\\?\C:\...`), que
    // nenhum editor reconhece no deep link.
    let lossy = chosen.to_string_lossy();
    let raw = lossy
        .strip_prefix(r"\\?\")
        .unwrap_or(&lossy)
        .replace('\\', "/");
    let encoded = raw
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F");
    Some(format!("{scheme}/{encoded}"))
}

/// Por onde um editor é aberto.
#[derive(Debug, PartialEq)]
enum RotaEditor {
    /// Executável encontrado pela detecção, que recebe o caminho como argumento.
    Executavel(String),
    /// Link profundo registrado (`vscode://file/…`), quando não há executável.
    Link(String),
    /// Entrega ao sistema.
    Sistema,
}

/// A escolha do usuário só vale se o editor estiver nesta máquina. Com o
/// executável encontrado na detecção, o caminho vai como argumento separado — o
/// mesmo canal que a tela usa para afirmar que o editor existe. Sem ele, fica o
/// link profundo; sem nenhum dos dois, o padrão do sistema.
fn rota_do_editor(editor: &str, alvo: Option<&Path>, instalados: &[EditorInstalado]) -> RotaEditor {
    let Some(alvo) = alvo else {
        return RotaEditor::Sistema;
    };
    if editor == "system" {
        return RotaEditor::Sistema;
    }
    if let Some(encontrado) = instalados.iter().find(|e| e.id == editor) {
        return RotaEditor::Executavel(encontrado.caminho.clone());
    }
    match editor_target(editor, Some(alvo)) {
        Some(link) => RotaEditor::Link(link),
        None => RotaEditor::Sistema,
    }
}

fn validate_file_path(raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.len() > 1024 || raw.contains('\0') {
        return Err("caminho de arquivo inválido".into());
    }
    Ok(())
}

fn trim_opt(value: Option<String>) -> Option<String> {
    let v = value?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitActionOutcome {
    pub ok: bool,
    pub output: String,
}

fn outcome(output: String) -> GitActionOutcome {
    GitActionOutcome {
        ok: true,
        output: output.trim().to_string(),
    }
}

// ---------- proteção contra segredos (§8) ----------

/// Regras extras configuráveis somadas às listas fixas do verificador. Se a
/// leitura falhar, a operação falha junto: checagem pela metade não protege
/// nada.
fn extra_secret_patterns(db: &DbHandle) -> Result<Vec<String>, String> {
    let conn = lock_db(db)?;
    let raw = db::setting_value(&conn, deploy::SECRET_PATTERNS_SETTING)?;
    Ok(security::extra_patterns(raw.as_deref()))
}

/// Mensagem de bloqueio. Nunca repete o conteúdo do arquivo — só o caminho e o
/// motivo, para não levar um segredo para a tela ou para o log.
fn secret_block_message(operacao: &str, findings: &[security::Finding]) -> String {
    let blocked: Vec<&security::Finding> = findings
        .iter()
        .filter(|f| f.severity == "block")
        .collect();
    let nomes: Vec<String> = blocked
        .iter()
        .take(5)
        .map(|f| format!("{} — {}", f.path, f.reason))
        .collect();
    let extras = blocked.len().saturating_sub(nomes.len());
    let mais = if extras > 0 {
        format!(" · e mais {extras} caminho(s)")
    } else {
        String::new()
    };
    format!(
        "possíveis segredos neste {operacao}: {}{mais}. Nada foi enviado. Revise os arquivos (ou \
        remova-os do índice) e refaça a operação; se você tem certeza de que não há credencial \
        real neles, autorize o {operacao} marcando a opção correspondente.",
        nomes.join(" · ")
    )
}

#[tauri::command]
async fn scan_project_secrets(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<security::Finding>, String> {
    let db = state.db.clone();
    let extra = extra_secret_patterns(&db)?;
    let (name, dir) = {
        let conn = lock_db(&db)?;
        let project = db::get_project(&conn, project_id)?;
        (project.name, git_probe::canonical_dir(&project.local_path)?)
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut found = security::scan_tree(&dir, &extra, 20_000)?;
        if git::is_repo(&dir) {
            found.extend(security::tracked_sensitive(&dir, &extra)?);
        }
        Ok::<Vec<security::Finding>, String>(security::summarize(found))
    })
    .await
    .map_err(|e| format!("falha interna na varredura de {name}: {e}"))?
}

/// O que a operação publicaria, checado no conteúdo real do Git.
/// `scope` `"push"` olha o que o remoto ainda não tem; qualquer outro valor
/// (inclusive ausente) olha o índice, que é o próximo commit.
#[tauri::command]
async fn git_secret_check(
    state: State<'_, AppState>,
    project_id: i64,
    scope: Option<String>,
) -> Result<Vec<security::Finding>, String> {
    let db = state.db.clone();
    let extra = extra_secret_patterns(&db)?;
    git_read(db, project_id, move |dir| match scope.as_deref() {
        Some("push") => {
            let rels = git::push_risk_paths(dir)?;
            Ok(security::head_findings(dir, &rels, &extra))
        }
        _ => security::index_findings(dir, &extra),
    })
    .await
}

/// Ação assistida: tira o arquivo do índice sem apagar a cópia local e, quando
/// pedido, o lista no `.gitignore`. Ficar no `.gitignore` não remove nada do
/// índice nem do histórico — por isso as duas coisas acontecem aqui.
#[tauri::command]
async fn git_untrack_file(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
    gitignore: Option<bool>,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    validate_file_path(&path)?;
    let ignore = gitignore.unwrap_or(true);
    let detail = format!(
        "{path} saiu do índice (o arquivo local foi mantido){}",
        if ignore { " e foi listado no .gitignore" } else { "" }
    );
    git_write(db, project_id, "git_untrack_file", detail, move |dir| {
        git::untrack(dir, &path)?;
        if ignore {
            git::ensure_ignored(dir, &path)?;
        }
        git::status(dir)
    })
    .await
}

#[tauri::command]
async fn git_status(state: State<'_, AppState>, project_id: i64) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    git_read(db, project_id, git::status).await
}

#[tauri::command]
async fn git_identity(state: State<'_, AppState>, project_id: i64) -> Result<GitIdentity, String> {
    let db = state.db.clone();
    git_read(db, project_id, |dir| {
        let (user_name, user_email) = git::identity(dir)?;
        Ok(GitIdentity { user_name, user_email })
    })
    .await
}

/// `staged` escolhe entre índice e cabeçalho; `commit` mostra um commit
/// específico; `from`/`to` comparam duas revisões.
#[tauri::command]
async fn git_diff(
    state: State<'_, AppState>,
    project_id: i64,
    path: Option<String>,
    staged: Option<bool>,
    commit: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<Vec<git::FileDiff>, String> {
    let db = state.db.clone();
    let path = trim_opt(path);
    let staged = staged.unwrap_or(false);
    let commit = trim_opt(commit);
    let from = trim_opt(from);
    let to = trim_opt(to);
    git_read(db, project_id, move |dir| match (&from, &to) {
        (Some(a), Some(b)) => git::compare(dir, a, b),
        _ => git::diff(dir, path.as_deref(), staged, commit.as_deref()),
    })
    .await
}

#[tauri::command]
async fn git_stage(
    state: State<'_, AppState>,
    project_id: i64,
    paths: Vec<String>,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    for path in &paths {
        validate_file_path(path)?;
    }
    let detail = format!("staging de {} arquivo(s)", paths.len());
    git_write(db, project_id, "git_stage", detail, move |dir| {
        git::stage(dir, &paths)?;
        git::status(dir)
    })
    .await
}

#[tauri::command]
async fn git_unstage(
    state: State<'_, AppState>,
    project_id: i64,
    paths: Vec<String>,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    for path in &paths {
        validate_file_path(path)?;
    }
    let detail = format!("remoção de {} arquivo(s) do staging", paths.len());
    git_write(db, project_id, "git_unstage", detail, move |dir| {
        git::unstage(dir, &paths)?;
        git::status(dir)
    })
    .await
}

/// Aplica (ou desfaz) apenas os hunks escolhidos de um arquivo.
#[tauri::command]
async fn git_stage_hunks(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
    indexes: Vec<usize>,
    staged: Option<bool>,
    reverse: Option<bool>,
) -> Result<usize, String> {
    let db = state.db.clone();
    validate_file_path(&path)?;
    let staged = staged.unwrap_or(false);
    let reverse = reverse.unwrap_or(false);
    let detail = format!(
        "{} hunk(s) de {path}",
        if reverse { "desstagei" } else { "stagei" }
    );
    git_write(db, project_id, "git_stage_hunks", detail, move |dir| {
        git::stage_hunks(dir, &path, &indexes, staged, reverse)
    })
    .await
}

#[tauri::command]
async fn git_discard(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    validate_file_path(&path)?;
    let detail = format!("descarte das alterações de {path}");
    git_write(db, project_id, "git_discard", detail, move |dir| {
        git::discard_file(dir, &path, false)?;
        git::status(dir)
    })
    .await
}

#[tauri::command]
async fn git_commit(
    state: State<'_, AppState>,
    project_id: i64,
    message: String,
    amend: Option<bool>,
    allow_secrets: Option<bool>,
) -> Result<String, String> {
    let db = state.db.clone();
    let detail = subject_line(&message);
    let amend = amend.unwrap_or(false);
    let allow_secrets = allow_secrets.unwrap_or(false);
    let extra = extra_secret_patterns(&db)?;
    git_write(db, project_id, "git_commit", format!("commit: {detail}"), move |dir| {
        // §8: nada é commitado enquanto houver candidato a segredo no índice.
        if !allow_secrets {
            let found = security::index_findings(dir, &extra)?;
            if security::blocks(&found) {
                return Err(secret_block_message("commit", &found));
            }
        }
        git::commit(dir, &message, amend)
    })
    .await
}

#[tauri::command]
async fn git_graph(
    state: State<'_, AppState>,
    project_id: i64,
    limit: Option<i64>,
    all_refs: Option<bool>,
) -> Result<Vec<git::GraphRow>, String> {
    let db = state.db.clone();
    let limit = limit.unwrap_or(100).clamp(1, 2000);
    let all_refs = all_refs.unwrap_or(true);
    git_read(db, project_id, move |dir| git::graph(dir, limit, all_refs)).await
}

#[tauri::command]
async fn git_commit_detail(
    state: State<'_, AppState>,
    project_id: i64,
    id: String,
) -> Result<git::CommitDetail, String> {
    let db = state.db.clone();
    git_read(db, project_id, move |dir| git::commit_detail(dir, &id)).await
}

#[tauri::command]
async fn git_branches(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<git::BranchInfo>, String> {
    let db = state.db.clone();
    git_read(db, project_id, git::branches).await
}

#[tauri::command]
async fn git_checkout(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    create: Option<bool>,
    start_point: Option<String>,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    let create = create.unwrap_or(false);
    let start_point = trim_opt(start_point);
    let detail = format!(
        "{} para {name}",
        if create { "criação" } else { "troca" }
    );
    git_write(db, project_id, "git_checkout", detail, move |dir| {
        git::checkout(dir, &name, create, start_point.as_deref())?;
        git::status(dir)
    })
    .await
}

#[tauri::command]
async fn git_rename_branch(
    state: State<'_, AppState>,
    project_id: i64,
    from: String,
    to: String,
) -> Result<Vec<git::BranchInfo>, String> {
    let db = state.db.clone();
    let detail = format!("branch {from} renomeado para {to}");
    git_write(db, project_id, "git_rename_branch", detail, move |dir| {
        git::rename_branch(dir, &from, &to)?;
        git::branches(dir)
    })
    .await
}

#[tauri::command]
async fn git_delete_branch(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    force: Option<bool>,
) -> Result<Vec<git::BranchInfo>, String> {
    let db = state.db.clone();
    let force = force.unwrap_or(false);
    let detail = format!(
        "branch {name} excluído{}",
        if force { " (forçado)" } else { "" }
    );
    git_write(db, project_id, "git_delete_branch", detail, move |dir| {
        git::delete_branch(dir, &name, force)?;
        git::branches(dir)
    })
    .await
}

#[tauri::command]
async fn git_merge(
    state: State<'_, AppState>,
    project_id: i64,
    branch: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!("merge de {branch}");
    git_write(db, project_id, "git_merge", detail, move |dir| {
        git::merge(dir, &branch).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_rebase(
    state: State<'_, AppState>,
    project_id: i64,
    upstream: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!("rebase sobre {upstream}");
    git_write(db, project_id, "git_rebase", detail, move |dir| {
        git::rebase(dir, &upstream).map(outcome)
    })
    .await
}

/// Cancela a operação que o Git tem parada no meio. O `kind` vem do
/// `RepoStatus.operation`, então a interface cancela exatamente o que existe.
#[tauri::command]
async fn git_abort(
    state: State<'_, AppState>,
    project_id: i64,
    kind: String,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    let detail = format!("cancelamento da operação {kind}");
    git_write(db, project_id, "git_abort", detail, move |dir| {
        git::abort_operation(dir, &kind)?;
        git::status(dir)
    })
    .await
}

/// Retoma a operação depois de resolver os conflitos, usando a mensagem que o
/// próprio git preparou — nada é commitado por conta própria além do passo que
/// já estava em andamento.
#[tauri::command]
async fn git_continue_pending(
    state: State<'_, AppState>,
    project_id: i64,
    kind: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!("continuação da operação {kind}");
    git_write(db, project_id, "git_continue_pending", detail, move |dir| {
        git::continue_operation(dir, &kind).map(outcome)
    })
    .await
}

/// Abandona o passo parado sem gravar nada dele, quando o git avisa que não
/// sobrou mudança nenhuma a aplicar.
#[tauri::command]
async fn git_skip_pending(
    state: State<'_, AppState>,
    project_id: i64,
    kind: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!("pular o passo da operação {kind}");
    git_write(db, project_id, "git_skip_pending", detail, move |dir| {
        git::skip_operation(dir, &kind).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_resolve_conflict(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
    side: String,
) -> Result<git::RepoStatus, String> {
    let db = state.db.clone();
    validate_file_path(&path)?;
    if side != "ours" && side != "theirs" {
        return Err("escolha 'ours' ou 'theirs' para resolver o conflito".into());
    }
    let detail = format!("conflito de {path} resolvido com {side}");
    git_write(db, project_id, "git_resolve_conflict", detail, move |dir| {
        git::resolve_conflict(dir, &path, &side)?;
        git::status(dir)
    })
    .await
}

#[tauri::command]
async fn git_revert(
    state: State<'_, AppState>,
    project_id: i64,
    commit: String,
    mainline: Option<i64>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!(
        "reversão do commit {commit}{}",
        mainline.map(|m| format!(" (pai {m} do merge)")).unwrap_or_default()
    );
    git_write(db, project_id, "git_revert", detail, move |dir| {
        git::revert(dir, &commit, mainline).map(outcome)
    })
    .await
}

/// Reproduz um commit de outro ramo gravando um novo aqui. Nada do ramo de
/// origem muda e nada vai ao remoto por conta própria.
#[tauri::command]
async fn git_cherry_pick(
    state: State<'_, AppState>,
    project_id: i64,
    commit: String,
    mainline: Option<i64>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!(
        "reprodução do commit {commit}{}",
        mainline.map(|m| format!(" (pai {m} do merge)")).unwrap_or_default()
    );
    git_write(db, project_id, "git_cherry_pick", detail, move |dir| {
        git::cherry_pick(dir, &commit, mainline).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_stash_list(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<git::StashEntry>, String> {
    let db = state.db.clone();
    git_read(db, project_id, git::stash_list).await
}

#[tauri::command]
async fn git_stash_push(
    state: State<'_, AppState>,
    project_id: i64,
    message: Option<String>,
    include_untracked: Option<bool>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let inclui = include_untracked.unwrap_or(false);
    let recado = trim_opt(message);
    // A auditoria guarda só o assunto, nunca o texto inteiro digitado.
    let detail = format!(
        "guardar trabalho{}",
        recado.as_deref().map(|m| format!(": {}", subject_line(m))).unwrap_or_default()
    );
    let nota = if inclui { " (com não rastreados)" } else { "" };
    git_write(
        db,
        project_id,
        "git_stash_push",
        format!("{detail}{nota}"),
        move |dir| git::stash_push(dir, recado.as_deref(), inclui).map(outcome),
    )
    .await
}

/// `pop` retira o guardado da lista depois de aplicar; `apply` deixa lá. Em
/// conflito o git mantém a entrada dos dois jeitos, e a frase devolvida diz isso.
#[tauri::command]
async fn git_stash_apply(
    state: State<'_, AppState>,
    project_id: i64,
    index: i64,
    pop: Option<bool>,
) -> Result<GitActionOutcome, String> {
    if index < 0 {
        return Err("índice de guardado inválido".into());
    }
    let indice = index as usize;
    let aplicar = pop.unwrap_or(false);
    let db = state.db.clone();
    let detail = format!(
        "{} do guardado {indice}",
        if aplicar { "aplicar e retirar" } else { "aplicar" }
    );
    git_write(db, project_id, "git_stash_apply", detail, move |dir| {
        git::stash_apply(dir, indice, aplicar).map(outcome)
    })
    .await
}

/// Excluir um guardado descarta aquele trabalho — a interface confirma antes e
/// nada aqui apaga sozinho.
#[tauri::command]
async fn git_stash_drop(state: State<'_, AppState>, project_id: i64, index: i64) -> Result<GitActionOutcome, String> {
    if index < 0 {
        return Err("índice de guardado inválido".into());
    }
    let indice = index as usize;
    let db = state.db.clone();
    let detail = format!("excluir o guardado {indice}");
    git_write(db, project_id, "git_stash_drop", detail, move |dir| {
        git::stash_drop(dir, indice).map(outcome)
    })
    .await
}

/// Mover a cabeça do ramo pode desfazer trabalho local; o modo `hard` vem da
/// interface só depois de confirmação explícita e fica registrado na auditoria.
#[tauri::command]
async fn git_reset(
    state: State<'_, AppState>,
    project_id: i64,
    mode: String,
    target: String,
) -> Result<GitActionOutcome, String> {
    if !matches!(mode.as_str(), "soft" | "mixed" | "hard") {
        return Err("modo de reversão inválido".into());
    }
    let db = state.db.clone();
    let detail = format!("cabeça movida para {target} (--{mode})");
    git_write(db, project_id, "git_reset", detail, move |dir| {
        git::reset(dir, &mode, &target).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_fetch(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let remote = trim_opt(remote);
    let detail = match &remote {
        Some(r) => format!("fetch de {r}"),
        None => "fetch de todos os remotos".to_string(),
    };
    git_write(db, project_id, "git_fetch", detail, move |dir| {
        git::fetch(dir, remote.as_deref()).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_pull(
    state: State<'_, AppState>,
    project_id: i64,
    strategy: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let detail = format!("pull com {strategy}");
    git_write(db, project_id, "git_pull", detail, move |dir| {
        git::pull(dir, &strategy).map(outcome)
    })
    .await
}

#[tauri::command]
async fn git_push(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
    branch: Option<String>,
    set_upstream: Option<bool>,
    allow_secrets: Option<bool>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let remote = trim_opt(remote);
    let branch = trim_opt(branch);
    let set_upstream = set_upstream.unwrap_or(false);
    let allow_secrets = allow_secrets.unwrap_or(false);
    let extra = extra_secret_patterns(&db)?;
    let detail = format!(
        "push{}",
        remote
            .as_ref()
            .map(|r| format!(" para {r}"))
            .unwrap_or_default()
    );
    git_write(db, project_id, "git_push", detail, move |dir| {
        // §8: o push é a operação que publica o conteúdo para fora da máquina,
        // então checamos o que ele levaria, lido no HEAD.
        if !allow_secrets {
            let rels = git::push_risk_paths(dir)?;
            let found = security::head_findings(dir, &rels, &extra);
            if security::blocks(&found) {
                return Err(secret_block_message("push", &found));
            }
        }
        git::push(dir, remote.as_deref(), branch.as_deref(), set_upstream).map(outcome)
    })
    .await
}

/// As últimas linhas de uma saída de comando. Numa falha o que importa está no
/// fim, e um build não seria despejado inteiro na tela nem no histórico.
fn last_lines(text: &str, max: usize) -> String {
    let linhas: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    if linhas.is_empty() {
        return "(sem saída)".to_string();
    }
    if linhas.len() <= max {
        return linhas.join("\n");
    }
    format!("…\n{}", linhas[linhas.len() - max..].join("\n"))
}

/// O que um comando local informou, sem o conteúdo todo: stderr tem a mensagem,
/// stdout costuma ter o resto do log.
fn failure_detail(result: &sshexec::ExecResult) -> String {
    let texto = if result.stderr.trim().is_empty() {
        &result.stdout
    } else {
        &result.stderr
    };
    last_lines(texto, 20)
}

const LOCAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_PUBLISH_COMMANDS: usize = 20;

/// Publicação por Git, sem servidor: verifica os segredos do que ainda não está
/// no remoto, roda os comandos locais de teste/build na pasta do projeto, envia
/// o ramo e, se pedida, cria e envia a tag. Antes do primeiro envio qualquer
/// falha interrompe tudo; depois do ramo publicado, uma falha na tag é reportada
/// dizendo exatamente o que entrou e o que ficou de fora.
#[tauri::command]
async fn git_publish(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
    tag: Option<String>,
    commands: Vec<String>,
    allow_secrets: Option<bool>,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let remote = trim_opt(remote);
    let tag = trim_opt(tag);
    let allow_secrets = allow_secrets.unwrap_or(false);
    if commands.len() > MAX_PUBLISH_COMMANDS {
        return Err(format!(
            "são {} comandos; o limite de uma publicação é {MAX_PUBLISH_COMMANDS}",
            commands.len()
        ));
    }
    for cmd in &commands {
        if cmd.trim().is_empty() {
            continue;
        }
        if cmd.len() > 500 {
            return Err("comando local longo demais (máximo 500 caracteres)".into());
        }
    }
    let extra = extra_secret_patterns(&db)?;
    let rotulo = remote.clone().unwrap_or_else(|| "o remoto padrão".to_string());
    let mut detail = format!("publicação para {rotulo}");
    if let Some(t) = &tag {
        detail.push_str(&format!(" com a tag {t}"));
    }
    let locais = commands
        .iter()
        .filter(|c| !c.trim().is_empty())
        .count();
    if locais > 0 {
        detail.push_str(&format!(" ({locais} comando(s) locais antes do envio)"));
    }
    git_write(db, project_id, "git_publish", detail, move |dir| {
        // §8: publicar é tirar o conteúdo da máquina, então o que o push levaria
        // é lido no HEAD antes de qualquer comando.
        if !allow_secrets {
            let rels = git::push_risk_paths(dir)?;
            let found = security::head_findings(dir, &rels, &extra);
            if security::blocks(&found) {
                return Err(secret_block_message("publicação", &found));
            }
        }
        let escolhido = match remote.as_deref() {
            Some(r) => r.to_string(),
            None => git::default_remote(dir)?,
        };
        let mut partes: Vec<String> = Vec::new();

        for raw in &commands {
            let cmd = raw.trim();
            if cmd.is_empty() {
                continue;
            }
            let cancel = AtomicBool::new(false);
            let result = sshexec::run_local(cmd, dir, LOCAL_COMMAND_TIMEOUT, &cancel)
                .map_err(|e| format!("{cmd}: {e}"))?;
            if result.cancelled {
                return Err(format!("o comando {cmd} foi interrompido — nada foi publicado"));
            }
            if result.timed_out {
                return Err(format!(
                    "o comando {cmd} passou de {} minutos — nada foi publicado",
                    LOCAL_COMMAND_TIMEOUT.as_secs() / 60
                ));
            }
            if !result.ok() {
                return Err(format!(
                    "o comando {cmd} terminou com código {} — nada foi publicado\n{}",
                    result.exit_code.unwrap_or(-1),
                    failure_detail(&result)
                ));
            }
            partes.push(format!("$ {cmd}\n{}", last_lines(&result.stdout, 8)));
        }

        let situacao = git::status(dir)?;
        // Ramo sem rastreio: o primeiro envio precisa dizer onde ele nasce.
        let enviado = git::push(dir, Some(&escolhido), None, situacao.upstream.is_none())?;
        partes.push(format!("git push {escolhido}\n{}", enviado.trim()));

        if let Some(nome) = tag.as_deref() {
            let existente = git::tags(dir)?.iter().any(|t| t.name == nome);
            if !existente {
                git::create_tag(dir, nome, None, None)?;
            }
            // O ramo já está publicado: se só a tag falhar, a resposta diz isso
            // em vez de fingir que a publicação inteira não aconteceu.
            let subida = match git::push_tag(dir, &escolhido, nome) {
                Ok(saida) => saida,
                Err(e) => {
                    let recebido = partes.join("\n\n");
                    return Err(format!(
                        "{recebido}\n\no ramo foi publicado; apenas a tag {nome} não chegou a {escolhido}: {e}"
                    ));
                }
            };
            let mut linha = format!(
                "tag {nome} {}",
                if existente { "enviada (já existia local)" } else { "criada e enviada" }
            );
            let resumo = subida.trim();
            if !resumo.is_empty() {
                linha.push('\n');
                linha.push_str(resumo);
            }
            partes.push(linha);
        }

        Ok(outcome(partes.join("\n\n")))
    })
    .await
}

/// Desfaz a marca publicada: a tag sai do remoto. O ramo e o histórico não são
/// tocados, e a tag local continua — para isso existe a exclusão de tag.
#[tauri::command]
async fn git_unpublish_tag(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
    name: String,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let remote = trim_opt(remote);
    let rotulo = remote.clone().unwrap_or_else(|| "o remoto padrão".to_string());
    let detail = format!("tag {name} retirada de {rotulo}");
    git_write(db, project_id, "git_unpublish_tag", detail, move |dir| {
        let escolhido = match remote.as_deref() {
            Some(r) => r.to_string(),
            None => git::default_remote(dir)?,
        };
        let saida = git::delete_remote_tag(dir, &escolhido, &name)?;
        Ok(outcome(format!(
            "git push {escolhido} :refs/tags/{name}\n{}",
            saida.trim()
        )))
    })
    .await
}

/// Lê a liberação da máquina para um remoto: `git ls-remote` com prazo, sem
/// prompt de credencial e sem escrever nada. O veredito negativo vira
/// `ok:false` com a frase do que falta — não um erro de IPC.
#[tauri::command]
async fn git_check_remote(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
) -> Result<git::RemoteAccess, String> {
    let db = state.db.clone();
    let remote = trim_opt(remote);
    git_read(db, project_id, move |dir| {
        let escolhido = match remote.as_deref() {
            Some(r) => r.to_string(),
            None => git::default_remote(dir)?,
        };
        git::remote_access(dir, &escolhido)
    })
    .await
}

#[tauri::command]
async fn git_tags(state: State<'_, AppState>, project_id: i64) -> Result<Vec<git::TagInfo>, String> {
    let db = state.db.clone();
    git_read(db, project_id, git::tags).await
}

#[tauri::command]
async fn git_create_tag(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    target: Option<String>,
    message: Option<String>,
) -> Result<Vec<git::TagInfo>, String> {
    let db = state.db.clone();
    let target = trim_opt(target);
    let message = trim_opt(message);
    let detail = format!("tag {name} criada");
    git_write(db, project_id, "git_create_tag", detail, move |dir| {
        git::create_tag(dir, &name, target.as_deref(), message.as_deref())?;
        git::tags(dir)
    })
    .await
}

#[tauri::command]
async fn git_delete_tag(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
) -> Result<Vec<git::TagInfo>, String> {
    let db = state.db.clone();
    let detail = format!("tag {name} excluída");
    git_write(db, project_id, "git_delete_tag", detail, move |dir| {
        git::delete_tag(dir, &name)?;
        git::tags(dir)
    })
    .await
}

#[tauri::command]
async fn git_remotes(
    state: State<'_, AppState>,
    project_id: i64,
) -> Result<Vec<git::RemoteInfo>, String> {
    let db = state.db.clone();
    git_read(db, project_id, |dir| {
        let remotes = git::remotes(dir)?;
        // URLs podem carregar token embutido; mostramos sempre redigidos.
        Ok(remotes
            .into_iter()
            .map(|mut r| {
                r.fetch_url = r.fetch_url.map(|u| redact_url(&u));
                r.push_url = r.push_url.map(|u| redact_url(&u));
                r
            })
            .collect::<Vec<_>>())
    })
    .await
}

/// A árvore que o Git rastreia: os caminhos relativos que o `ls-files` devolve.
#[tauri::command]
async fn git_tree(state: State<'_, AppState>, project_id: i64) -> Result<Vec<String>, String> {
    let db = state.db.clone();
    git_read(db, project_id, git::tracked_files).await
}

#[tauri::command]
async fn git_file_content(
    state: State<'_, AppState>,
    project_id: i64,
    path: String,
) -> Result<git::FileContent, String> {
    let db = state.db.clone();
    git_read(db, project_id, move |dir| git::file_content(dir, &path)).await
}

#[tauri::command]
async fn git_add_remote(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    url: String,
) -> Result<Vec<git::RemoteInfo>, String> {
    let db = state.db.clone();
    let detail = format!("remoto {name} apontando para {}", redact_url(&url));
    git_write(db, project_id, "git_add_remote", detail, move |dir| {
        git::add_remote(dir, &name, &url)?;
        git::remotes(dir)
    })
    .await
}

#[tauri::command]
async fn git_set_remote_url(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
    url: String,
) -> Result<Vec<git::RemoteInfo>, String> {
    let db = state.db.clone();
    let detail = format!("URL do remoto {name} alterada para {}", redact_url(&url));
    git_write(db, project_id, "git_set_remote_url", detail, move |dir| {
        git::set_remote_url(dir, &name, &url)?;
        git::remotes(dir)
    })
    .await
}

#[tauri::command]
async fn git_remove_remote(
    state: State<'_, AppState>,
    project_id: i64,
    name: String,
) -> Result<Vec<git::RemoteInfo>, String> {
    let db = state.db.clone();
    let detail = format!("remoto {name} removido");
    git_write(db, project_id, "git_remove_remote", detail, move |dir| {
        git::remove_remote(dir, &name)?;
        git::remotes(dir)
    })
    .await
}

/// Clona em uma pasta já existente no disco. O registro do projeto continua
/// sendo um passo separado, feito pela seção de projetos.
#[tauri::command]
async fn git_clone(
    state: State<'_, AppState>,
    parent_path: String,
    url: String,
    target_name: Option<String>,
    branch: Option<String>,
) -> Result<String, String> {
    let dir_parent = git_probe::canonical_dir(&parent_path)?;
    let target_name = trim_opt(target_name);
    let branch = trim_opt(branch);
    let shown = redact_url(&url);
    let result = tauri::async_runtime::spawn_blocking(move || {
        git::clone(&dir_parent, &url, target_name.as_deref(), branch.as_deref())
    })
    .await
    .map_err(|e| format!("falha interna no clone: {e}"))?;
    let conn = lock_db(&state.db)?;
    audit::log(
        &conn,
        if result.is_ok() { "success" } else { "error" },
        "git",
        "git_clone",
        None,
        None,
        None,
        Some(&format!("clone de {shown}")),
    )?;
    result
}

/// Abre o repositório (ou um arquivo dele) no editor configurado. O caminho é
/// sempre resolvido a partir do projeto cadastrado e validado para não sair do
/// repositório; a API do plugin opener não impõe o escopo de capacidades.
#[tauri::command]
async fn open_in_editor(
    state: State<'_, AppState>,
    project_id: i64,
    path: Option<String>,
) -> Result<(), String> {
    let (name, dir) = project_repo(&state.db, project_id)?;
    let editor = read_setting(&state.db, "git_editor")?.unwrap_or_else(|| "system".into());
    let target: Option<PathBuf> = match trim_opt(path) {
        Some(rel) => {
            validate_file_path(&rel)?;
            let joined = dir.join(&rel);
            let canon = git_probe::simple(
                &joined
                    .canonicalize()
                    .map_err(|e| format!("o arquivo {rel} não pôde ser aberto: {e}"))?,
            );
            if !canon.starts_with(&dir) {
                return Err("o caminho solicitado está fora do repositório".into());
            }
            Some(canon)
        }
        None => Some(dir.clone()),
    };

    let instalados = tools::detectar_editores();
    let opened = match rota_do_editor(&editor, target.as_deref(), &instalados) {
        RotaEditor::Executavel(caminho) => match target.as_deref() {
            Some(alvo) => match crate::proc::command(&caminho).arg(alvo).spawn() {
                Ok(_) => Ok(()),
                // o executável encontrado não abriu (foi movido, o antivírus
                // segurou): ainda resta o atalho que o instalador registrou
                Err(e) => match editor_target(&editor, Some(alvo)) {
                    Some(link) => open_url(link.as_str(), None::<&str>)
                        .map_err(|_| format!("o editor {caminho} não abriu: {e}")),
                    None => Err(format!("o editor {caminho} não abriu: {e}")),
                },
            },
            None => Err("nenhum arquivo para abrir".to_string()),
        },
        RotaEditor::Link(link) => {
            open_url(link.as_str(), None::<&str>).map_err(|e| format!("falha ao abrir o editor: {e}"))
        }
        RotaEditor::Sistema => match target.as_deref() {
            Some(file) => open_path(file, None::<&str>).map_err(|e| format!("falha ao abrir o editor: {e}")),
            None => Err("nenhum arquivo para abrir".to_string()),
        },
    };
    let conn = lock_db(&state.db)?;
    audit::log(
        &conn,
        if opened.is_ok() { "success" } else { "error" },
        "git",
        "open_in_editor",
        None,
        Some(&name),
        None,
        Some(&format!("editor: {editor}")),
    )?;
    opened
}

#[tauri::command]
async fn query_audit_log(
    state: State<'_, AppState>,
    category: Option<String>,
    level: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<AuditEntry>, String> {
    let conn = lock_db(&state.db)?;
    let limit = limit.unwrap_or(100).clamp(1, 500);
    let offset = offset.unwrap_or(0).max(0);

    let mut clauses: Vec<&'static str> = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(cat) = category.as_deref().filter(|c| !c.is_empty()) {
        clauses.push("category = ?");
        params.push(Box::new(cat.to_string()));
    }
    if let Some(lvl) = level.as_deref().filter(|l| !l.is_empty()) {
        clauses.push("level = ?");
        params.push(Box::new(lvl.to_string()));
    }
    if let Some(q) = search.as_deref().filter(|s| !s.trim().is_empty()) {
        let pattern = like_pattern(q.trim());
        clauses.push(
            "(operation LIKE ? ESCAPE '\\' OR detail LIKE ? ESCAPE '\\' OR server_name LIKE ? ESCAPE '\\' OR project_name LIKE ? ESCAPE '\\')",
        );
        for _ in 0..4 {
            params.push(Box::new(pattern.clone()));
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, ts, level, category, operation, server_name, project_name, exit_code, detail
         FROM audit_log {where_sql} ORDER BY ts DESC, id DESC LIMIT ? OFFSET ?"
    );
    params.push(Box::new(limit));
    params.push(Box::new(offset));
    let refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("falha ao consultar histórico: {e}"))?;
    let mut rows = stmt
        .query(refs.as_slice())
        .map_err(|e| format!("falha ao consultar histórico: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("falha ao consultar histórico: {e}"))? {
        out.push(db::row_to_audit(row).map_err(|e| format!("falha ao ler registro: {e}"))?);
    }
    Ok(out)
}

#[tauri::command]
async fn search_all(state: State<'_, AppState>, query: String) -> Result<SearchResults, String> {
    let q = query.trim().to_string();
    if q.len() < 2 {
        return Ok(SearchResults {
            servers: Vec::new(),
            projects: Vec::new(),
            history: Vec::new(),
        });
    }
    let conn = lock_db(&state.db)?;
    let pattern = like_pattern(&q);

    let mut servers = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, name, host FROM servers WHERE name LIKE ?1 ESCAPE '\\' OR host LIKE ?1 ESCAPE '\\' OR username LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT 5")
            .map_err(|e| format!("falha na busca: {e}"))?;
        let rows = stmt
            .query_map([&pattern], |r| {
                Ok(SearchServerRow { id: r.get(0)?, name: r.get(1)?, host: r.get(2)? })
            })
            .map_err(|e| format!("falha na busca: {e}"))?;
        for row in rows {
            servers.push(row.map_err(|e| format!("falha na busca: {e}"))?);
        }
    }

    let mut projects = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, name, local_path FROM projects WHERE name LIKE ?1 ESCAPE '\\' OR local_path LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT 5")
            .map_err(|e| format!("falha na busca: {e}"))?;
        let rows = stmt
            .query_map([&pattern], |r| {
                Ok(SearchProjectRow { id: r.get(0)?, name: r.get(1)?, local_path: r.get(2)? })
            })
            .map_err(|e| format!("falha na busca: {e}"))?;
        for row in rows {
            projects.push(row.map_err(|e| format!("falha na busca: {e}"))?);
        }
    }

    let mut history = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, ts, level, category, operation, server_name, project_name, exit_code, detail
                      FROM audit_log WHERE operation LIKE ?1 ESCAPE '\\' OR detail LIKE ?1 ESCAPE '\\' OR server_name LIKE ?1 ESCAPE '\\' OR project_name LIKE ?1 ESCAPE '\\'
                      ORDER BY ts DESC LIMIT 5")
            .map_err(|e| format!("falha na busca: {e}"))?;
        let rows = stmt
            .query_map([&pattern], db::row_to_audit)
            .map_err(|e| format!("falha na busca: {e}"))?;
        for row in rows {
            history.push(row.map_err(|e| format!("falha na busca: {e}"))?);
        }
    }

    Ok(SearchResults { servers, projects, history })
}

#[tauri::command]
async fn open_terminal_session(
    state: State<'_, AppState>,
    term: State<'_, TerminalState>,
    app: AppHandle,
    server_id: i64,
    cols: u16,
    rows: u16,
) -> Result<TerminalSessionInfo, String> {
    if !(2..=500).contains(&cols) || !(2..=300).contains(&rows) {
        return Err(format!("dimensões de terminal inválidas: {cols}x{rows}"));
    }
    terminal::open_session(&app, &state.db, &term, server_id, cols, rows)
}

#[tauri::command]
fn write_terminal_session(term: State<'_, TerminalState>, session_id: i64, data: String) -> Result<(), String> {
    if data.len() > 100_000 {
        return Err("bloco de dados grande demais para o terminal".into());
    }
    terminal::write_session(&term, session_id, &data)
}

#[tauri::command]
fn resize_terminal_session(term: State<'_, TerminalState>, session_id: i64, cols: u16, rows: u16) -> Result<(), String> {
    terminal::resize_session(&term, session_id, cols, rows)
}

#[tauri::command]
async fn close_terminal_session(
    state: State<'_, AppState>,
    term: State<'_, TerminalState>,
    session_id: i64,
) -> Result<(), String> {
    terminal::close_session(&state.db, &term, session_id)
}

#[tauri::command]
fn list_terminal_sessions(term: State<'_, TerminalState>) -> Result<Vec<TerminalSessionInfo>, String> {
    terminal::list_sessions(&term)
}

#[tauri::command]
async fn list_routines(state: State<'_, AppState>) -> Result<Vec<Routine>, String> {
    let conn = lock_db(&state.db)?;
    db::list_routines(&conn)
}

#[tauri::command]
async fn save_routine(state: State<'_, AppState>, input: RoutineInput) -> Result<Routine, String> {
    input.validate()?;
    let conn = lock_db(&state.db)?;
    let routine = match input.id {
        None => db::insert_routine(&conn, &input)?,
        Some(id) => {
            let existing = db::get_routine(&conn, id)?;
            if existing.is_builtin {
                return Err(format!("a rotina \"{}\" é pré-instalada e não pode ser alterada", existing.name));
            }
            db::update_routine_row(&conn, id, &input)?
        }
    };
    audit::log(
        &conn,
        "success",
        "terminal",
        if input.id.is_some() { "Rotina atualizada" } else { "Rotina criada" },
        None,
        None,
        None,
        Some(&format!(
            "{} · classificação: {}",
            routine.name,
            if routine.danger == "read" { "leitura" } else { "altera o servidor" }
        )),
    )?;
    Ok(routine)
}

#[tauri::command]
async fn delete_routine(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = lock_db(&state.db)?;
    let existing = db::get_routine(&conn, id)?;
    if existing.is_builtin {
        return Err(format!("a rotina \"{}\" é pré-instalada e não pode ser removida", existing.name));
    }
    db::delete_routine_row(&conn, id)?;
    audit::log(
        &conn,
        "info",
        "terminal",
        "Rotina removida",
        None,
        None,
        None,
        Some(&existing.name),
    )
}

#[tauri::command]
async fn record_terminal_command(
    state: State<'_, AppState>,
    term: State<'_, TerminalState>,
    session_id: i64,
    command: String,
) -> Result<(), String> {
    let command = command.trim().to_string();
    if command.is_empty() || command.len() > 5000 {
        return Ok(());
    }
    let conn = lock_db(&state.db)?;
    if db::setting_value(&conn, "terminal_history_enabled")? == Some("false".into()) {
        return Ok(());
    }
    let (server_id, server_name) = {
        let sessions = terminal::list_sessions(&term)?;
        sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| (Some(s.server_id), Some(s.server_name.clone())))
            .unwrap_or((None, None))
    };
    db::record_terminal_command(&conn, server_id, server_name.as_deref(), &command)
}

#[tauri::command]
async fn list_terminal_history(state: State<'_, AppState>, limit: Option<i64>) -> Result<Vec<TerminalHistoryEntry>, String> {
    let conn = lock_db(&state.db)?;
    db::list_terminal_history(&conn, limit.unwrap_or(100).clamp(1, 500))
}

#[tauri::command]
async fn clear_terminal_history(state: State<'_, AppState>) -> Result<(), String> {
    let conn = lock_db(&state.db)?;
    let removed = db::clear_terminal_history(&conn)?;
    audit::log(
        &conn,
        "info",
        "terminal",
        "Histórico de comandos limpo",
        None,
        None,
        None,
        Some(&format!("{removed} registro(s) removido(s)")),
    )
}

// ---------- Etapa 5 — explorador de arquivos remoto (sftp) ----------

/// Busca o servidor no banco fora da lock para o caller mover para a thread.
fn server_snapshot(db: &DbHandle, server_id: i64) -> Result<Server, String> {
    let conn = lock_db(db)?;
    db::get_server(&conn, server_id)
}

/// Roda `op` em thread bloqueante. Se o servidor autentica por senha, a senha
/// é pedida na interface e existe apenas como `AskPass` nessa thread — nada é
/// gravado em disco ou no banco, e o auxiliar de prompt some no fim da operação.
async fn with_askpass<T, F>(
    app: AppHandle,
    broker: AuthBroker,
    server: Server,
    purpose: &'static str,
    op: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Server, Option<&AskPass>) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let pass = auth::prepare(&broker, &app, &server, purpose)?;
        op(&server, pass.as_ref())
    })
    .await
    .map_err(|e| format!("falha interna na operação remota: {e}"))?
}

/// Executa uma mutação sftp em thread bloqueante e audita o desfecho com o
/// `detail` já resolvido (o chamador tem o path/mode em escopo).
async fn sftp_mutation<F>(
    app: AppHandle,
    db: DbHandle,
    broker: AuthBroker,
    server_id: i64,
    op: F,
    operation: &'static str,
    detail: String,
) -> Result<(), String>
where
    F: FnOnce(&Server, Option<&AskPass>) -> Result<(), String> + Send + 'static,
{
    let server = server_snapshot(&db, server_id)?;
    let name = server.name.clone();
    let result = with_askpass(app, broker, server, "Arquivos remotos (SFTP)", op).await;
    let conn = lock_db(&db)?;
    audit::log(
        &conn,
        if result.is_ok() { "success" } else { "error" },
        "files",
        operation,
        Some(&name),
        None,
        None,
        Some(&detail),
    )?;
    result
}

#[tauri::command]
async fn sftp_list(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
) -> Result<Vec<RemoteEntry>, String> {
    let db = state.db.clone();
    let server = server_snapshot(&db, server_id)?;
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o diretório a listar".into());
    }
    let name = server.name.clone();
    let display = path.clone();
    let entries = with_askpass(app, (*passwords).clone(), server, "Arquivos remotos (SFTP)", move |server, pass| {
        files::list_dir(server, &path, pass)
    })
    .await?;
    let conn = lock_db(&db)?;
    audit::log(
        &conn,
        "info",
        "files",
        "Listagem remota",
        Some(&name),
        None,
        None,
        Some(&format!("{display} · {} entrada(s)", entries.len())),
    )?;
    Ok(entries)
}

#[tauri::command]
async fn sftp_mkdir(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o caminho do novo diretório".into());
    }
    let detail = format!("mkdir {path}");
    sftp_mutation(app, state.db.clone(), (*passwords).clone(), server_id, move |s, pass| {
        files::make_dir(s, &path, pass)
    }, "Diretório criado", detail).await
}

#[tauri::command]
async fn sftp_rename(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    from: String,
    to: String,
) -> Result<(), String> {
    let from = from.trim().to_string();
    let to = to.trim().to_string();
    if from.is_empty() || to.is_empty() {
        return Err("caminhos de origem e destino são obrigatórios".into());
    }
    let detail = format!("rename {from} → {to}");
    sftp_mutation(
        app,
        state.db.clone(),
        (*passwords).clone(),
        server_id,
        move |s, pass| files::rename(s, &from, &to, pass),
        "Renomeação remota",
        detail,
    )
    .await
}

#[tauri::command]
async fn sftp_remove(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o arquivo a remover".into());
    }
    let detail = format!("rm {path}");
    sftp_mutation(app, state.db.clone(), (*passwords).clone(), server_id, move |s, pass| {
        files::remove_file(s, &path, pass)
    }, "Arquivo removido", detail).await
}

#[tauri::command]
async fn sftp_remove_dir(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o diretório a remover".into());
    }
    let detail = format!("rmdir {path}");
    sftp_mutation(app, state.db.clone(), (*passwords).clone(), server_id, move |s, pass| {
        files::remove_dir(s, &path, pass)
    }, "Diretório removido", detail).await
}

#[tauri::command]
async fn sftp_chmod(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
    mode: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o caminho".into());
    }
    let detail = format!("chmod {mode} {path}");
    sftp_mutation(app, state.db.clone(), (*passwords).clone(), server_id, move |s, pass| {
        files::set_perms(s, &path, &mode, pass)
    }, "Permissão alterada", detail).await
}

#[tauri::command]
async fn sftp_read_text(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
) -> Result<String, String> {
    let db = state.db.clone();
    let server = server_snapshot(&db, server_id)?;
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o arquivo a abrir".into());
    }
    let name = server.name.clone();
    let display = path.clone();
    let content = with_askpass(app, (*passwords).clone(), server, "Arquivos remotos (SFTP)", move |server, pass| {
        files::read_text(server, &path, pass)
    })
    .await?;
    let conn = lock_db(&db)?;
    audit::log(
        &conn,
        "info",
        "files",
        "Arquivo aberto",
        Some(&name),
        None,
        None,
        Some(&format!("{display} · {} bytes", content.len())),
    )?;
    Ok(content)
}

#[tauri::command]
async fn sftp_write_text(
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    app: AppHandle,
    server_id: i64,
    path: String,
    content: String,
) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("informe o arquivo a salvar".into());
    }
    if content.len() > 4_000_000 {
        return Err("conteúdo grande demais para edição direta (limite: 4 MB)".into());
    }
    let detail = format!("write {path} (upload + rename) · {} bytes", content.len());
    sftp_mutation(
        app,
        state.db.clone(),
        (*passwords).clone(),
        server_id,
        move |s, pass| files::write_text(s, &path, &content, pass),
        "Arquivo salvo",
        detail,
    )
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferInput {
    server_id: i64,
    kind: String,
    remote_path: String,
    local_path: String,
    #[serde(default)]
    recursive: bool,
}

#[tauri::command]
async fn transfer_start(
    app: AppHandle,
    state: State<'_, AppState>,
    passwords: State<'_, AuthBroker>,
    transfers: State<'_, TransferState>,
    input: TransferInput,
) -> Result<i64, String> {
    if input.kind != "upload" && input.kind != "download" {
        return Err("tipo de transferência inválido".into());
    }
    let db = state.db.clone();
    let server = server_snapshot(&db, input.server_id)?;
    let kind_static: &'static str = if input.kind == "upload" { "upload" } else { "download" };
    let broker = (*passwords).clone();
    let transfers_db = db.clone();
    let app_for_prepare = app.clone();
    let server_for_prepare = server.clone();
    // A senha é solicitada fora da runtime async para não bloquear outros
    // comandos enquanto o usuário digita.
    let pass = tauri::async_runtime::spawn_blocking(move || {
        auth::prepare(&broker, &app_for_prepare, &server_for_prepare, "Transferência de arquivos")
    })
    .await
    .map_err(|e| format!("falha interna ao preparar a transferência: {e}"))??;

    let id = files_transfer::start(
        app,
        &transfers,
        files_transfer::Job {
            db: transfers_db,
            server: server.clone(),
            kind: kind_static,
            remote: input.remote_path,
            local: input.local_path,
            recursive: input.recursive,
            askpass: pass,
        },
    )?;
    let conn = lock_db(&db)?;
    audit::log(
        &conn,
        "info",
        "files",
        if kind_static == "upload" { "Upload iniciado" } else { "Download iniciado" },
        Some(&server.name),
        None,
        Some(id),
        Some(&format!(
            "transferência #{id} em andamento{}",
            if input.recursive { " (recursiva)" } else { "" }
        )),
    )?;
    Ok(id)
}

#[tauri::command]
async fn transfer_cancel(transfers: State<'_, TransferState>, transfer_id: i64) -> Result<(), String> {
    files_transfer::cancel(&transfers, transfer_id)
}

#[tauri::command]
async fn transfers_list(transfers: State<'_, TransferState>) -> Result<Vec<TransferInfo>, String> {
    Ok(files_transfer::list_active(&transfers))
}

/// Executa o trabalho fora da runtime async: as chamadas ao GitHub são
/// bloqueantes e podem levar segundos.
async fn spawn<T, F>(context: &'static str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("falha interna ({context}): {e}"))?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubClientIdInput {
    client_id: String,
}

#[tauri::command]
async fn github_status(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
) -> Result<github::GitHubStatus, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_status", move || {
        let conn = lock_db(&db)?;
        github::status(&conn, &gh)
    })
    .await
}

#[tauri::command]
async fn github_set_client_id(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: GitHubClientIdInput,
) -> Result<github::GitHubStatus, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    let client_id = input.client_id;
    spawn("github_set_client_id", move || {
        let conn = lock_db(&db)?;
        github::store_client_id(&conn, &client_id)?;
        github::status(&conn, &gh)
    })
    .await
}

#[tauri::command]
async fn github_connect_start(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
) -> Result<github::DeviceFlow, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_connect_start", move || {
        let conn = lock_db(&db)?;
        github::connect_start(&conn, &gh)
    })
    .await
}

#[tauri::command]
async fn github_connect_poll(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
) -> Result<github::ConnectResult, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_connect_poll", move || {
        let conn = lock_db(&db)?;
        github::connect_poll(&conn, &gh)
    })
    .await
}

#[tauri::command]
async fn github_disconnect(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
) -> Result<github::GitHubStatus, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_disconnect", move || {
        let conn = lock_db(&db)?;
        github::disconnect(&conn, &gh)?;
        github::status(&conn, &gh)
    })
    .await
}

#[tauri::command]
async fn github_repos(
    gh: State<'_, github::GitHubState>,
) -> Result<Vec<github::GitHubRepo>, String> {
    let gh = (*gh).clone();
    spawn("github_repos", move || github::repos(&gh)).await
}

#[tauri::command]
async fn github_pulls(
    gh: State<'_, github::GitHubState>,
    repo: String,
) -> Result<Vec<github::GitHubPull>, String> {
    let gh = (*gh).clone();
    spawn("github_pulls", move || github::pulls(&gh, &repo)).await
}

#[tauri::command]
async fn github_create_repo(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::CreateRepoInput,
) -> Result<github::GitHubRepo, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_create_repo", move || {
        let conn = lock_db(&db)?;
        github::create_repo(&conn, &gh, &input.name, input.private, input.description.as_deref())
    })
    .await
}

// ---------- workspace do GitHub: bandeja, PRs, issues e verificações ----------

#[tauri::command]
async fn github_inbox(gh: State<'_, github::GitHubState>) -> Result<github::GitHubInbox, String> {
    let gh = (*gh).clone();
    spawn("github_inbox", move || github::inbox(&gh)).await
}

#[tauri::command]
async fn github_pull_detail(
    gh: State<'_, github::GitHubState>,
    input: github::PullNumberInput,
) -> Result<github::PullDetail, String> {
    let gh = (*gh).clone();
    spawn("github_pull_detail", move || {
        github::pull_detail(&gh, &input.repo, input.number)
    })
    .await
}

#[tauri::command]
async fn github_pull_files(
    gh: State<'_, github::GitHubState>,
    input: github::PullNumberInput,
) -> Result<Vec<git::FileDiff>, String> {
    let gh = (*gh).clone();
    spawn("github_pull_files", move || {
        github::pull_files(&gh, &input.repo, input.number)
    })
    .await
}

#[tauri::command]
async fn github_issues(
    gh: State<'_, github::GitHubState>,
    repo: String,
) -> Result<Vec<github::GitHubIssue>, String> {
    let gh = (*gh).clone();
    spawn("github_issues", move || github::issues(&gh, &repo)).await
}

#[tauri::command]
async fn github_runs(
    gh: State<'_, github::GitHubState>,
    repo: String,
) -> Result<Vec<github::GitHubRun>, String> {
    let gh = (*gh).clone();
    spawn("github_runs", move || github::runs(&gh, &repo)).await
}

#[tauri::command]
async fn github_create_issue(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::NewIssueInput,
) -> Result<github::GitHubIssue, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_create_issue", move || {
        let conn = lock_db(&db)?;
        github::create_issue(
            &conn,
            &gh,
            &input.repo,
            &input.title,
            input.body.as_deref(),
        )
    })
    .await
}

#[tauri::command]
async fn github_comment(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::CommentInput,
) -> Result<(), String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_comment", move || {
        let conn = lock_db(&db)?;
        github::comment(&conn, &gh, &input.repo, input.number, &input.body)
    })
    .await
}

#[tauri::command]
async fn github_close_issue(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::CloseIssueInput,
) -> Result<(), String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_close_issue", move || {
        let conn = lock_db(&db)?;
        github::close_issue(&conn, &gh, &input.repo, input.number)
    })
    .await
}

#[tauri::command]
async fn github_review(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::ReviewInput,
) -> Result<(), String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_review", move || {
        let conn = lock_db(&db)?;
        github::review_pull(
            &conn,
            &gh,
            &input.repo,
            input.number,
            &input.event,
            input.body.as_deref(),
        )
    })
    .await
}

#[tauri::command]
async fn github_merge(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::MergeInput,
) -> Result<String, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_merge", move || {
        let conn = lock_db(&db)?;
        github::merge_pull(
            &conn,
            &gh,
            &input.repo,
            input.number,
            &input.method,
            &input.sha,
        )
    })
    .await
}

#[tauri::command]
async fn github_rerun(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::RerunInput,
) -> Result<(), String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_rerun", move || {
        let conn = lock_db(&db)?;
        github::rerun(&conn, &gh, &input.repo, input.run_id)
    })
    .await
}

#[tauri::command]
async fn github_comments(
    gh: State<'_, github::GitHubState>,
    input: github::PullNumberInput,
) -> Result<Vec<github::GitHubComment>, String> {
    let gh = (*gh).clone();
    spawn("github_comments", move || {
        github::comentarios(&gh, &input.repo, input.number)
    })
    .await
}

#[tauri::command]
async fn github_branches(
    gh: State<'_, github::GitHubState>,
    repo: String,
) -> Result<Vec<String>, String> {
    let gh = (*gh).clone();
    spawn("github_branches", move || github::ramos(&gh, &repo)).await
}

#[tauri::command]
async fn github_create_pull(
    state: State<'_, AppState>,
    gh: State<'_, github::GitHubState>,
    input: github::NewPullInput,
) -> Result<github::PullDetail, String> {
    let db = state.db.clone();
    let gh = (*gh).clone();
    spawn("github_create_pull", move || {
        let conn = lock_db(&db)?;
        github::create_pull(&conn, &gh, &input)
    })
    .await
}

/// O pull request vira trabalho local: busca a cabeça do PR e muda para a
/// branch de leitura. Sem remoto informado, usa o mesmo da publicação.
#[tauri::command]
async fn git_checkout_pull(
    state: State<'_, AppState>,
    project_id: i64,
    remote: Option<String>,
    number: i64,
) -> Result<GitActionOutcome, String> {
    let db = state.db.clone();
    let remoto = trim_opt(remote);
    let detalhe = format!("leitura do pull request #{number}");
    git_write(db, project_id, "git_checkout_pull", detalhe, move |dir| {
        let alvo = match &remoto {
            Some(r) => r.clone(),
            None => git::default_remote(dir)?,
        };
        git::checkout_pull(dir, &alvo, number).map(outcome)
    })
    .await
}

// Análise de código-fonte (plano CI/CD, etapa 5). A segunda porta da mesma
// mecânica: a tela fala com estes comandos e o nó `analise` do grafo chama a
// mesma `analysis::executar` por dentro do motor — uma varredura, um veredito.

#[tauri::command]
async fn analysis_detectar(state: State<'_, AppState>) -> Result<analysis::Ferramenta, String> {
    let db = state.db.clone();
    spawn("analysis_detectar", move || {
        let conn = lock_db(&db)?;
        analysis::sonda_ferramenta(&conn)
    })
    .await
}

#[tauri::command]
async fn analysis_regras() -> Result<Vec<analysis::RegraInfo>, String> {
    Ok(analysis::regras_embarcadas())
}

#[tauri::command]
async fn analysis_conferir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<analysis::Validacao, String> {
    let db = state.db.clone();
    let dados = db::pasta_de_dados(&app)?;
    spawn("analysis_conferir", move || {
        let conn = lock_db(&db)?;
        analysis::conferir_regras(&conn, &dados)
    })
    .await
}

/// Roda a varredura pedida e devolve a corrida como ela ficou gravada. A varredura
/// é lenta de propósito (ler o projeto inteiro), por isso vai para a thread
/// bloqueante e por isso o registro recusa a segunda do mesmo projeto.
#[tauri::command]
async fn analysis_executar(
    app: AppHandle,
    state: State<'_, AppState>,
    varreduras: State<'_, analysis::Estado>,
    entrada: AnalysisInput,
) -> Result<AnalysisRun, String> {
    let projeto = {
        let conn = lock_db(&state.db)?;
        db::get_project(&conn, entrada.project_id)?
    };
    let nome = projeto.name.clone();
    let raiz = analysis::raiz_do_projeto(&projeto)?;
    let dados = db::pasta_de_dados(&app)?;
    let db = state.db.clone();
    let id = projeto.id;
    let flag = varreduras.iniciar(id)?;
    let resultado = spawn("analysis_executar", move || {
        let conn = lock_db(&db)?;
        analysis::executar(&conn, &dados, &projeto, &raiz, &entrada, &flag)
    })
    .await;
    varreduras.terminar(id);
    let corrida = resultado?;
    let conn = lock_db(&state.db)?;
    let _ = audit::log(
        &conn,
        if corrida.status != "concluida" {
            "error"
        } else if corrida.blocked {
            "warning"
        } else {
            "info"
        },
        "análise",
        &format!("Varredura {} · {}", corrida.tool, nome),
        None,
        Some(&nome),
        corrida.exit_code,
        Some(&format!(
            "corrida #{} · {} arquivo(s) · {} achado(s): {} erro, {} aviso, {} info · {}",
            corrida.id,
            corrida.files_scanned,
            corrida.counts.total(),
            corrida.counts.error,
            corrida.counts.warning,
            corrida.counts.info,
            corrida.note.as_deref().unwrap_or("sem nota")
        )),
    );
    Ok(corrida)
}

#[tauri::command]
async fn analysis_interromper(
    varreduras: State<'_, analysis::Estado>,
    project_id: i64,
) -> Result<bool, String> {
    Ok(varreduras.interromper(project_id))
}

#[tauri::command]
async fn analysis_varrendo(varreduras: State<'_, analysis::Estado>) -> Result<Vec<i64>, String> {
    Ok(varreduras.em_andamento())
}

#[tauri::command]
async fn analysis_corridas(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<AnalysisRun>, String> {
    let conn = lock_db(&state.db)?;
    db::listar_analises(&conn, project_id, limit.unwrap_or(50).clamp(1, 200))
}

#[tauri::command]
async fn analysis_achados(
    state: State<'_, AppState>,
    run_id: i64,
    severidade: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<AnalysisFinding>, String> {
    let conn = lock_db(&state.db)?;
    db::listar_achados(
        &conn,
        run_id,
        severidade.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        limit.unwrap_or(500).clamp(1, 5000),
    )
}

/// O pacote que uma IA lê para decidir. É a mesma corrida da tela, em texto
/// determinístico, com o que não foi lido dito em voz alta.
#[tauri::command]
async fn analysis_pacote(
    state: State<'_, AppState>,
    run_id: i64,
    limite: Option<usize>,
) -> Result<serde_json::Value, String> {
    let conn = lock_db(&state.db)?;
    analysis::pacote(&conn, run_id, limite)
}

#[tauri::command]
async fn analysis_exportar(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    formato: String,
    limite: Option<usize>,
) -> Result<String, String> {
    let db = state.db.clone();
    let dados = db::pasta_de_dados(&app)?;
    spawn("analysis_exportar", move || {
        let conn = lock_db(&db)?;
        analysis::exportar(&conn, &dados, run_id, formato.trim(), limite)
    })
    .await
}

#[tauri::command]
async fn analysis_remover(state: State<'_, AppState>, run_id: i64) -> Result<(), String> {
    let corrida = {
        let conn = lock_db(&state.db)?;
        let corrida = db::obter_analise(&conn, run_id)?;
        db::apagar_analise(&conn, run_id)?;
        corrida
    };
    let conn = lock_db(&state.db)?;
    let _ = audit::log(
        &conn,
        "warning",
        "análise",
        "Corrida de análise removida",
        None,
        Some(&corrida.project),
        None,
        Some(&format!(
            "corrida #{run_id} · {} · {} achado(s)",
            corrida.tool,
            corrida.counts.total()
        )),
    );
    Ok(())
}

pub fn register_all(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        get_settings,
        set_setting,
        get_app_data_dir,
        get_tool_versions,
        list_servers,
        add_server,
        update_server,
        delete_server,
        test_connection,
        server_metrics,
        provide_password,
        cancel_password_prompt,
        discover_ssh,
        import_ssh_hosts,
        list_projects,
        add_project,
        refresh_project,
        update_project,
        delete_project,
        list_deploy_profiles,
        save_deploy_profile,
        delete_deploy_profile,
        set_default_deploy_profile,
        git_status,
        git_identity,
        git_diff,
        git_stage,
        git_unstage,
        git_stage_hunks,
        git_discard,
        scan_project_secrets,
        git_secret_check,
        git_untrack_file,
        git_commit,
        git_graph,
        git_commit_detail,
        git_branches,
        git_checkout,
        git_rename_branch,
        git_delete_branch,
        git_merge,
        git_rebase,
        git_abort,
        git_continue_pending,
        git_skip_pending,
        git_resolve_conflict,
        git_revert,
        git_cherry_pick,
        git_stash_list,
        git_stash_push,
        git_stash_apply,
        git_stash_drop,
        git_reset,
        git_fetch,
        git_pull,
        git_push,
        git_publish,
        git_unpublish_tag,
        git_check_remote,
        git_tags,
        git_create_tag,
        git_delete_tag,
        git_remotes,
        git_tree,
        git_file_content,
        git_add_remote,
        git_set_remote_url,
        git_remove_remote,
        git_clone,
        open_in_editor,
        query_audit_log,
        search_all,
        open_terminal_session,
        write_terminal_session,
        resize_terminal_session,
        close_terminal_session,
        list_terminal_sessions,
        list_routines,
        save_routine,
        delete_routine,
        record_terminal_command,
        list_terminal_history,
        clear_terminal_history,
        sftp_list,
        sftp_mkdir,
        sftp_rename,
        sftp_remove,
        sftp_remove_dir,
        sftp_chmod,
        sftp_read_text,
        sftp_write_text,
        transfer_start,
        transfer_cancel,
        transfers_list,
        deploy_preflight,
        deploy_start,
        deploy_rollback,
        deploy_cancel,
        deploy_active,
        deploy_runs,
        deploy_run,
        pipeline_definitions,
        pipeline_revisions,
        pipeline_validate,
        pipeline_retry_policy,
        pipeline_retry,
        pipeline_nodes,
        pipeline_save_nodes,
        pipeline_check_nodes,
        pipeline_engine_steps,
        pipeline_run_node,
        pipeline_approve,
        pipeline_approvals_pending,
        analysis_detectar,
        analysis_regras,
        analysis_conferir,
        analysis_executar,
        analysis_interromper,
        analysis_varrendo,
        analysis_corridas,
        analysis_achados,
        analysis_pacote,
        analysis_exportar,
        analysis_remover,
        github_status,
        github_set_client_id,
        github_connect_start,
        github_connect_poll,
        github_disconnect,
        github_repos,
        github_pulls,
        github_create_repo,
        github_inbox,
        github_pull_detail,
        github_pull_files,
        github_issues,
        github_runs,
        github_create_issue,
        github_comment,
        github_close_issue,
        github_review,
        github_merge,
        github_rerun,
        github_comments,
        github_branches,
        github_create_pull,
        git_checkout_pull,
    ])
}
#[cfg(test)]
mod git_helpers_tests {
    use super::*;

    #[test]
    fn url_com_credencial_e_redigida() {
        assert_eq!(
            redact_url("https://DouglasGDev:ghp_SECRET123@github.com/acme/app.git"),
            "https://***@github.com/acme/app.git"
        );
        // sem credencial permanece igual
        assert_eq!(
            redact_url("git@github.com:acme/app.git"),
            "git@github.com:acme/app.git"
        );
        assert_eq!(
            redact_url("ssh://deploy@10.0.0.5:2222/srv/app.git"),
            "ssh://***@10.0.0.5:2222/srv/app.git"
        );
    }

    #[test]
    fn mensagem_de_erro_com_url_e_redigida_sem_duplicar_o_resto() {
        let raw = "fatal: unable to access 'https://user:token@github.com/acme/app.git/'";
        let red = redact_url(raw);
        assert!(red.contains("fatal: unable to access"));
        assert!(red.contains("https://***@github.com/acme/app.git/"));
        assert!(!red.contains("token"));
        // a cauda aparece exatamente uma vez
        assert_eq!(red.matches("app.git").count(), 1);
    }

    #[test]
    fn saida_longa_de_comando_local_mostra_so_o_fim() {
        let longo = (1..=40)
            .map(|n| format!("linha {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cortado = last_lines(&longo, 8);
        assert!(cortado.starts_with("…\n"), "avisa que houve corte");
        assert_eq!(cortado.lines().count(), 9);
        assert!(cortado.ends_with("linha 40"), "o fim é o que importa");
        assert_eq!(last_lines("a\nb", 8), "a\nb", "curta fica inteira");
        assert_eq!(last_lines("  \n\n  ", 8), "(sem saída)");
    }

    #[test]
    fn falha_local_prefere_o_stderr_e_usa_o_stdout_como_plano_b() {
        let com_erro = sshexec::ExecResult {
            exit_code: Some(1),
            stdout: "42 testes executados\n".into(),
            stderr: "erro: falta a variável TOKEN\n".into(),
            timed_out: false,
            cancelled: false,
        };
        assert_eq!(failure_detail(&com_erro), "erro: falta a variável TOKEN");
        let sem_erro = sshexec::ExecResult {
            exit_code: Some(2),
            stdout: "rodou até aqui\n".into(),
            stderr: "   ".into(),
            timed_out: false,
            cancelled: false,
        };
        assert_eq!(failure_detail(&sem_erro), "rodou até aqui");
    }

    #[test]
    fn assunto_do_commit_cabe_na_auditoria() {
        assert_eq!(subject_line("  corrige o login\n\ne mais corpo"), "corrige o login");
        assert_eq!(subject_line("\no corpo sozinho"), "(sem mensagem)");
        let longa = "x".repeat(200);
        let cortada = subject_line(&longa);
        assert_eq!(cortada.chars().count(), 121);
        assert!(cortada.ends_with('…'));
    }

    #[test]
    fn deep_links_de_editor() {
        let dir = Path::new("C:/Users/dougl/Projetos/com meu app");
        assert_eq!(
            editor_target("vscode", Some(dir)),
            Some("vscode://file/C:/Users/dougl/Projetos/com%20meu%20app".into())
        );
        assert!(editor_target("cursor", Some(dir))
            .unwrap()
            .starts_with("cursor://file/C:/Users/dougl/Projetos/com%20meu%20app"));
        // editor do sistema: nenhum deep link, o chamador usa o manipulador padrão
        assert_eq!(editor_target("system", Some(dir)), None);
        assert_eq!(editor_target("vscode", None), None);
    }

    #[test]
    fn rota_do_editor_prefere_o_executavel_encontrado() {
        let dir = Path::new("C:/Users/dougl/app");
        let instalados = vec![EditorInstalado {
            id: "vscode".into(),
            nome: "Visual Studio Code".into(),
            caminho: "C:/Program Files/Microsoft VS Code/Code.exe".into(),
        }];
        // o editor que a detecção achou é o que abre o arquivo
        assert_eq!(
            rota_do_editor("vscode", Some(dir), &instalados),
            RotaEditor::Executavel("C:/Program Files/Microsoft VS Code/Code.exe".into())
        );
        // escolha que não existe no disco ainda tem o link profundo como recurso
        assert_eq!(
            rota_do_editor("cursor", Some(dir), &instalados),
            RotaEditor::Link("cursor://file/C:/Users/dougl/app".into())
        );
        // o padrão do sistema e um id desconhecido vão para o manipulador nativo
        assert_eq!(rota_do_editor("system", Some(dir), &instalados), RotaEditor::Sistema);
        assert_eq!(rota_do_editor("gedit", Some(dir), &instalados), RotaEditor::Sistema);
        // sem arquivo alvo não há o que entregar ao editor
        assert_eq!(rota_do_editor("vscode", None, &instalados), RotaEditor::Sistema);
    }

    #[test]
    fn caminho_verbatim_do_windows_nao_aparece_no_deep_link() {
        let dir = Path::new(r"\\?\C:\Users\dougl\app");
        assert_eq!(
            editor_target("vscode", Some(dir)),
            Some("vscode://file/C:/Users/dougl/app".into())
        );
    }

    #[test]
    fn espacos_em_referencias_e_caminhos_passam_pela_validacao_basica() {
        assert!(validate_file_path("src/meu arquivo.ts").is_ok());
        assert!(validate_file_path("").is_err());
        assert!(validate_file_path(&"a".repeat(1025)).is_err());
    }

    #[test]
    fn opcoes_opcionais_sao_aparadas_ou_nulas() {
        assert_eq!(trim_opt(Some("  origin ".into())), Some("origin".into()));
        assert_eq!(trim_opt(Some("   ".into())), None);
        assert_eq!(trim_opt(None), None);
    }
}
