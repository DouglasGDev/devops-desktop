use std::{path::PathBuf, sync::Arc, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, Row};
use tauri::Manager;

use crate::models::{
    AuditEntry, DeployProfile, DeployProfileInput, DeployRun, DeployRunStep, Project, Routine,
    RoutineInput, RoutineParam, Server, ServerInput, TerminalHistoryEntry,
};

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
}

/// Alça compartilhada do banco: todos os módulos que precisam de acesso fora
/// da thread principal usam este tipo em vez de repetir o alias.
pub type DbHandle = Arc<Mutex<Connection>>;

impl AppState {
    pub fn new(conn: Connection) -> Self {
        Self { db: Arc::new(Mutex::new(conn)) }
    }
}

/// Migrações incrementais. O índice do vetor é a versão do esquema.
const MIGRATIONS: &[&str] = &[
    // v1 — estado inicial
    "CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE IF NOT EXISTS servers (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        host TEXT NOT NULL,
        port INTEGER NOT NULL DEFAULT 22,
        username TEXT NOT NULL,
        auth_method TEXT NOT NULL DEFAULT 'unknown',
        identity_file TEXT,
        ssh_alias TEXT,
        environment TEXT NOT NULL DEFAULT 'dev',
        group_name TEXT,
        notes TEXT,
        source TEXT NOT NULL DEFAULT 'manual',
        last_status TEXT,
        last_status_detail TEXT,
        last_checked_at TEXT,
        last_ok_at TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS projects (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        local_path TEXT NOT NULL UNIQUE,
        is_git_repo INTEGER NOT NULL DEFAULT 0,
        git_branch TEXT,
        git_remote TEXT,
        default_server_id INTEGER REFERENCES servers(id) ON DELETE SET NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts TEXT NOT NULL,
        level TEXT NOT NULL,
        category TEXT NOT NULL,
        operation TEXT NOT NULL,
        server_name TEXT,
        project_name TEXT,
        exit_code INTEGER,
        detail TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts DESC);
    CREATE INDEX IF NOT EXISTS idx_audit_category ON audit_log(category);
    CREATE INDEX IF NOT EXISTS idx_servers_env ON servers(environment);",
    // v2 — terminal SSH: rotinas do executor e histórico local de comandos
    r#"
    CREATE TABLE IF NOT EXISTS routines (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        description TEXT,
        command TEXT NOT NULL,
        danger TEXT NOT NULL DEFAULT 'read' CHECK (danger IN ('read', 'write')),
        params TEXT NOT NULL DEFAULT '[]',
        is_builtin INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS terminal_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts TEXT NOT NULL,
        server_id INTEGER,
        server_name TEXT,
        command TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_terminal_history_ts ON terminal_history(ts DESC);
    INSERT OR IGNORE INTO routines (name, description, command, danger, params, is_builtin, created_at, updated_at) VALUES
        ('Uso de recursos (CPU, RAM e disco)', 'Visão geral de consumo: processos, memória e disco', 'top -bn1 | head -n 15; echo; free -h; echo; df -h', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Processos ordenados por CPU', 'Os 15 processos que mais consomem CPU', 'ps aux --sort=-%cpu | head -n 15', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Portas TCP/UDP em escuta', 'Serviços e portas abertas no servidor', 'ss -tulpn 2>/dev/null || netstat -tulpn 2>/dev/null | head -n 40', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Espaço em disco por partição', 'Uso de cada partição montada', 'df -h', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Containers Docker', 'Todos os containers, inclusive os parados', 'docker ps -a', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Imagens Docker', 'Imagens locais e seus tamanhos', 'docker images', 'read', '[]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Últimas linhas de um log', 'Mostra as 100 últimas linhas de um arquivo de log', 'tail -n 100 {{arquivo}}', 'read', '[{"name":"arquivo","label":"Caminho do arquivo de log","required":true,"default":null}]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Arquivos grandes (acima de 100 MB)', 'Procura arquivos grandes a partir de um diretório', 'find {{diretorio}} -type f -size +100M -exec ls -lh {} \; 2>/dev/null | head -n 20', 'read', '[{"name":"diretorio","label":"Diretório base da busca","required":false,"default":"/var"}]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        ('Status de serviço systemd', 'Situação atual de um serviço do sistema', 'systemctl status {{servico}} --no-pager -l', 'read', '[{"name":"servico","label":"Nome do serviço (ex.: nginx)","required":true,"default":null}]', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
    "#,
    // v3 — modelos de deploy e histórico das execuções do pipeline
    r#"
    CREATE TABLE IF NOT EXISTS deploy_profiles (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        server_id INTEGER NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
        strategy TEXT NOT NULL CHECK (strategy IN ('sftp','git','artifacts','script','docker')),
        git_branch TEXT,
        remote_path TEXT NOT NULL,
        include_paths TEXT NOT NULL DEFAULT '[]',
        exclude_paths TEXT NOT NULL DEFAULT '[]',
        prepare_commands TEXT NOT NULL DEFAULT '[]',
        build_commands TEXT NOT NULL DEFAULT '[]',
        restart_commands TEXT NOT NULL DEFAULT '[]',
        health_checks TEXT NOT NULL DEFAULT '[]',
        rollback_commands TEXT NOT NULL DEFAULT '[]',
        secret_keys TEXT NOT NULL DEFAULT '[]',
        backup_enabled INTEGER NOT NULL DEFAULT 1,
        releases_enabled INTEGER NOT NULL DEFAULT 0,
        delete_extras INTEGER NOT NULL DEFAULT 0,
        step_timeout_secs INTEGER NOT NULL DEFAULT 600,
        is_default INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        UNIQUE (project_id, name)
    );
    CREATE INDEX IF NOT EXISTS idx_profiles_project ON deploy_profiles(project_id);
    CREATE TABLE IF NOT EXISTS deploy_runs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
        status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','success','failed','cancelled')),
        started_at TEXT NOT NULL,
        finished_at TEXT,
        release_dir TEXT,
        previous_release TEXT,
        detail TEXT
    );
    CREATE TABLE IF NOT EXISTS deploy_run_steps (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id INTEGER NOT NULL REFERENCES deploy_runs(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        name TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'waiting',
        command TEXT,
        exit_code INTEGER,
        message TEXT,
        started_at TEXT,
        finished_at TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_run_steps_run ON deploy_run_steps(run_id, position);
    CREATE INDEX IF NOT EXISTS idx_runs_started ON deploy_runs(started_at DESC);
    "#,
    // v4 — testes automatizados da etapa 6 do pipeline
    r#"
    ALTER TABLE deploy_profiles ADD COLUMN test_commands TEXT NOT NULL DEFAULT '[]';
    "#,
];

pub fn init(app: &tauri::AppHandle) -> Result<Connection, String> {
    let dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("não foi possível determinar o diretório de dados: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("não foi possível criar o diretório de dados {}: {e}", dir.display()))?;
    let db_path = dir.join("devops-desktop.db");
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("não foi possível abrir o banco {}: {e}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("falha ao definir timeout do banco: {e}"))?;
    let _wal: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .map_err(|e| format!("falha ao ativar WAL: {e}"))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| format!("falha ao ler versão do esquema: {e}"))?;
    for (i, script) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as i64;
        if version < target {
            conn.execute_batch(script)
                .map_err(|e| format!("falha na migração v{target}: {e}"))?;
            conn.pragma_update(None, "user_version", target)
                .map_err(|e| format!("falha ao registrar versão do esquema: {e}"))?;
        }
    }
    Ok(())
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn row_to_server(row: &Row) -> rusqlite::Result<Server> {
    Ok(Server {
        id: row.get("id")?,
        name: row.get("name")?,
        host: row.get("host")?,
        port: row.get("port")?,
        username: row.get("username")?,
        auth_method: row.get("auth_method")?,
        identity_file: row.get("identity_file")?,
        ssh_alias: row.get("ssh_alias")?,
        environment: row.get("environment")?,
        group_name: row.get("group_name")?,
        notes: row.get("notes")?,
        source: row.get("source")?,
        last_status: row.get("last_status")?,
        last_status_detail: row.get("last_status_detail")?,
        last_checked_at: row.get("last_checked_at")?,
        last_ok_at: row.get("last_ok_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_project(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        name: row.get("name")?,
        local_path: row.get("local_path")?,
        is_git_repo: row.get::<_, i64>("is_git_repo")? != 0,
        git_branch: row.get("git_branch")?,
        git_remote: row.get("git_remote")?,
        default_server_id: row.get("default_server_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn row_to_audit(row: &Row) -> rusqlite::Result<AuditEntry> {
    Ok(AuditEntry {
        id: row.get("id")?,
        ts: row.get("ts")?,
        level: row.get("level")?,
        category: row.get("category")?,
        operation: row.get("operation")?,
        server_name: row.get("server_name")?,
        project_name: row.get("project_name")?,
        exit_code: row.get("exit_code")?,
        detail: row.get("detail")?,
    })
}

const SERVER_COLUMNS: &str = "id, name, host, port, username, auth_method, identity_file, ssh_alias, environment, group_name, notes, source, last_status, last_status_detail, last_checked_at, last_ok_at, created_at, updated_at";
const PROJECT_COLUMNS: &str = "id, name, local_path, is_git_repo, git_branch, git_remote, default_server_id, created_at, updated_at";

pub fn list_servers(conn: &Connection) -> Result<Vec<Server>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {SERVER_COLUMNS} FROM servers ORDER BY environment, name"))
        .map_err(|e| format!("falha ao consultar servidores: {e}"))?;
    let rows = stmt
        .query_map([], row_to_server)
        .map_err(|e| format!("falha ao consultar servidores: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler servidor: {e}"))?);
    }
    Ok(out)
}

pub fn get_server(conn: &Connection, id: i64) -> Result<Server, String> {
    conn.query_row(
        &format!("SELECT {SERVER_COLUMNS} FROM servers WHERE id = ?1"),
        [id],
        row_to_server,
    )
    .map_err(|e| format!("servidor #{id} não encontrado: {e}"))
}

pub fn insert_server(conn: &Connection, input: &ServerInput, source: &str) -> Result<Server, String> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO servers (name, host, port, username, auth_method, identity_file, ssh_alias, environment, group_name, notes, source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        rusqlite::params![
            input.name.trim(),
            input.host.trim(),
            input.port,
            input.username.trim(),
            input.auth_method,
            input.identity_file,
            input.ssh_alias,
            input.environment,
            input.group_name,
            input.notes,
            source,
            now,
        ],
    )
    .map_err(|e| format!("falha ao cadastrar servidor: {e}"))?;
    let id = conn.last_insert_rowid();
    get_server(conn, id)
}

pub fn update_server_row(conn: &Connection, id: i64, input: &ServerInput) -> Result<Server, String> {
    let now = now_rfc3339();
    let affected = conn
        .execute(
            "UPDATE servers SET name = ?1, host = ?2, port = ?3, username = ?4, auth_method = ?5, identity_file = ?6, ssh_alias = ?7, environment = ?8, group_name = ?9, notes = ?10, updated_at = ?11 WHERE id = ?12",
            rusqlite::params![
                input.name.trim(),
                input.host.trim(),
                input.port,
                input.username.trim(),
                input.auth_method,
                input.identity_file,
                input.ssh_alias,
                input.environment,
                input.group_name,
                input.notes,
                now,
                id,
            ],
        )
        .map_err(|e| format!("falha ao atualizar servidor: {e}"))?;
    if affected == 0 {
        return Err(format!("servidor #{id} não encontrado"));
    }
    get_server(conn, id)
}

pub fn delete_server_row(conn: &Connection, id: i64) -> Result<(), String> {
    let affected = conn
        .execute("DELETE FROM servers WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover servidor: {e}"))?;
    if affected == 0 {
        return Err(format!("servidor #{id} não encontrado"));
    }
    Ok(())
}

pub fn server_exists(conn: &Connection, alias: &str, host: &str, user: &str, port: i64) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM servers WHERE (ssh_alias = ?1 AND ?1 IS NOT NULL) OR (host = ?2 AND username = ?3 AND port = ?4)",
        rusqlite::params![alias, host, user, port],
        |r| r.get::<_, i64>(0),
    )
    .map(|c| c > 0)
    .map_err(|e| format!("falha ao verificar duplicidade: {e}"))
}

pub fn record_connection_status(
    conn: &Connection,
    id: i64,
    ok: bool,
    detail: &str,
) -> Result<(), String> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE servers SET last_status = ?1, last_status_detail = ?2, last_checked_at = ?3, last_ok_at = CASE WHEN ?1 = 'ok' THEN ?3 ELSE last_ok_at END WHERE id = ?4",
        rusqlite::params![if ok { "ok" } else { "failed" }, detail, now, id],
    )
    .map_err(|e| format!("falha ao registrar status da conexão: {e}"))?;
    Ok(())
}

pub fn list_projects(conn: &Connection) -> Result<Vec<Project>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {PROJECT_COLUMNS} FROM projects ORDER BY name"))
        .map_err(|e| format!("falha ao consultar projetos: {e}"))?;
    let rows = stmt
        .query_map([], row_to_project)
        .map_err(|e| format!("falha ao consultar projetos: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler projeto: {e}"))?);
    }
    Ok(out)
}

pub fn get_project(conn: &Connection, id: i64) -> Result<Project, String> {
    conn.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
        [id],
        row_to_project,
    )
    .map_err(|e| format!("projeto #{id} não encontrado: {e}"))
}

pub fn insert_project(
    conn: &Connection,
    name: &str,
    local_path: &str,
    is_git_repo: bool,
    git_branch: Option<&str>,
    git_remote: Option<&str>,
) -> Result<Project, String> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO projects (name, local_path, is_git_repo, git_branch, git_remote, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        rusqlite::params![name, local_path, is_git_repo as i64, git_branch, git_remote, now],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            format!("esta pasta já está cadastrada como projeto: {local_path}")
        } else {
            format!("falha ao cadastrar projeto: {e}")
        }
    })?;
    get_project(conn, conn.last_insert_rowid())
}

pub fn update_project_row(
    conn: &Connection,
    id: i64,
    name: &str,
    default_server_id: Option<i64>,
) -> Result<Project, String> {
    if let Some(sid) = default_server_id {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM servers WHERE id = ?1", [sid], |r| r.get(0))
            .map_err(|e| format!("falha ao validar servidor: {e}"))?;
        if count == 0 {
            return Err(format!("servidor #{sid} não existe"));
        }
    }
    let now = now_rfc3339();
    let affected = conn
        .execute(
            "UPDATE projects SET name = ?1, default_server_id = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![name, default_server_id, now, id],
        )
        .map_err(|e| format!("falha ao atualizar projeto: {e}"))?;
    if affected == 0 {
        return Err(format!("projeto #{id} não encontrado"));
    }
    get_project(conn, id)
}

pub fn update_project_git(
    conn: &Connection,
    id: i64,
    is_git_repo: bool,
    git_branch: Option<&str>,
    git_remote: Option<&str>,
) -> Result<Project, String> {
    let now = now_rfc3339();
    conn.execute(
        "UPDATE projects SET is_git_repo = ?1, git_branch = ?2, git_remote = ?3, updated_at = ?4 WHERE id = ?5",
        rusqlite::params![is_git_repo as i64, git_branch, git_remote, now, id],
    )
    .map_err(|e| format!("falha ao atualizar dados Git do projeto: {e}"))?;
    get_project(conn, id)
}

pub fn delete_project_row(conn: &Connection, id: i64) -> Result<(), String> {
    let affected = conn
        .execute("DELETE FROM projects WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover projeto: {e}"))?;
    if affected == 0 {
        return Err(format!("projeto #{id} não encontrado"));
    }
    Ok(())
}

fn clean_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

const ROUTINE_COLUMNS: &str = "id, name, description, command, danger, params, is_builtin, created_at, updated_at";

fn row_to_routine(row: &Row) -> rusqlite::Result<Routine> {
    let params_json: String = row.get("params")?;
    let params: Vec<RoutineParam> = serde_json::from_str(&params_json).unwrap_or_default();
    Ok(Routine {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        command: row.get("command")?,
        danger: row.get("danger")?,
        params,
        is_builtin: row.get::<_, i64>("is_builtin")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list_routines(conn: &Connection) -> Result<Vec<Routine>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {ROUTINE_COLUMNS} FROM routines ORDER BY is_builtin DESC, name"))
        .map_err(|e| format!("falha ao consultar rotinas: {e}"))?;
    let rows = stmt
        .query_map([], row_to_routine)
        .map_err(|e| format!("falha ao consultar rotinas: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler rotina: {e}"))?);
    }
    Ok(out)
}

pub fn get_routine(conn: &Connection, id: i64) -> Result<Routine, String> {
    conn.query_row(
        &format!("SELECT {ROUTINE_COLUMNS} FROM routines WHERE id = ?1"),
        [id],
        row_to_routine,
    )
    .map_err(|e| format!("rotina #{id} não encontrada: {e}"))
}

fn routine_params_json(params: &[RoutineParam]) -> Result<String, String> {
    serde_json::to_string(params).map_err(|e| format!("falha ao serializar parâmetros: {e}"))
}

pub fn insert_routine(conn: &Connection, input: &RoutineInput) -> Result<Routine, String> {
    let now = now_rfc3339();
    let params_json = routine_params_json(&input.params)?;
    conn.execute(
        "INSERT INTO routines (name, description, command, danger, params, is_builtin, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
        rusqlite::params![
            input.name.trim(),
            clean_optional(&input.description),
            input.command.trim(),
            input.danger,
            params_json,
            now,
        ],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            format!("já existe uma rotina chamada \"{}\"", input.name.trim())
        } else {
            format!("falha ao salvar rotina: {e}")
        }
    })?;
    get_routine(conn, conn.last_insert_rowid())
}

pub fn update_routine_row(conn: &Connection, id: i64, input: &RoutineInput) -> Result<Routine, String> {
    let now = now_rfc3339();
    let params_json = routine_params_json(&input.params)?;
    let affected = conn
        .execute(
            "UPDATE routines SET name = ?1, description = ?2, command = ?3, danger = ?4, params = ?5, updated_at = ?6 WHERE id = ?7",
            rusqlite::params![
                input.name.trim(),
                clean_optional(&input.description),
                input.command.trim(),
                input.danger,
                params_json,
                now,
                id,
            ],
        )
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                format!("já existe uma rotina chamada \"{}\"", input.name.trim())
            } else {
                format!("falha ao atualizar rotina: {e}")
            }
        })?;
    if affected == 0 {
        return Err(format!("rotina #{id} não encontrada"));
    }
    get_routine(conn, id)
}

pub fn delete_routine_row(conn: &Connection, id: i64) -> Result<(), String> {
    let affected = conn
        .execute("DELETE FROM routines WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover rotina: {e}"))?;
    if affected == 0 {
        return Err(format!("rotina #{id} não encontrada"));
    }
    Ok(())
}

const PROFILE_COLUMNS: &str = "p.id, p.project_id, p.name, p.server_id, s.name AS server_name, \
     s.environment AS server_environment, p.strategy, p.git_branch, p.remote_path, p.include_paths, \
     p.exclude_paths, p.prepare_commands, p.build_commands, p.restart_commands, p.health_checks, \
     p.rollback_commands, p.secret_keys, p.backup_enabled, p.releases_enabled, p.delete_extras, \
     p.step_timeout_secs, p.is_default, p.created_at, p.updated_at, p.test_commands";

fn json_list(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn to_json_list(items: &[String]) -> Result<String, String> {
    let trimmed: Vec<String> = items
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    serde_json::to_string(&trimmed).map_err(|e| format!("falha ao serializar lista: {e}"))
}

fn row_to_profile(row: &Row) -> rusqlite::Result<DeployProfile> {
    Ok(DeployProfile {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        name: row.get("name")?,
        server_id: row.get("server_id")?,
        server_name: row.get("server_name")?,
        server_environment: row.get("server_environment")?,
        strategy: row.get("strategy")?,
        git_branch: row.get("git_branch")?,
        remote_path: row.get("remote_path")?,
        include_paths: json_list(row.get::<_, String>("include_paths")?.as_str()),
        exclude_paths: json_list(row.get::<_, String>("exclude_paths")?.as_str()),
        prepare_commands: json_list(row.get::<_, String>("prepare_commands")?.as_str()),
        build_commands: json_list(row.get::<_, String>("build_commands")?.as_str()),
        restart_commands: json_list(row.get::<_, String>("restart_commands")?.as_str()),
        health_checks: json_list(row.get::<_, String>("health_checks")?.as_str()),
        rollback_commands: json_list(row.get::<_, String>("rollback_commands")?.as_str()),
        test_commands: json_list(row.get::<_, String>("test_commands")?.as_str()),
        secret_keys: json_list(row.get::<_, String>("secret_keys")?.as_str()),
        backup_enabled: row.get::<_, i64>("backup_enabled")? != 0,
        releases_enabled: row.get::<_, i64>("releases_enabled")? != 0,
        delete_extras: row.get::<_, i64>("delete_extras")? != 0,
        step_timeout_secs: row.get("step_timeout_secs")?,
        is_default: row.get::<_, i64>("is_default")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn exists(conn: &Connection, table: &str, id: i64, label: &str) -> Result<(), String> {
    let count: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {table} WHERE id = ?1"), [id], |r| r.get(0))
        .map_err(|e| format!("falha ao validar {label}: {e}"))?;
    if count == 0 {
        return Err(format!("{label} #{id} não existe"));
    }
    Ok(())
}

pub fn list_deploy_profiles(
    conn: &Connection,
    project_id: Option<i64>,
) -> Result<Vec<DeployProfile>, String> {
    let sql = match project_id {
        Some(_) => format!(
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p JOIN servers s ON s.id = p.server_id \
             WHERE p.project_id = ?1 ORDER BY p.is_default DESC, p.name"
        ),
        None => format!(
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p JOIN servers s ON s.id = p.server_id \
             ORDER BY p.is_default DESC, p.name"
        ),
    };
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("falha ao consultar modelos de deploy: {e}"))?;
    let mut out = Vec::new();
    match project_id {
        Some(pid) => {
            let rows = stmt
                .query_map([pid], row_to_profile)
                .map_err(|e| format!("falha ao consultar modelos de deploy: {e}"))?;
            for r in rows {
                out.push(r.map_err(|e| format!("falha ao ler modelo de deploy: {e}"))?);
            }
        }
        None => {
            let rows = stmt
                .query_map([], row_to_profile)
                .map_err(|e| format!("falha ao consultar modelos de deploy: {e}"))?;
            for r in rows {
                out.push(r.map_err(|e| format!("falha ao ler modelo de deploy: {e}"))?);
            }
        }
    }
    Ok(out)
}

pub fn get_deploy_profile(conn: &Connection, id: i64) -> Result<DeployProfile, String> {
    conn.query_row(
        &format!(
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p JOIN servers s ON s.id = p.server_id WHERE p.id = ?1"
        ),
        [id],
        row_to_profile,
    )
    .map_err(|e| format!("modelo de deploy #{id} não encontrado: {e}"))
}

pub fn upsert_deploy_profile(
    conn: &Connection,
    input: &DeployProfileInput,
) -> Result<DeployProfile, String> {
    input.validate()?;
    exists(conn, "projects", input.project_id, "projeto")?;
    exists(conn, "servers", input.server_id, "servidor")?;

    let now = now_rfc3339();
    let include = to_json_list(&input.include_paths)?;
    let exclude = to_json_list(&input.exclude_paths)?;
    let prepare = to_json_list(&input.prepare_commands)?;
    let build = to_json_list(&input.build_commands)?;
    let restart = to_json_list(&input.restart_commands)?;
    let health = to_json_list(&input.health_checks)?;
    let rollback = to_json_list(&input.rollback_commands)?;
    let tests = to_json_list(&input.test_commands)?;
    let keys = to_json_list(&input.secret_keys)?;
    let branch = clean_optional(&input.git_branch);

    match input.id {
        Some(id) => {
            let affected = conn
                .execute(
                    "UPDATE deploy_profiles SET project_id = ?1, name = ?2, server_id = ?3, \
                     strategy = ?4, git_branch = ?5, remote_path = ?6, include_paths = ?7, \
                     exclude_paths = ?8, prepare_commands = ?9, build_commands = ?10, \
                     restart_commands = ?11, health_checks = ?12, rollback_commands = ?13, \
                     secret_keys = ?14, backup_enabled = ?15, releases_enabled = ?16, \
                     delete_extras = ?17, step_timeout_secs = ?18, is_default = ?19, updated_at = ?20, \
                     test_commands = ?21 \
                     WHERE id = ?22",
                    rusqlite::params![
                        input.project_id,
                        input.name.trim(),
                        input.server_id,
                        input.strategy,
                        branch,
                        input.remote_path.trim(),
                        include,
                        exclude,
                        prepare,
                        build,
                        restart,
                        health,
                        rollback,
                        keys,
                        input.backup_enabled as i64,
                        input.releases_enabled as i64,
                        input.delete_extras as i64,
                        input.step_timeout_secs,
                        input.is_default as i64,
                        now,
                        tests,
                        id
                    ],
                )
                .map_err(|e| format!("falha ao salvar modelo de deploy: {e}"))?;
            if affected == 0 {
                return Err(format!("modelo de deploy #{id} não encontrado"));
            }
            get_deploy_profile(conn, id)
        }
        None => {
            conn.execute(
                "INSERT INTO deploy_profiles (project_id, name, server_id, strategy, git_branch, \
                 remote_path, include_paths, exclude_paths, prepare_commands, build_commands, \
                 restart_commands, health_checks, rollback_commands, secret_keys, backup_enabled, \
                 releases_enabled, delete_extras, step_timeout_secs, is_default, created_at, updated_at, test_commands) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?20, ?21)",
                    rusqlite::params![
                        input.project_id,
                        input.name.trim(),
                        input.server_id,
                        input.strategy,
                        branch,
                        input.remote_path.trim(),
                        include,
                        exclude,
                        prepare,
                        build,
                        restart,
                        health,
                        rollback,
                        keys,
                        input.backup_enabled as i64,
                        input.releases_enabled as i64,
                        input.delete_extras as i64,
                        input.step_timeout_secs,
                        input.is_default as i64,
                        now,
                        tests
                    ],
            )
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    format!("o projeto já tem um modelo de deploy chamado “{}”", input.name.trim())
                } else {
                    format!("falha ao criar modelo de deploy: {e}")
                }
            })?;
            get_deploy_profile(conn, conn.last_insert_rowid())
        }
    }
}

pub fn delete_deploy_profile(conn: &Connection, id: i64) -> Result<(), String> {
    let affected = conn
        .execute("DELETE FROM deploy_profiles WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover modelo de deploy: {e}"))?;
    if affected == 0 {
        return Err(format!("modelo de deploy #{id} não encontrado"));
    }
    Ok(())
}

/// Marca um modelo como padrão do projeto, desmarcando os demais.
pub fn set_default_deploy_profile(conn: &Connection, id: i64) -> Result<DeployProfile, String> {
    let profile = get_deploy_profile(conn, id)?;
    conn.execute(
        "UPDATE deploy_profiles SET is_default = 0 WHERE project_id = ?1",
        [profile.project_id],
    )
    .map_err(|e| format!("falha ao atualizar o modelo padrão: {e}"))?;
    conn.execute(
        "UPDATE deploy_profiles SET is_default = 1, updated_at = ?2 WHERE id = ?1",
        rusqlite::params![id, now_rfc3339()],
    )
    .map_err(|e| format!("falha ao atualizar o modelo padrão: {e}"))?;
    get_deploy_profile(conn, id)
}

/// Registra um comando do executor no histórico local. Segredos conhecidos são
/// mascarados antes da gravação — o histórico nunca guarda tokens ou senhas.
pub fn record_terminal_command(
    conn: &Connection,
    server_id: Option<i64>,
    server_name: Option<&str>,
    command: &str,
) -> Result<(), String> {
    let clean = crate::audit::scrub_secrets(command);
    conn.execute(
        "INSERT INTO terminal_history (ts, server_id, server_name, command) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![now_rfc3339(), server_id, server_name, clean],
    )
    .map_err(|e| format!("falha ao registrar comando no histórico: {e}"))?;
    Ok(())
}

pub fn list_terminal_history(conn: &Connection, limit: i64) -> Result<Vec<TerminalHistoryEntry>, String> {
    let mut stmt = conn
        .prepare("SELECT id, ts, server_id, server_name, command FROM terminal_history ORDER BY id DESC LIMIT ?1")
        .map_err(|e| format!("falha ao consultar histórico do terminal: {e}"))?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(TerminalHistoryEntry {
                id: r.get(0)?,
                ts: r.get(1)?,
                server_id: r.get(2)?,
                server_name: r.get(3)?,
                command: r.get(4)?,
            })
        })
        .map_err(|e| format!("falha ao consultar histórico do terminal: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler histórico: {e}"))?);
    }
    Ok(out)
}

pub fn clear_terminal_history(conn: &Connection) -> Result<usize, String> {
    conn.execute("DELETE FROM terminal_history", [])
        .map_err(|e| format!("falha ao limpar o histórico: {e}"))
}

/// Abre uma execução do pipeline e devolve o identificador.
pub fn insert_run(conn: &Connection, profile_id: i64) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO deploy_runs (profile_id, status, started_at) VALUES (?1, 'running', ?2)",
        rusqlite::params![profile_id, now_rfc3339()],
    )
    .map_err(|e| format!("falha ao registrar o início do deploy: {e}"))?;
    Ok(conn.last_insert_rowid())
}

/// Estado de uma etapa no instante em que ela é gravada.
pub struct StepUpdate<'a> {
    pub run_id: i64,
    pub position: i64,
    pub name: &'a str,
    pub status: &'a str,
    pub command: Option<&'a str>,
    pub exit_code: Option<i64>,
    pub message: Option<&'a str>,
}

/// Cria ou atualiza a linha de uma etapa. `command` nunca contém segredos: o
/// corpo dos scripts viaja pela stdin do ssh e não é gravado.
pub fn upsert_step(conn: &Connection, etapa: &StepUpdate) -> Result<(), String> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO deploy_run_steps (run_id, position, name, status, command, exit_code, message, started_at, finished_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?4 = 'running' THEN ?8 ELSE NULL END, \
           CASE WHEN ?4 IN ('done','failed','skipped') THEN ?8 ELSE NULL END) \
         ON CONFLICT(run_id, position) DO UPDATE SET \
           name = excluded.name, \
           status = excluded.status, \
           command = COALESCE(excluded.command, deploy_run_steps.command), \
           exit_code = COALESCE(excluded.exit_code, deploy_run_steps.exit_code), \
           message = COALESCE(excluded.message, deploy_run_steps.message), \
           started_at = COALESCE(deploy_run_steps.started_at, excluded.started_at), \
           finished_at = CASE WHEN excluded.status IN ('done','failed','skipped') \
             THEN excluded.finished_at ELSE deploy_run_steps.finished_at END",
        rusqlite::params![
            etapa.run_id,
            etapa.position,
            etapa.name,
            etapa.status,
            etapa.command,
            etapa.exit_code,
            etapa.message,
            now
        ],
    )
    .map_err(|e| format!("falha ao registrar etapa do deploy: {e}"))?;
    Ok(())
}

pub fn finish_run(
    conn: &Connection,
    run_id: i64,
    status: &str,
    release_dir: Option<&str>,
    previous_release: Option<&str>,
    detail: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE deploy_runs SET status = ?1, finished_at = ?2, release_dir = ?3, previous_release = ?4, detail = ?5 WHERE id = ?6",
        rusqlite::params![status, now_rfc3339(), release_dir, previous_release, detail, run_id],
    )
    .map_err(|e| format!("falha ao encerrar o registro do deploy: {e}"))?;
    Ok(())
}

fn row_to_step(row: &Row) -> rusqlite::Result<DeployRunStep> {
    Ok(DeployRunStep {
        position: row.get("position")?,
        name: row.get("name")?,
        status: row.get("status")?,
        command: row.get("command")?,
        exit_code: row.get("exit_code")?,
        message: row.get("message")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
    })
}

fn row_to_run(row: &Row) -> rusqlite::Result<DeployRun> {
    Ok(DeployRun {
        id: row.get("id")?,
        profile_id: row.get("profile_id")?,
        project_name: row.get("project_name")?,
        server_name: row.get("server_name")?,
        strategy: row.get("strategy")?,
        status: row.get("status")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        release_dir: row.get("release_dir")?,
        previous_release: row.get("previous_release")?,
        detail: row.get("detail")?,
        steps: Vec::new(),
    })
}

const RUN_SELECT: &str = "r.id, r.profile_id, pr.name AS project_name, s.name AS server_name, p.strategy, \
     r.status, r.started_at, r.finished_at, r.release_dir, r.previous_release, r.detail";

const RUN_FROM: &str = "FROM deploy_runs r \
     JOIN deploy_profiles p ON p.id = r.profile_id \
     JOIN projects pr ON pr.id = p.project_id \
     JOIN servers s ON s.id = p.server_id";

pub fn get_run(conn: &Connection, run_id: i64) -> Result<DeployRun, String> {
    let mut base = conn
        .query_row(
            &format!("SELECT {RUN_SELECT} {RUN_FROM} WHERE r.id = ?1"),
            [run_id],
            row_to_run,
        )
        .map_err(|e| format!("execução de deploy #{run_id} não encontrada: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT position, name, status, command, exit_code, message, started_at, finished_at \
             FROM deploy_run_steps WHERE run_id = ?1 ORDER BY position",
        )
        .map_err(|e| format!("falha ao consultar etapas do deploy: {e}"))?;
    let rows = stmt
        .query_map([run_id], row_to_step)
        .map_err(|e| format!("falha ao consultar etapas do deploy: {e}"))?;
    for r in rows {
        base.steps.push(r.map_err(|e| format!("falha ao ler etapa do deploy: {e}"))?);
    }
    Ok(base)
}

/// Histórico de deploys, sem as etapas (o detalhe vem em `get_run`).
pub fn list_runs(conn: &Connection, project_id: Option<i64>, limit: i64) -> Result<Vec<DeployRun>, String> {
    let sql = match project_id {
        Some(_) => format!(
            "SELECT {RUN_SELECT} {RUN_FROM} WHERE p.project_id = ?1 ORDER BY r.started_at DESC LIMIT ?2"
        ),
        None => format!("SELECT {RUN_SELECT} {RUN_FROM} ORDER BY r.started_at DESC LIMIT ?1"),
    };
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("falha ao consultar execuções de deploy: {e}"))?;
    let mut out = Vec::new();
    match project_id {
        Some(pid) => {
            let rows = stmt
                .query_map(rusqlite::params![pid, limit], row_to_run)
                .map_err(|e| format!("falha ao consultar execuções de deploy: {e}"))?;
            for r in rows {
                out.push(r.map_err(|e| format!("falha ao ler execução de deploy: {e}"))?);
            }
        }
        None => {
            let rows = stmt
                .query_map([limit], row_to_run)
                .map_err(|e| format!("falha ao consultar execuções de deploy: {e}"))?;
            for r in rows {
                out.push(r.map_err(|e| format!("falha ao ler execução de deploy: {e}"))?);
            }
        }
    }
    Ok(out)
}

pub fn setting_value(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .map_err(|e| format!("falha ao ler configuração: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().expect("abrir banco em memória");
        migrate(&conn).expect("migrações");
        conn
    }

    fn sample_input() -> ServerInput {
        ServerInput {
            name: "web-prod".into(),
            host: "203.0.113.10".into(),
            port: 22,
            username: "deploy".into(),
            auth_method: "key".into(),
            identity_file: Some("~/.ssh/id_ed25519".into()),
            ssh_alias: Some("web-prod".into()),
            environment: "prod".into(),
            group_name: None,
            notes: None,
        }
    }

    #[test]
    fn migra_e_faz_crud_de_servidor() {
        let conn = mem_db();
        assert!(sample_input().validate().is_ok());

        let server = insert_server(&conn, &sample_input(), "imported").unwrap();
        assert_eq!(server.name, "web-prod");
        assert_eq!(server.environment, "prod");
        assert!(server_exists(&conn, "web-prod", "203.0.113.10", "deploy", 22).unwrap());

        let mut input = sample_input();
        input.name = "web-prod-2".into();
        let updated = update_server_row(&conn, server.id, &input).unwrap();
        assert_eq!(updated.name, "web-prod-2");

        record_connection_status(&conn, server.id, true, "ok").unwrap();
        let re = get_server(&conn, server.id).unwrap();
        assert_eq!(re.last_status.as_deref(), Some("ok"));
        assert!(re.last_ok_at.is_some());

        record_connection_status(&conn, server.id, false, "falhou").unwrap();
        let re = get_server(&conn, server.id).unwrap();
        assert_eq!(re.last_status.as_deref(), Some("failed"));
        // last_ok_at preserva a data da última conexão bem-sucedida
        assert!(re.last_ok_at.is_some());

        delete_server_row(&conn, server.id).unwrap();
        assert!(get_server(&conn, server.id).is_err());
        assert!(!server_exists(&conn, "web-prod", "203.0.113.10", "deploy", 22).unwrap());
    }

    #[test]
    fn projeto_rejeita_servidor_padrao_inexistente() {
        let conn = mem_db();
        let p = insert_project(&conn, "site", "C:/projetos/site", true, Some("main"), None).unwrap();
        assert!(p.is_git_repo);

        assert!(update_project_row(&conn, p.id, "site", Some(999)).is_err());

        let updated = update_project_row(&conn, p.id, "site novo", None).unwrap();
        assert_eq!(updated.name, "site novo");
        assert_eq!(updated.default_server_id, None);

        assert!(insert_project(&conn, "outro", "C:/projetos/site", false, None, None).is_err());

        delete_project_row(&conn, p.id).unwrap();
        assert!(get_project(&conn, p.id).is_err());
    }

    fn seeded(conn: &Connection) -> (i64, i64) {
        let server = insert_server(conn, &sample_input(), "manual").unwrap();
        let project = insert_project(conn, "site", "C:/projetos/site", true, Some("main"), None).unwrap();
        (project.id, server.id)
    }

    fn profile_input(project_id: i64, server_id: i64) -> DeployProfileInput {
        DeployProfileInput {
            id: None,
            project_id,
            name: "produção".into(),
            server_id,
            strategy: "artifacts".into(),
            git_branch: Some(" main ".into()),
            remote_path: " /var/www/site ".into(),
            include_paths: vec!["dist".into(), ' '.to_string(), "public/favicon.ico".into()],
            exclude_paths: vec!["**/*.map".into()],
            prepare_commands: vec![],
            build_commands: vec!["npm ci".into(), "npm run build".into()],
            restart_commands: vec!["sudo systemctl reload caddy".into()],
            health_checks: vec!["curl -fsS http://127.0.0.1/healthz".into()],
            rollback_commands: vec![],
            test_commands: vec!["npm run test".into()],
            secret_keys: vec!["SENTRY_TOKEN".into()],
            backup_enabled: true,
            releases_enabled: true,
            delete_extras: false,
            step_timeout_secs: 900,
            is_default: true,
        }
    }

    #[test]
    fn modelo_de_deploy_guarda_configuracao_sem_valores_secretos() {
        let conn = mem_db();
        let (project_id, server_id) = seeded(&conn);

        let saved = upsert_deploy_profile(&conn, &profile_input(project_id, server_id)).unwrap();
        assert_eq!(saved.remote_path, "/var/www/site");
        assert_eq!(saved.git_branch.as_deref(), Some("main"));
        // itens vazios são descartados e os demais aparecem na ordem gravada
        assert_eq!(saved.include_paths, vec!["dist".to_string(), "public/favicon.ico".to_string()]);
        assert_eq!(saved.build_commands.len(), 2);
        assert_eq!(saved.step_timeout_secs, 900);
        assert!(saved.releases_enabled && saved.backup_enabled && !saved.delete_extras);
        assert_eq!(saved.server_name, "web-prod");
        assert_eq!(saved.server_environment, "prod");

        // somente os nomes das variáveis existem no esquema — não há coluna de valor
        let keys: String = conn
            .query_row("SELECT secret_keys FROM deploy_profiles WHERE id = ?1", [saved.id], |r| r.get(0))
            .unwrap();
        assert_eq!(keys, "[\"SENTRY_TOKEN\"]");

        let listed = list_deploy_profiles(&conn, Some(project_id)).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_default);

        let other = {
            let mut input = profile_input(project_id, server_id);
            input.name = "homologação".into();
            input.strategy = "git".into();
            input.is_default = false;
            upsert_deploy_profile(&conn, &input).unwrap()
        };
        // definir outro como padrão desmarca o anterior
        let novo = set_default_deploy_profile(&conn, other.id).unwrap();
        assert!(novo.is_default);
        assert!(!get_deploy_profile(&conn, saved.id).unwrap().is_default);

        let mut edit = profile_input(project_id, server_id);
        edit.id = Some(saved.id);
        edit.name = "produção".into();
        edit.step_timeout_secs = 300;
        let edited = upsert_deploy_profile(&conn, &edit).unwrap();
        assert_eq!(edited.step_timeout_secs, 300);
        assert_eq!(edited.id, saved.id);

        // apagar o modelo remove as execuções junto (checado na etapa do motor)
        delete_deploy_profile(&conn, other.id).unwrap();
        assert!(get_deploy_profile(&conn, other.id).is_err());
    }

    #[test]
    fn modelo_de_deploy_rejeita_entradas_perigosas() {
        let conn = mem_db();
        let (project_id, server_id) = seeded(&conn);

        let mut caminho_relativo = profile_input(project_id, server_id);
        caminho_relativo.remote_path = "var/www/site".into();
        assert!(upsert_deploy_profile(&conn, &caminho_relativo)
            .unwrap_err()
            .contains("caminho absoluto"));

        let mut com_quebra = profile_input(project_id, server_id);
        com_quebra.remote_path = "/var/www\nrm -rf /".into();
        assert!(upsert_deploy_profile(&conn, &com_quebra).is_err());

        let mut estrategia_falsa = profile_input(project_id, server_id);
        estrategia_falsa.strategy = "scp-rapido".into();
        assert!(upsert_deploy_profile(&conn, &estrategia_falsa)
            .unwrap_err()
            .contains("estratégia"));

        let mut sem_branch = profile_input(project_id, server_id);
        sem_branch.strategy = "git".into();
        sem_branch.git_branch = Some("   ".into());
        assert!(upsert_deploy_profile(&conn, &sem_branch)
            .unwrap_err()
            .contains("branch"));

        let branch_com_espaco = {
            let mut input = profile_input(project_id, server_id);
            input.strategy = "git".into();
            input.git_branch = Some("main; rm".into());
            input
        };
        assert!(upsert_deploy_profile(&conn, &branch_com_espaco).is_err());

        let mut chave_ruim = profile_input(project_id, server_id);
        chave_ruim.secret_keys = vec!["1TOKEN".into()];
        assert!(upsert_deploy_profile(&conn, &chave_ruim)
            .unwrap_err()
            .contains("variável"));

        let mut chave_repetida = profile_input(project_id, server_id);
        chave_repetida.secret_keys = vec!["TOKEN".into(), "TOKEN".into()];
        assert!(upsert_deploy_profile(&conn, &chave_repetida).is_err());

        let mut timeout_curto = profile_input(project_id, server_id);
        timeout_curto.step_timeout_secs = 1;
        assert!(upsert_deploy_profile(&conn, &timeout_curto)
            .unwrap_err()
            .contains("tempo limite"));

        let mut projeto_falso = profile_input(project_id, server_id);
        projeto_falso.project_id = 4242;
        assert!(upsert_deploy_profile(&conn, &projeto_falso)
            .unwrap_err()
            .contains("projeto"));

        let mut servidor_falso = profile_input(project_id, server_id);
        servidor_falso.server_id = 4242;
        assert!(upsert_deploy_profile(&conn, &servidor_falso)
            .unwrap_err()
            .contains("servidor"));

        let mut duplicado = profile_input(project_id, server_id);
        duplicado.name = "produção ".into();
        upsert_deploy_profile(&conn, &duplicado).unwrap();
        assert!(upsert_deploy_profile(&conn, &duplicado)
            .unwrap_err()
            .contains("já tem um modelo"));

        assert_eq!(list_deploy_profiles(&conn, None).unwrap().len(), 1);
    }

    #[test]
    fn auditoria_limpa_segredo_antes_de_gravar() {
        let conn = mem_db();
        crate::audit::log(
            &conn,
            "info",
            "app",
            "Teste de auditoria",
            None,
            None,
            None,
            Some("token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234 no log"),
        )
        .unwrap();
        let detail: String = conn
            .query_row("SELECT detail FROM audit_log LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(detail.contains("ghp_***"));
        assert!(!detail.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ1234"));
    }

    #[test]
    fn rotinas_padrao_e_crud() {
        let conn = mem_db();
        let routines = list_routines(&conn).unwrap();
        assert!(routines.len() >= 9, "esperava as rotinas padrão, veio {}", routines.len());
        assert!(routines.iter().all(|r| r.danger == "read"));
        assert!(routines.iter().all(|r| r.is_builtin));
        let tail = routines.iter().find(|r| r.command.contains("{{arquivo}}")).unwrap();
        assert_eq!(tail.params.len(), 1);
        assert_eq!(tail.params[0].name, "arquivo");

        let input = RoutineInput {
            id: None,
            name: "Reiniciar nginx".into(),
            description: Some("  Reinicia o serviço  ".into()),
            command: "systemctl restart {{servico}}".into(),
            danger: "write".into(),
            params: vec![RoutineParam {
                name: "servico".into(),
                label: Some("Serviço".into()),
                required: Some(true),
                default: Some("nginx".into()),
            }],
        };
        input.validate().unwrap();
        let created = insert_routine(&conn, &input).unwrap();
        assert_eq!(created.danger, "write");
        assert_eq!(created.description.as_deref(), Some("Reinicia o serviço"));
        assert!(!created.is_builtin);
        assert_eq!(created.params.len(), 1);

        let updated = update_routine_row(
            &conn,
            created.id,
            &RoutineInput {
                id: Some(created.id),
                name: "Reiniciar serviço".into(),
                description: None,
                command: "systemctl restart {{servico}}".into(),
                danger: "write".into(),
                params: created.params.clone(),
            },
        )
        .unwrap();
        assert_eq!(updated.name, "Reiniciar serviço");
        assert_eq!(updated.description, None);

        // nome duplicado é rejeitado (usa o nome atual da rotina renomeada)
        let duplicado = RoutineInput { name: "Reiniciar serviço".into(), ..input };
        assert!(insert_routine(&conn, &duplicado).is_err());

        delete_routine_row(&conn, created.id).unwrap();
        assert!(get_routine(&conn, created.id).is_err());
    }

    #[test]
    fn rotina_rejeita_dados_invalidos() {
        let input = RoutineInput {
            id: None,
            name: "x".into(),
            description: None,
            command: "echo {{a}}".into(),
            danger: "admin".into(),
            params: vec![],
        };
        assert!(input.validate().is_err()); // classificação inválida

        let mut ok_input = RoutineInput { danger: "read".into(), ..input };
        assert!(ok_input.validate().is_err()); // {{a}} usado sem parâmetro definido

        ok_input.params = vec![RoutineParam { name: "a b".into(), label: None, required: None, default: None }];
        assert!(ok_input.validate().is_err()); // nome de parâmetro com espaço

        ok_input.params = vec![RoutineParam { name: "a".into(), label: None, required: None, default: None }];
        ok_input.validate().unwrap();
    }

    #[test]
    fn historico_do_terminal_limpa_segredos() {
        let conn = mem_db();
        record_terminal_command(
            &conn,
            Some(1),
            Some("web-prod"),
            "curl -H 'Authorization: token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234' https://api.exemplo",
        )
        .unwrap();
        let entries = list_terminal_history(&conn, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].server_name.as_deref(), Some("web-prod"));
        assert!(entries[0].command.contains("ghp_***"));
        assert!(!entries[0].command.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ1234"));

        assert_eq!(clear_terminal_history(&conn).unwrap(), 1);
        assert!(list_terminal_history(&conn, 10).unwrap().is_empty());

        // preferência ausente → quem decide o padrão é a camada de comandos
        assert_eq!(setting_value(&conn, "terminal_history_enabled").unwrap(), None);
    }
}
