use std::{path::PathBuf, sync::Arc, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, Row};
use tauri::Manager;

use crate::models::{
    AuditEntry, DefinitionRevision, DeployProfile, DeployProfileInput, DeployRun, DeployRunStep,
    PipelineNode, PipelineNodeInput, Project, Routine, RoutineInput, RoutineParam, Server,
    ServerInput, TerminalHistoryEntry,
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
    -- Único porque `upsert_step` grava com ON CONFLICT(run_id, position): sem um
    -- índice único nessas colunas o SQLite recusa a instrução inteira.
    CREATE UNIQUE INDEX IF NOT EXISTS idx_run_steps_run ON deploy_run_steps(run_id, position);
    CREATE INDEX IF NOT EXISTS idx_runs_started ON deploy_runs(started_at DESC);
    "#,
    // v4 — testes automatizados da etapa 6 do pipeline
    r#"
    ALTER TABLE deploy_profiles ADD COLUMN test_commands TEXT NOT NULL DEFAULT '[]';
    "#,
    // v5 — modelo de deploy sem servidor: o destino pode ser esta própria máquina.
    // SQLite não tira um NOT NULL de coluna existente, então a tabela é reconstruída.
    // O `DROP TABLE` do meio é o perigo real: com as chaves estrangeiras aplicadas
    // (o SQLite embutido vem com SQLITE_DEFAULT_FOREIGN_KEYS=1), apagar
    // deploy_profiles apagaria em cascata todo o histórico de deploy do usuário.
    // O procedimento documentado para reconstruir tabela-mãe é desligar a checagem
    // durante o troca-e-troca e ligar de novo no fim. A v5 também conserta o índice
    // dos passos, que impedia qualquer etapa de ser registrada.
    r#"
    PRAGMA foreign_keys=OFF;
    CREATE TABLE deploy_profiles_new (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        server_id INTEGER REFERENCES servers(id) ON DELETE CASCADE,
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
        test_commands TEXT NOT NULL DEFAULT '[]',
        UNIQUE (project_id, name)
    );
    INSERT INTO deploy_profiles_new (id, project_id, name, server_id, strategy, git_branch, remote_path,
        include_paths, exclude_paths, prepare_commands, build_commands, restart_commands, health_checks,
        rollback_commands, secret_keys, backup_enabled, releases_enabled, delete_extras, step_timeout_secs,
        is_default, created_at, updated_at, test_commands)
    SELECT id, project_id, name, server_id, strategy, git_branch, remote_path,
        include_paths, exclude_paths, prepare_commands, build_commands, restart_commands, health_checks,
        rollback_commands, secret_keys, backup_enabled, releases_enabled, delete_extras, step_timeout_secs,
        is_default, created_at, updated_at, test_commands FROM deploy_profiles;
    DROP TABLE deploy_profiles;
    ALTER TABLE deploy_profiles_new RENAME TO deploy_profiles;
    CREATE INDEX IF NOT EXISTS idx_profiles_project ON deploy_profiles(project_id);
    -- A v3 criava o índice das etapas sem ser único, e `upsert_step` grava com
    -- ON CONFLICT(run_id, position): sem índice único nessas colunas o SQLite recusa
    -- a instrução e nenhuma etapa era registrada no histórico. Aqui o índice é trocado.
    DELETE FROM deploy_run_steps WHERE id NOT IN (SELECT MIN(id) FROM deploy_run_steps GROUP BY run_id, position);
    DROP INDEX IF EXISTS idx_run_steps_run;
    CREATE UNIQUE INDEX IF NOT EXISTS idx_run_steps_run ON deploy_run_steps(run_id, position);
    PRAGMA foreign_keys=ON;
    "#,
    // v6 — contrato central de pipeline (plano CI/CD, etapa 2). A definição já
    // existia como modelo de deploy; o que falta é ela ter **versão**, cada alteração
    // deixar um retrato do que era, e cada execução registrar de onde saiu: versão da
    // definição, destino, ambiente e commit de origem. Sem tabela nova de definição —
    // `deploy_profiles` já é a definição, e duplicá-la criaria o segundo motor que o
    // plano proíbe.
    r#"
    ALTER TABLE deploy_profiles ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
    -- IF NOT EXISTS: quem recria o esquema antigo por cima (é o que o teste de
    -- migração faz) continua com a tabela da v6 pronta, e a migração precisa passar
    -- de novo sem trombizar com ela.
    CREATE TABLE IF NOT EXISTS pipeline_revisions (
        profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
        version INTEGER NOT NULL,
        spec TEXT NOT NULL,
        saved_at TEXT NOT NULL,
        PRIMARY KEY (profile_id, version)
    );
    ALTER TABLE deploy_runs ADD COLUMN definition_version INTEGER;
    ALTER TABLE deploy_runs ADD COLUMN destination TEXT;
    ALTER TABLE deploy_runs ADD COLUMN environment TEXT;
    ALTER TABLE deploy_runs ADD COLUMN commit_sha TEXT;
    "#,
    // v7 — construtor visual do pipeline (plano CI/CD, etapa 3). O grafo é parte da
    // definição, então os nós pertencem ao modelo de deploy e a nenhuma tabela nova
    // de "pipeline": uma definição, um validador, um motor. `toca_destino` não é
    // escolha de quem desenha — é derivado do tipo do nó na gravação, porque é o que
    // decide se uma execução pode ser repetida sem conferir o destino. As duas
    // colunas em `deploy_run_steps` amarram cada linha do histórico ao nó que a
    // produziu; numa execução linear elas ficam vazias, como sempre estiveram.
    r#"
    CREATE TABLE IF NOT EXISTS pipeline_nodes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
        node_key TEXT NOT NULL,
        title TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('motor','comando','aprovacao')),
        engine_step INTEGER,
        executor TEXT NOT NULL DEFAULT 'local' CHECK (executor IN ('local','ssh')),
        environment TEXT,
        working_dir TEXT,
        commands TEXT NOT NULL DEFAULT '[]',
        depends_on TEXT NOT NULL DEFAULT '[]',
        parallel INTEGER NOT NULL DEFAULT 0,
        timeout_secs INTEGER NOT NULL DEFAULT 0,
        retry_limit INTEGER NOT NULL DEFAULT 0,
        condition TEXT,
        requires_approval INTEGER NOT NULL DEFAULT 0,
        touches_destination INTEGER NOT NULL DEFAULT 0,
        x INTEGER NOT NULL DEFAULT 0,
        y INTEGER NOT NULL DEFAULT 0,
        UNIQUE (profile_id, node_key)
    );
    CREATE INDEX IF NOT EXISTS idx_pipeline_nodes_profile ON pipeline_nodes(profile_id, x, y);
    ALTER TABLE deploy_run_steps ADD COLUMN node_key TEXT;
    ALTER TABLE deploy_run_steps ADD COLUMN touched_destination INTEGER NOT NULL DEFAULT 0;
    "#,
    // v8 — executores de runtime (plano CI/CD, etapa 4). As colunas do modelo entram
    // por `ALTER`: são campos que já têm um único dono (`runtime::Alvo::conferir`,
    // chamado pela tela antes de salvar e pelo motor antes de tocar no servidor), e
    // `deploy_profiles` agora tem três filhas com cascata — reconstruir a tabela-mãe
    // por um `CHECK` que o Rust já impõe seria risco sem ganho. `pipeline_nodes` é
    // reconstruída porque não há ALTER que alargue um `CHECK` inline: sem isso o nó
    // `git` não teria como existir no esquema, e um tipo que a tela desenha mas o
    // banco recusa é exatamente o botão enfeitado que a especificação proíbe.
    r#"
    ALTER TABLE deploy_profiles ADD COLUMN runtime TEXT NOT NULL DEFAULT '';
    ALTER TABLE deploy_profiles ADD COLUMN image_name TEXT;
    ALTER TABLE deploy_profiles ADD COLUMN dockerfile TEXT;
    ALTER TABLE deploy_profiles ADD COLUMN compose_file TEXT;
    ALTER TABLE deploy_profiles ADD COLUMN service_name TEXT;
    ALTER TABLE deploy_profiles ADD COLUMN publish_ports TEXT NOT NULL DEFAULT '[]';
    PRAGMA foreign_keys=OFF;
    CREATE TABLE pipeline_nodes_new (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
        node_key TEXT NOT NULL,
        title TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('motor','comando','aprovacao','git')),
        engine_step INTEGER,
        executor TEXT NOT NULL DEFAULT 'local' CHECK (executor IN ('local','ssh')),
        environment TEXT,
        working_dir TEXT,
        commands TEXT NOT NULL DEFAULT '[]',
        depends_on TEXT NOT NULL DEFAULT '[]',
        parallel INTEGER NOT NULL DEFAULT 0,
        timeout_secs INTEGER NOT NULL DEFAULT 0,
        retry_limit INTEGER NOT NULL DEFAULT 0,
        condition TEXT,
        requires_approval INTEGER NOT NULL DEFAULT 0,
        touches_destination INTEGER NOT NULL DEFAULT 0,
        git_op TEXT CHECK (git_op IN ('pull','checkout','tag')),
        git_ref TEXT,
        x INTEGER NOT NULL DEFAULT 0,
        y INTEGER NOT NULL DEFAULT 0,
        UNIQUE (profile_id, node_key)
    );
    INSERT INTO pipeline_nodes_new (id, profile_id, node_key, title, kind, engine_step, executor,
        environment, working_dir, commands, depends_on, parallel, timeout_secs, retry_limit,
        condition, requires_approval, touches_destination, x, y)
    SELECT id, profile_id, node_key, title, kind, engine_step, executor,
        environment, working_dir, commands, depends_on, parallel, timeout_secs, retry_limit,
        condition, requires_approval, touches_destination, x, y FROM pipeline_nodes;
    DROP TABLE pipeline_nodes;
    ALTER TABLE pipeline_nodes_new RENAME TO pipeline_nodes;
    -- O índice acompanha a tabela: DROP leva o índice embora com ela.
    CREATE INDEX IF NOT EXISTS idx_pipeline_nodes_profile ON pipeline_nodes(profile_id, x, y);
    PRAGMA foreign_keys=ON;
    "#,
    // v9 — a imagem que estava no ar antes de uma corrida com executor Docker. A
    // reversão de um container não tem symlink para trocar: o que precisa voltar ao
    // lugar é a imagem, e ela só pode vir de onde foi lida — o `ANTERIOR:` que o
    // próprio roteiro imprimiu durante a corrida. Guardada numa coluna própria em vez
    // de `previous_release` porque as duas coisas voltam juntas numa publicação com
    // releases: um caminho e um digest, cada um com o seu mecanismo de retorno.
    r#"
    ALTER TABLE deploy_runs ADD COLUMN previous_image TEXT;
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
    // O SQLite embutido já vem com SQLITE_DEFAULT_FOREIGN_KEYS=1, mas as cascatas do
    // esquema são parte da consistência dos dados: a promessa é declarada aqui, e não
    // deixada nas mãos de como a lib foi compilada.
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("falha ao ativar as chaves estrangeiras: {e}"))?;
    migrate(&conn)?;
    Ok(conn)
}

pub(crate) fn migrate(conn: &Connection) -> Result<(), String> {
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
    if version < MIGRATIONS.len() as i64 {
        retrato_inicial_das_definicoes(conn)?;
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

/// Remove modelos e o que foi registrado por eles. As chaves estrangeiras do
/// esquema são aplicadas — o SQLite embutido vem compilado com
/// `SQLITE_DEFAULT_FOREIGN_KEYS=1`, então o `ON DELETE CASCADE` já funcionaria
/// sozinho. A passada explícita existe por dois motivos: contar quantos modelos
/// saíram (a auditoria avisa isso na interface) e continuar certa caso o banco
/// seja aberto por uma build sem essa flag. Os ids vêm de colunas
/// `INTEGER PRIMARY KEY`, nunca de texto do usuário.
fn apagar_modelos_e_execucoes(conn: &Connection, modelos: &[i64]) -> Result<usize, String> {
    if modelos.is_empty() {
        return Ok(0);
    }
    let lista = modelos
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    conn.execute(
        &format!("DELETE FROM deploy_run_steps WHERE run_id IN (SELECT id FROM deploy_runs WHERE profile_id IN ({lista}))"),
        [],
    )
    .map_err(|e| format!("falha ao remover as etapas dos deploys: {e}"))?;
    conn.execute(
        &format!("DELETE FROM deploy_runs WHERE profile_id IN ({lista})"),
        [],
    )
    .map_err(|e| format!("falha ao remover os deploys registrados: {e}"))?;
    let afetados = conn
        .execute(&format!("DELETE FROM deploy_profiles WHERE id IN ({lista})"), [])
        .map_err(|e| format!("falha ao remover os modelos de deploy: {e}"))?;
    Ok(afetados)
}

/// Ids dos modelos ligados a um projeto ou a um servidor, conforme a coluna pedida.
fn ids_de_modelos(conn: &Connection, coluna: &str, id: i64) -> Result<Vec<i64>, String> {
    let sql = format!("SELECT id FROM deploy_profiles WHERE {coluna} = ?1");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("falha ao listar os modelos de deploy: {e}"))?;
    let rows = stmt
        .query_map([id], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("falha ao listar os modelos de deploy: {e}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("falha ao listar os modelos de deploy: {e}"))
}

/// Remove um servidor junto com os modelos que publicavam nele: um modelo sem
/// servidor é outra coisa (publica nesta máquina), então ele não pode ficar
/// apontando para um cadastro que já era.
pub fn delete_server_row(conn: &Connection, id: i64) -> Result<usize, String> {
    let modelos = ids_de_modelos(conn, "server_id", id)?;
    let removidos = apagar_modelos_e_execucoes(conn, &modelos)?;
    let afetados = conn
        .execute("DELETE FROM servers WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover servidor: {e}"))?;
    if afetados == 0 {
        return Err(format!("servidor #{id} não encontrado"));
    }
    Ok(removidos)
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

pub fn delete_project_row(conn: &Connection, id: i64) -> Result<usize, String> {
    let modelos = ids_de_modelos(conn, "project_id", id)?;
    let removidos = apagar_modelos_e_execucoes(conn, &modelos)?;
    let affected = conn
        .execute("DELETE FROM projects WHERE id = ?1", [id])
        .map_err(|e| format!("falha ao remover projeto: {e}"))?;
    if affected == 0 {
        return Err(format!("projeto #{id} não encontrado"));
    }
    Ok(removidos)
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
     p.step_timeout_secs, p.is_default, p.created_at, p.updated_at, p.test_commands, p.version, \
     p.runtime, p.image_name, p.dockerfile, p.compose_file, p.service_name, p.publish_ports";

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
        version: row.get("version")?,
        runtime: row.get("runtime")?,
        image_name: row.get("image_name")?,
        dockerfile: row.get("dockerfile")?,
        compose_file: row.get("compose_file")?,
        service_name: row.get("service_name")?,
        publish_ports: json_list(row.get::<_, String>("publish_ports")?.as_str()),
    })
}

/// O que o executor escolhido realmente lê. Guardar um Dockerfile numa definição
/// PM2 faria o retrato da versão afirmar que a execução tem algo que ela não tem.
struct ExecutorGuardado {
    runtime: String,
    imagem: Option<String>,
    dockerfile: Option<String>,
    compose_file: Option<String>,
    servico: Option<String>,
    portas: Vec<String>,
}

fn guardar_executor(input: &DeployProfileInput) -> ExecutorGuardado {
    let runtime = input
        .runtime
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (imagem, dockerfile, compose_file, servico) = match runtime.as_str() {
        "docker" => (
            clean_optional(&input.image_name),
            clean_optional(&input.dockerfile),
            None,
            clean_optional(&input.service_name),
        ),
        "compose" => (
            None,
            None,
            clean_optional(&input.compose_file),
            clean_optional(&input.service_name),
        ),
        "pm2" => (None, None, None, clean_optional(&input.service_name)),
        _ => (None, None, None, None),
    };
    let portas = match runtime.as_str() {
        "docker" => input.publish_ports.clone().unwrap_or_default(),
        _ => Vec::new(),
    };
    ExecutorGuardado {
        runtime,
        imagem,
        dockerfile,
        compose_file,
        servico,
        portas,
    }
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
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p LEFT JOIN servers s ON s.id = p.server_id \
             WHERE p.project_id = ?1 ORDER BY p.is_default DESC, p.name"
        ),
        None => format!(
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p LEFT JOIN servers s ON s.id = p.server_id \
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
            "SELECT {PROFILE_COLUMNS} FROM deploy_profiles p LEFT JOIN servers s ON s.id = p.server_id WHERE p.id = ?1"
        ),
        [id],
        row_to_profile,
    )
    .map_err(|e| format!("modelo de deploy #{id} não encontrado: {e}"))
}

/// O retrato JSON do que a definição era naquela versão. São os campos que o motor
/// lê — comandos, caminhos e limites — **e o grafo**, porque num pipeline visual a
/// ordem das etapas é parte da definição: sem ela a versão não diz o que foi
/// executado. Os **valores** das variáveis nunca aparecem aqui: a definição guarda
/// só os nomes, e o valor continua sendo digitado a cada execução.
fn spec_da_definicao(p: &DeployProfile, nodes: &[PipelineNode]) -> String {
    let mapa = serde_json::json!({
        "nome": p.name,
        "servidorId": p.server_id,
        "estrategia": p.strategy,
        "ramo": p.git_branch,
        "destino": p.remote_path,
        "inclusao": p.include_paths,
        "exclusao": p.exclude_paths,
        "preparacao": p.prepare_commands,
        "build": p.build_commands,
        "testes": p.test_commands,
        "atualizacao": p.restart_commands,
        "saude": p.health_checks,
        "rollback": p.rollback_commands,
        "variaveis": p.secret_keys,
        "backup": p.backup_enabled,
        "releases": p.releases_enabled,
        "apagarExcedentes": p.delete_extras,
        "prazoPorEtapaSegundos": p.step_timeout_secs,
        "executor": p.runtime,
        "imagem": p.image_name,
        "dockerfile": p.dockerfile,
        "arquivoCompose": p.compose_file,
        "servico": p.service_name,
        "portas": p.publish_ports,
        "nos": nodes,
    });
    serde_json::to_string(&mapa).unwrap_or_else(|_| "{}".to_string())
}

/// Grava a versão atual da definição como um retrato. `ON CONFLICT DO UPDATE` porque
/// a migração v6 semeia a versão 1 com o que já estava salvo, e o primeiro salvamento
/// depois disso reescreve a mesma linha com o mesmo número.
fn gravar_revisao(conn: &Connection, p: &DeployProfile) -> Result<(), String> {
    let nos = list_pipeline_nodes(conn, p.id).unwrap_or_default();
    conn.execute(
        "INSERT INTO pipeline_revisions (profile_id, version, spec, saved_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(profile_id, version) DO UPDATE SET spec = excluded.spec, saved_at = excluded.saved_at",
        rusqlite::params![p.id, p.version, spec_da_definicao(p, &nos), p.updated_at],
    )
    .map_err(|e| format!("falha ao registrar a versão da definição: {e}"))?;
    Ok(())
}

/// As versões salvas de uma definição, da mais nova para a mais antiga.
pub fn list_revisions(conn: &Connection, profile_id: i64) -> Result<Vec<DefinitionRevision>, String> {
    let mut stmt = conn
        .prepare("SELECT version, spec, saved_at FROM pipeline_revisions WHERE profile_id = ?1 ORDER BY version DESC")
        .map_err(|e| format!("falha ao preparar o histórico da definição: {e}"))?;
    let rows = stmt
        .query_map([profile_id], |row| {
            let spec: String = row.get("spec")?;
            Ok(DefinitionRevision {
                version: row.get("version")?,
                saved_at: row.get("saved_at")?,
                spec: serde_json::from_str(&spec).unwrap_or(serde_json::json!({})),
            })
        })
        .map_err(|e| format!("falha ao ler o histórico da definição: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler o histórico da definição: {e}"))?);
    }
    Ok(out)
}

/// Retrato inicial das definições que já existiam, na virada para a v6: sem isso a
/// primeira execução depois da migração apontaria para uma versão que nunca foi
/// registrada. Roda uma vez por definição, e nunca cobre um retrato já gravado.
fn retrato_inicial_das_definicoes(conn: &Connection) -> Result<(), String> {
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT p.id FROM deploy_profiles p \
                 WHERE NOT EXISTS (SELECT 1 FROM pipeline_revisions r WHERE r.profile_id = p.id)",
            )
            .map_err(|e| format!("falha ao procurar definições sem retrato: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| format!("falha ao procurar definições sem retrato: {e}"))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    for id in ids {
        let perfil = get_deploy_profile(conn, id)?;
        gravar_revisao(conn, &perfil)?;
    }
    Ok(())
}

pub fn upsert_deploy_profile(
    conn: &Connection,
    input: &DeployProfileInput,
) -> Result<DeployProfile, String> {
    input.validate()?;
    exists(conn, "projects", input.project_id, "projeto")?;
    if let Some(server_id) = input.server_id {
        exists(conn, "servers", server_id, "servidor")?;
    }

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
    let exec = guardar_executor(input);
    let portas = to_json_list(&exec.portas)?;
    let (runtime, imagem, dockerfile, compose_file, servico) = (
        exec.runtime,
        exec.imagem,
        exec.dockerfile,
        exec.compose_file,
        exec.servico,
    );

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
                     test_commands = ?21, runtime = ?22, image_name = ?23, dockerfile = ?24, \
                     compose_file = ?25, service_name = ?26, publish_ports = ?27, version = version + 1 \
                     WHERE id = ?28",
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
                        runtime,
                        imagem,
                        dockerfile,
                        compose_file,
                        servico,
                        portas,
                        id
                    ],
                )
                .map_err(|e| format!("falha ao salvar modelo de deploy: {e}"))?;
            if affected == 0 {
                return Err(format!("modelo de deploy #{id} não encontrado"));
            }
            let salvo = get_deploy_profile(conn, id)?;
            gravar_revisao(conn, &salvo)?;
            Ok(salvo)
        }
        None => {
            conn.execute(
                "INSERT INTO deploy_profiles (project_id, name, server_id, strategy, git_branch, \
                 remote_path, include_paths, exclude_paths, prepare_commands, build_commands, \
                 restart_commands, health_checks, rollback_commands, secret_keys, backup_enabled, \
                 releases_enabled, delete_extras, step_timeout_secs, is_default, created_at, updated_at, test_commands, \
                 runtime, image_name, dockerfile, compose_file, service_name, publish_ports) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
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
                        runtime,
                        imagem,
                        dockerfile,
                        compose_file,
                        servico,
                        portas
                    ],
            )
            .map_err(|e| {
                if e.to_string().contains("UNIQUE") {
                    format!("o projeto já tem um modelo de deploy chamado “{}”", input.name.trim())
                } else {
                    format!("falha ao criar modelo de deploy: {e}")
                }
            })?;
            let criado = get_deploy_profile(conn, conn.last_insert_rowid())?;
            gravar_revisao(conn, &criado)?;
            Ok(criado)
        }
    }
}

pub fn delete_deploy_profile(conn: &Connection, id: i64) -> Result<(), String> {
    let removidos = apagar_modelos_e_execucoes(conn, &[id])?;
    if removidos == 0 {
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

// ---------- nós do grafo (plano CI/CD, etapa 3) ----------

const NODE_COLUMNS: &str = "id, profile_id, node_key, title, kind, engine_step, executor, \
     environment, working_dir, commands, git_op, git_ref, depends_on, parallel, timeout_secs, \
     retry_limit, condition, requires_approval, touches_destination, x, y";

fn row_to_node(row: &Row) -> rusqlite::Result<PipelineNode> {
    let commands: String = row.get("commands")?;
    let depends: String = row.get("depends_on")?;
    Ok(PipelineNode {
        id: row.get("id")?,
        profile_id: row.get("profile_id")?,
        node_key: row.get("node_key")?,
        title: row.get("title")?,
        kind: row.get("kind")?,
        engine_step: row.get("engine_step")?,
        executor: row.get("executor")?,
        environment: row.get("environment")?,
        working_dir: row.get("working_dir")?,
        commands: json_list(&commands),
        git_op: row.get("git_op")?,
        git_ref: row.get("git_ref")?,
        depends_on: json_list(&depends),
        parallel: row.get("parallel")?,
        timeout_secs: row.get("timeout_secs")?,
        retry_limit: row.get("retry_limit")?,
        condition: row.get("condition")?,
        requires_approval: row.get("requires_approval")?,
        touches_destination: row.get("touches_destination")?,
        x: row.get("x")?,
        y: row.get("y")?,
    })
}

/// Os nós salvos de uma definição, na ordem em que aparecem na tela. Lista vazia
/// significa uma coisa só: a definição roda o pipeline linear de quinze etapas que
/// já existia — não há grafo "escondido" nem default inventado.
pub fn list_pipeline_nodes(conn: &Connection, profile_id: i64) -> Result<Vec<PipelineNode>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {NODE_COLUMNS} FROM pipeline_nodes WHERE profile_id = ?1 ORDER BY x, y, node_key"
        ))
        .map_err(|e| format!("falha ao preparar o grafo do pipeline: {e}"))?;
    let rows = stmt
        .query_map([profile_id], row_to_node)
        .map_err(|e| format!("falha ao ler o grafo do pipeline: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("falha ao ler o grafo do pipeline: {e}"))?);
    }
    Ok(out)
}

/// Grava o grafo de uma definição: substitui os nós e sobe a versão da definição,
/// porque o grafo **é** a definição. A conferência da forma do grafo vem antes, em
/// `pipeline::checar_grafo`; aqui entra o que ela já aceitou.
pub fn replace_pipeline_nodes(
    conn: &Connection,
    profile_id: i64,
    nodes: &[PipelineNodeInput],
) -> Result<Vec<PipelineNode>, String> {
    // A definição precisa existir: sem isso um identificador errado criaria nós
    // órfãos que o SQLite recusaria, e a mensagem dele não diz o que falta.
    get_deploy_profile(conn, profile_id)?;
    conn.execute(
        "DELETE FROM pipeline_nodes WHERE profile_id = ?1",
        [profile_id],
    )
    .map_err(|e| format!("falha ao limpar o grafo do pipeline: {e}"))?;
    for node in nodes {
        let executor = match node.kind().as_str() {
            "comando" => node.executor(),
            _ => "local".to_string(),
        };
        // Só o nó `git` declara operação e referência: o que sobrou de um tipo que
        // mudou na tela não pode continuar gravado como se fizesse parte da definição.
        let (git_op, git_ref) = match node.kind().as_str() {
            "git" => (Some(node.git_op()), clean_optional(&node.git_ref)),
            _ => (None, None),
        };
        conn.execute(
            "INSERT INTO pipeline_nodes (profile_id, node_key, title, kind, engine_step, executor, \
             environment, working_dir, commands, git_op, git_ref, depends_on, parallel, timeout_secs, \
             retry_limit, condition, requires_approval, touches_destination, x, y) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
            rusqlite::params![
                profile_id,
                node.node_key.trim(),
                node.title.trim(),
                node.kind(),
                node.engine_step,
                executor,
                clean_optional(&node.environment),
                clean_optional(&node.working_dir),
                to_json_list(&node.commands())?,
                git_op,
                git_ref,
                to_json_list(&node.depends_on())?,
                node.parallel.unwrap_or(false) as i64,
                node.timeout_secs.unwrap_or(0),
                node.retry_limit.unwrap_or(0),
                clean_optional(&node.condition),
                node.requires_approval.unwrap_or(false) as i64,
                node.touches_destination() as i64,
                node.x.unwrap_or(0),
                node.y.unwrap_or(0),
            ],
        )
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                format!("o grafo tem dois nós com o mesmo identificador “{}”", node.node_key.trim())
            } else {
                format!("falha ao gravar o nó “{}”: {e}", node.node_key.trim())
            }
        })?;
    }
    conn.execute(
        "UPDATE deploy_profiles SET version = version + 1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now_rfc3339(), profile_id],
    )
    .map_err(|e| format!("falha ao versionar a definição: {e}"))?;
    let salvo = get_deploy_profile(conn, profile_id)?;
    gravar_revisao(conn, &salvo)?;
    list_pipeline_nodes(conn, profile_id)
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

/// De onde uma execução saiu. É copiado da definição no instante em que a execução
/// abre, porque a definição pode mudar depois e o histórico não pode mudar junto.
pub struct Procedencia {
    pub definition_version: i64,
    pub destination: String,
    pub environment: String,
    pub commit_sha: Option<String>,
}

/// Abre uma execução do pipeline e devolve o identificador.
pub fn insert_run(conn: &Connection, profile_id: i64, proc: &Procedencia) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO deploy_runs (profile_id, status, started_at, definition_version, destination, \
         environment, commit_sha) VALUES (?1, 'running', ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            profile_id,
            now_rfc3339(),
            proc.definition_version,
            proc.destination,
            proc.environment,
            proc.commit_sha
        ],
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
    /// O nó do grafo que produziu esta linha; `None` no pipeline linear.
    pub node_key: Option<&'a str>,
    /// Se a etapa mexe no destino. É o que autoriza (ou não) repetir a execução
    /// sem conferir o que aconteceu lá fora.
    pub touched_destination: bool,
}

/// Cria ou atualiza a linha de uma etapa. `command` nunca contém segredos: o
/// corpo dos scripts viaja pela stdin do ssh e não é gravado.
pub fn upsert_step(conn: &Connection, etapa: &StepUpdate) -> Result<(), String> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO deploy_run_steps (run_id, position, name, status, command, exit_code, message, \
           started_at, finished_at, node_key, touched_destination) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?4 = 'running' THEN ?8 ELSE NULL END, \
           CASE WHEN ?4 IN ('done','failed','skipped') THEN ?8 ELSE NULL END, ?9, ?10) \
         ON CONFLICT(run_id, position) DO UPDATE SET \
           name = excluded.name, \
           status = excluded.status, \
           command = COALESCE(excluded.command, deploy_run_steps.command), \
           exit_code = COALESCE(excluded.exit_code, deploy_run_steps.exit_code), \
           message = CASE WHEN excluded.status IN ('done','failed','skipped') \
             THEN excluded.message ELSE COALESCE(excluded.message, deploy_run_steps.message) END, \
           started_at = COALESCE(deploy_run_steps.started_at, excluded.started_at), \
           touched_destination = MAX(deploy_run_steps.touched_destination, excluded.touched_destination), \
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
            now,
            etapa.node_key,
            etapa.touched_destination as i64,
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
    previous_image: Option<&str>,
    detail: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE deploy_runs SET status = ?1, finished_at = ?2, release_dir = ?3, previous_release = ?4, previous_image = ?5, detail = ?6 WHERE id = ?7",
        rusqlite::params![status, now_rfc3339(), release_dir, previous_release, previous_image, detail, run_id],
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
        node_key: row.get("node_key")?,
        touched_destination: row.get("touched_destination")?,
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
        previous_image: row.get("previous_image")?,
        detail: row.get("detail")?,
        definition_version: row.get("definition_version")?,
        destination: row.get("destination")?,
        environment: row.get("environment")?,
        commit_sha: row.get("commit_sha")?,
        steps: Vec::new(),
    })
}

const RUN_SELECT: &str = "r.id, r.profile_id, pr.name AS project_name, s.name AS server_name, p.strategy, \
     r.status, r.started_at, r.finished_at, r.release_dir, r.previous_release, r.previous_image, r.detail, \
     r.definition_version, r.destination, r.environment, r.commit_sha";

const RUN_FROM: &str = "FROM deploy_runs r \
     JOIN deploy_profiles p ON p.id = r.profile_id \
     JOIN projects pr ON pr.id = p.project_id \
     LEFT JOIN servers s ON s.id = p.server_id";

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
            "SELECT position, name, status, command, exit_code, message, started_at, finished_at, \
             node_key, touched_destination \
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

    /// Procedência de teste: o que a execução registra de onde saiu.
    fn proc_de_teste(destino: &str) -> Procedencia {
        Procedencia {
            definition_version: 1,
            destination: destino.to_string(),
            environment: "local".into(),
            commit_sha: None,
        }
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
        profile_input_no_servidor(project_id, Some(server_id))
    }

    /// Mesmo molde, mas com o servidor opcional — é o que permite testar o
    /// modelo que publica nesta máquina.
    fn profile_input_no_servidor(project_id: i64, server_id: Option<i64>) -> DeployProfileInput {
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
            // O molde é sem executor de runtime: as quinze etapas continuam exatamente
            // como eram, e quem quer Docker/Compose/PM2 declara isso no teste.
            runtime: None,
            image_name: None,
            dockerfile: None,
            compose_file: None,
            service_name: None,
            publish_ports: None,
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
        assert_eq!(saved.server_name.as_deref(), Some("web-prod"));
        assert_eq!(saved.server_environment.as_deref(), Some("prod"));

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

        // apagar o modelo remove as execuções junto (não há chave estrangeira ativa)
        let outra_execucao = insert_run(&conn, other.id, &proc_de_teste(&other.remote_path)).unwrap();
        delete_deploy_profile(&conn, other.id).unwrap();
        assert!(get_deploy_profile(&conn, other.id).is_err());
        assert!(get_run(&conn, outra_execucao).is_err());
    }

    #[test]
    fn modelo_sem_servidor_guarda_o_destino_local() {
        let conn = mem_db();
        let (project_id, server_id) = seeded(&conn);

        let mut local = profile_input_no_servidor(project_id, None);
        local.name = "cópia local".into();
        local.strategy = "artifacts".into();
        local.releases_enabled = false;
        local.remote_path = "C:\\publicados\\site".into();
        let salvo = upsert_deploy_profile(&conn, &local).unwrap();

        assert_eq!(salvo.server_id, None);
        assert_eq!(salvo.server_name, None);
        assert_eq!(salvo.server_environment, None);
        assert_eq!(salvo.remote_path, "C:\\publicados\\site");
        assert!(!salvo.releases_enabled);

        // o modelo com servidor continua mostrando o servidor dele
        let remoto = profile_input(project_id, server_id);
        let remoto = upsert_deploy_profile(&conn, &remoto).unwrap();

        let listados = list_deploy_profiles(&conn, Some(project_id)).unwrap();
        assert_eq!(listados.len(), 2);
        let lido_local = listados.iter().find(|p| p.id == salvo.id).unwrap();
        assert_eq!(lido_local.server_name, None);
        let lido_remoto = listados.iter().find(|p| p.id == remoto.id).unwrap();
        assert_eq!(lido_remoto.server_name.as_deref(), Some("web-prod"));

        // uma execução de um modelo sem servidor também aparece no histórico,
        // sem nome de servidor
        let run_id = insert_run(&conn, salvo.id, &proc_de_teste(&salvo.remote_path)).unwrap();
        let execucao = get_run(&conn, run_id).unwrap();
        assert_eq!(execucao.server_name, None);
        assert_eq!(execucao.strategy, "artifacts");
    }

    #[test]
    fn apagar_servidor_ou_projeto_leva_os_modelos_e_as_execucoes_junto() {
        let conn = mem_db();
        let server = insert_server(&conn, &sample_input(), "manual").unwrap();
        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();
        let modelo = upsert_deploy_profile(&conn, &profile_input(projeto.id, server.id)).unwrap();
        let run_id = insert_run(&conn, modelo.id, &proc_de_teste(&modelo.remote_path)).unwrap();

        // um modelo que publica nesta máquina não depende do servidor
        let mut local = profile_input_no_servidor(projeto.id, None);
        local.name = "cópia local".into();
        local.releases_enabled = false;
        local.remote_path = "C:/publicados/site".into();
        let modelo_local = upsert_deploy_profile(&conn, &local).unwrap();

        assert_eq!(delete_server_row(&conn, server.id).unwrap(), 1);
        assert!(get_deploy_profile(&conn, modelo.id).is_err());
        assert!(get_run(&conn, run_id).is_err());
        assert!(get_deploy_profile(&conn, modelo_local.id).is_ok());

        // sobrou só o modelo do projeto — apagar o projeto o leva junto
        assert_eq!(delete_project_row(&conn, projeto.id).unwrap(), 1);
        assert!(get_deploy_profile(&conn, modelo_local.id).is_err());
        assert!(get_project(&conn, projeto.id).is_err());
        assert!(list_deploy_profiles(&conn, None).unwrap().is_empty());
    }

    /// O esquema de antes da v5 e da v6: `server_id` obrigatório, sem coluna de
    /// versão na definição e sem procedência no histórico. É o ponto de partida que
    /// a migração precisa sobreviver — inclusive porque o teste recria essas tabelas
    /// por cima de um banco que já passou pela v6.
    const ESQUEMA_ANTIGO: &str = "PRAGMA foreign_keys=OFF;
        DELETE FROM pipeline_revisions;
        DROP TABLE deploy_profiles;
        CREATE TABLE deploy_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            server_id INTEGER NOT NULL,
            strategy TEXT NOT NULL,
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
            test_commands TEXT NOT NULL DEFAULT '[]',
            UNIQUE (project_id, name)
        );
        -- o histórico também era mais estreito: sem versão da definição e sem
        -- destino, ambiente ou commit de origem.
        DELETE FROM deploy_run_steps;
        DROP TABLE deploy_runs;
        CREATE TABLE deploy_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','success','failed','cancelled')),
            started_at TEXT NOT NULL,
            finished_at TEXT,
            release_dir TEXT,
            previous_release TEXT,
            detail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_runs_started ON deploy_runs(started_at DESC);
        -- as linhas de etapa também eram estreitas: nada ligava uma linha a um nó do
        -- grafo, porque o grafo ainda não existia.
        DROP TABLE deploy_run_steps;
        CREATE TABLE deploy_run_steps (
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
        CREATE UNIQUE INDEX idx_run_steps_run ON deploy_run_steps(run_id, position);
        PRAGMA foreign_keys=ON;";

    #[test]
    fn migracao_preserva_modelos_e_historico_e_afrouxa_o_servidor() {
        let conn = mem_db();
        conn.execute_batch(ESQUEMA_ANTIGO).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();

        let server = insert_server(&conn, &sample_input(), "manual").unwrap();
        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();

        // O modelo e a execução entram por SQL cru, escritos como o código antigo os
        // gravaria: pelas funções de hoje o insert já pediria `version` e as colunas
        // de procedência, que não existiam nessa base.
        conn.execute(
            "INSERT INTO deploy_profiles (project_id, name, server_id, strategy, git_branch, remote_path, \
             include_paths, exclude_paths, prepare_commands, build_commands, restart_commands, health_checks, \
             rollback_commands, secret_keys, backup_enabled, releases_enabled, delete_extras, step_timeout_secs, \
             is_default, created_at, updated_at, test_commands) \
             VALUES (?1, 'produção', ?2, 'artifacts', 'main', '/var/www/site', '[\"dist\"]', '[\"**/*.map\"]', \
             '[]', '[\"npm ci\",\"npm run build\"]', '[\"sudo systemctl reload caddy\"]', \
             '[\"curl -fsS http://127.0.0.1/healthz\"]', '[]', '[\"SENTRY_TOKEN\"]', 1, 1, 0, 900, 1, \
             ?3, ?3, '[\"npm run test\"]')",
            rusqlite::params![projeto.id, server.id, now_rfc3339()],
        )
        .unwrap();
        let modelo_id: i64 = conn
            .query_row("SELECT id FROM deploy_profiles WHERE name = 'produção'", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO deploy_runs (profile_id, status, started_at, detail) VALUES (?1, 'success', ?2, 'publicado')",
            rusqlite::params![modelo_id, now_rfc3339()],
        )
        .unwrap();
        let run_id: i64 = conn
            .query_row("SELECT id FROM deploy_runs WHERE profile_id = ?1", [modelo_id], |r| r.get(0))
            .unwrap();
        // a linha de etapa também entra por SQL cru: pelas funções de hoje ela pediria
        // as colunas do grafo, que uma base dessa época não tinha.
        conn.execute(
            "INSERT INTO deploy_run_steps (run_id, position, name, status, command, exit_code, message) \
             VALUES (?1, 1, 'Validar configurações e credenciais', 'done', NULL, 0, 'modelo pronto')",
            [run_id],
        )
        .unwrap();

        migrate(&conn).unwrap();

        // nada se perdeu na reconstrução da tabela
        let lido = get_deploy_profile(&conn, modelo_id).unwrap();
        assert_eq!(lido.name, "produção");
        assert_eq!(lido.remote_path, "/var/www/site");
        assert_eq!(lido.server_name.as_deref(), Some("web-prod"));
        assert_eq!(lido.test_commands, vec!["npm run test".to_string()]);
        assert!(lido.is_default && lido.releases_enabled);

        // a definição antiga recebe a versão 1 e o retrato dela, sem isso a primeira
        // execução depois da migração apontaria para uma versão nunca registrada
        assert_eq!(lido.version, 1);
        let retratos = list_revisions(&conn, modelo_id).unwrap();
        assert_eq!(retratos.len(), 1);
        assert_eq!(retratos[0].version, 1);
        assert_eq!(retratos[0].spec["nome"], "produção");

        // o histórico é o ponto crítico: um DROP em cascata apagaria tudo
        let execucao = get_run(&conn, run_id).unwrap();
        assert_eq!(execucao.profile_id, modelo_id);
        assert_eq!(execucao.steps.len(), 1);
        // a linha antiga continua linear: sem nó e sem marca de destino
        assert_eq!(execucao.steps[0].node_key, None);
        assert!(!execucao.steps[0].touched_destination);
        assert_eq!(execucao.steps[0].status, "done");
        assert_eq!(execucao.server_name.as_deref(), Some("web-prod"));
        // e a execução antiga não finge procedência: ela simplesmente não tem
        assert_eq!(execucao.definition_version, None);
        assert_eq!(execucao.destination, None);
        assert_eq!(execucao.commit_sha, None);

        // e a checagem de chaves continua ligada depois da migração
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1, "a migração deixou as chaves desligadas");

        // a mesma base agora aceita um modelo sem servidor
        let mut local = profile_input_no_servidor(projeto.id, None);
        local.name = "cópia local".into();
        local.releases_enabled = false;
        local.remote_path = "C:/publicados/site".into();
        let salvo = upsert_deploy_profile(&conn, &local).unwrap();
        assert_eq!(salvo.server_id, None);

        // mexer numa definição sobe a versão e deixa o retrato do que ela era antes
        let mut editado = local.clone();
        editado.id = Some(salvo.id);
        editado.remote_path = "D:/publicados/site".into();
        let segunda = upsert_deploy_profile(&conn, &editado).unwrap();
        assert_eq!(segunda.version, 2);
        let retratos = list_revisions(&conn, salvo.id).unwrap();
        assert_eq!(retratos.len(), 2);
        assert_eq!(retratos[0].spec["destino"], "D:/publicados/site");
        assert_eq!(retratos[1].spec["destino"], "C:/publicados/site");

        // uma execução aberta agora registra de onde saiu
        let nova = insert_run(
            &conn,
            salvo.id,
            &Procedencia {
                definition_version: 2,
                destination: "D:/publicados/site".into(),
                environment: "local".into(),
                commit_sha: Some("3f9a1c2".into()),
            },
        )
        .unwrap();
        let execucao = get_run(&conn, nova).unwrap();
        assert_eq!(execucao.definition_version, Some(2));
        assert_eq!(execucao.destination.as_deref(), Some("D:/publicados/site"));
        assert_eq!(execucao.environment.as_deref(), Some("local"));
        assert_eq!(execucao.commit_sha.as_deref(), Some("3f9a1c2"));

        // apagar o servidor continua levando só os modelos dele
        assert_eq!(delete_server_row(&conn, server.id).unwrap(), 1);
        assert!(get_deploy_profile(&conn, modelo_id).is_err());
        assert!(get_run(&conn, run_id).is_err());
        assert!(get_deploy_profile(&conn, salvo.id).is_ok());
    }

    /// A base como a v7 a deixou, e não há migração simulada sem ela: aqui o `CHECK`
    /// do nó ainda recusa `git` e o modelo não tem nenhuma coluna de executor. Se a
    /// v8 fosse escrita em cima do esquema de hoje ela só acrescentaria colunas, e o
    /// que importa conferir é justamente o desenho antigo sobreviver à troca de tabela.
    const ESQUEMA_V7: &str = "PRAGMA foreign_keys=OFF;
        DELETE FROM pipeline_revisions;
        DROP TABLE pipeline_nodes;
        DROP TABLE deploy_profiles;
        CREATE TABLE deploy_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            server_id INTEGER REFERENCES servers(id) ON DELETE CASCADE,
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
            test_commands TEXT NOT NULL DEFAULT '[]',
            version INTEGER NOT NULL DEFAULT 1,
            UNIQUE (project_id, name)
        );
        CREATE TABLE pipeline_nodes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
            node_key TEXT NOT NULL,
            title TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('motor','comando','aprovacao')),
            engine_step INTEGER,
            executor TEXT NOT NULL DEFAULT 'local' CHECK (executor IN ('local','ssh')),
            environment TEXT,
            working_dir TEXT,
            commands TEXT NOT NULL DEFAULT '[]',
            depends_on TEXT NOT NULL DEFAULT '[]',
            parallel INTEGER NOT NULL DEFAULT 0,
            timeout_secs INTEGER NOT NULL DEFAULT 0,
            retry_limit INTEGER NOT NULL DEFAULT 0,
            condition TEXT,
            requires_approval INTEGER NOT NULL DEFAULT 0,
            touches_destination INTEGER NOT NULL DEFAULT 0,
            x INTEGER NOT NULL DEFAULT 0,
            y INTEGER NOT NULL DEFAULT 0,
            UNIQUE (profile_id, node_key)
        );
        CREATE INDEX idx_pipeline_nodes_profile ON pipeline_nodes(profile_id, x, y);
        -- O histórico da v7 já tinha a procedência, mas nenhuma coluna de imagem: é
        -- ela que a v9 acrescenta, e recriar a tabela aqui é o que impede a base
        -- antiga de um teste já trazer a resposta da migração.
        DELETE FROM deploy_run_steps;
        DROP TABLE deploy_runs;
        CREATE TABLE deploy_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            profile_id INTEGER NOT NULL REFERENCES deploy_profiles(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running','success','failed','cancelled')),
            started_at TEXT NOT NULL,
            finished_at TEXT,
            release_dir TEXT,
            previous_release TEXT,
            detail TEXT,
            definition_version INTEGER,
            destination TEXT,
            environment TEXT,
            commit_sha TEXT
        );
        CREATE INDEX idx_runs_started ON deploy_runs(started_at DESC);
        PRAGMA foreign_keys=ON;";

    #[test]
    fn a_v8_alarga_o_check_do_no_sem_perder_o_desenho_salvo() {
        let conn = mem_db();
        conn.execute_batch(ESQUEMA_V7).unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();

        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();
        let agora = now_rfc3339();
        conn.execute(
            "INSERT INTO deploy_profiles (project_id, name, server_id, strategy, remote_path, \
             created_at, updated_at) VALUES (?1, 'api', NULL, 'script', '/srv/api', ?2, ?2)",
            rusqlite::params![projeto.id, agora],
        )
        .unwrap();
        let modelo_id: i64 = conn
            .query_row("SELECT id FROM deploy_profiles WHERE name = 'api'", [], |r| r.get(0))
            .unwrap();
        // um grafo desenhado na v7: dois nós, com aresta e executor
        conn.execute(
            "INSERT INTO pipeline_nodes (profile_id, node_key, title, kind, executor, commands, \
             depends_on, touches_destination, x, y) \
             VALUES (?1, 'prepara', 'Preparo', 'comando', 'ssh', '[\"npm ci\"]', '[]', 1, 10, 20)",
            [modelo_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pipeline_nodes (profile_id, node_key, title, kind, engine_step, depends_on, x, y) \
             VALUES (?1, 'envio', 'Envio', 'motor', 10, '[\"prepara\"]', 30, 20)",
            [modelo_id],
        )
        .unwrap();

        // na base antiga o tipo novo não entra — é exatamente por isso que a tabela
        // precisa ser reconstruída, e não só completada com ALTER
        let recusa_antiga = conn.execute(
            "INSERT INTO pipeline_nodes (profile_id, node_key, title, kind) VALUES (?1, 'marca', 'Marca', 'git')",
            [modelo_id],
        );
        assert!(recusa_antiga.unwrap_err().to_string().contains("CHECK"));

        migrate(&conn).unwrap();

        let lidos = list_pipeline_nodes(&conn, modelo_id).unwrap();
        assert_eq!(lidos.len(), 2, "a reconstrução perdeu nós");
        assert_eq!(lidos[0].node_key, "prepara");
        assert_eq!(lidos[0].executor, "ssh");
        assert_eq!(lidos[0].commands, vec!["npm ci".to_string()]);
        assert!(lidos[0].touches_destination);
        assert_eq!(lidos[1].depends_on, vec!["prepara".to_string()]);
        assert_eq!(lidos[1].engine_step, Some(10));
        // e um nó que não é git não tem operação nem referência
        assert_eq!(lidos[0].git_op, None);
        assert_eq!(lidos[0].git_ref, None);

        // o modelo antigo ganha as colunas vazias, e vazio quer dizer nenhum executor
        let velho = get_deploy_profile(&conn, modelo_id).unwrap();
        assert_eq!(velho.runtime, "");
        assert_eq!(velho.image_name, None);
        assert!(velho.publish_ports.is_empty());
        assert_eq!(velho.name, "api");
        assert_eq!(velho.strategy, "script");

        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!(fk, 1, "a migração deixou as chaves desligadas");

        // agora sim o tipo novo entra, pela porta de fora: a mesma gravação da tela
        let mut marca = PipelineNodeInput {
            node_key: "marca".into(),
            title: "Marca o lançamento".into(),
            kind: "git".into(),
            engine_step: None,
            executor: Some("local".into()),
            environment: None,
            working_dir: None,
            commands: None,
            git_op: Some(" TAG ".into()),
            git_ref: Some(" v1.2.0 ".into()),
            depends_on: None,
            parallel: Some(false),
            timeout_secs: Some(0),
            retry_limit: Some(0),
            condition: None,
            requires_approval: Some(false),
            x: Some(50),
            y: Some(20),
        };
        let salvos = replace_pipeline_nodes(&conn, modelo_id, &[marca.clone()]).unwrap();
        assert_eq!(salvos.len(), 1);
        let no_git = &salvos[0];
        assert_eq!(no_git.kind, "git");
        assert_eq!(no_git.git_op.as_deref(), Some("tag"));
        assert_eq!(no_git.git_ref.as_deref(), Some("v1.2.0"));
        assert_eq!(no_git.executor, "local", "o nó git roda nesta máquina");
        assert!(!no_git.touches_destination);
        assert!(no_git.commands.is_empty(), "nó git não tem comandos soltos");

        // e o executor do modelo é salvo lido de volta, com as portas. Docker, Compose
        // e PM2 rodam no destino, e o destino de um executor é o servidor: o molde
        // abaixo é cadastrado sobre um.
        let web = insert_server(&conn, &sample_input(), "manual").unwrap();
        let mut com_docker = profile_input(projeto.id, web.id);
        com_docker.name = "com docker".into();
        com_docker.strategy = "artifacts".into();
        com_docker.remote_path = "/srv/api".into();
        com_docker.releases_enabled = false;
        com_docker.runtime = Some(" docker ".into());
        com_docker.image_name = Some(" meuapp ".into());
        com_docker.service_name = Some(" api ".into());
        com_docker.publish_ports = Some(vec![" 8080:80 ".into(), String::new()]);
        com_docker.compose_file = Some("docker-compose.yml".into());
        let docker = upsert_deploy_profile(&conn, &com_docker).unwrap();
        assert_eq!(docker.runtime, "docker");
        assert_eq!(docker.image_name.as_deref(), Some("meuapp"));
        assert_eq!(docker.service_name.as_deref(), Some("api"));
        assert_eq!(docker.publish_ports, vec!["8080:80".to_string()]);
        // o arquivo do compose não é do executor docker: não fica guardado
        assert_eq!(docker.compose_file, None);

        // o retrato da versão conta o executor, porque é parte do que foi executado
        let retratos = list_revisions(&conn, docker.id).unwrap();
        assert_eq!(retratos[0].spec["executor"], "docker");
        assert_eq!(retratos[0].spec["portas"], serde_json::json!(["8080:80"]));

        // PM2 não tem imagem: o que sobra de outro executor não fica guardado. E as
        // portas nem chegam ao gravador — a checagem do PM2 recusa porta declarada.
        let mut com_pm2 = com_docker.clone();
        com_pm2.id = Some(docker.id);
        com_pm2.runtime = Some("pm2".into());
        com_pm2.service_name = Some("api".into());
        com_pm2.publish_ports = None;
        let pm2 = upsert_deploy_profile(&conn, &com_pm2).unwrap();
        assert_eq!(pm2.runtime, "pm2");
        assert_eq!(pm2.image_name, None);
        assert!(pm2.publish_ports.is_empty());
        assert_eq!(pm2.dockerfile, None);
        assert_eq!(pm2.service_name.as_deref(), Some("api"));
        let mut porta_no_pm2 = com_pm2.clone();
        porta_no_pm2.publish_ports = Some(vec!["8080:80".into()]);
        assert!(upsert_deploy_profile(&conn, &porta_no_pm2)
            .unwrap_err()
            .contains("próprio app"));
        let mut sem_app = com_pm2.clone();
        sem_app.service_name = None;
        assert!(upsert_deploy_profile(&conn, &sem_app)
            .unwrap_err()
            .contains("pm2 list"));

        // E um executor sem servidor não passa pela gravação: o modelo publica nesta
        // máquina, e não há docker/pm2 aqui para subir nada.
        let mut docker_local = com_docker.clone();
        docker_local.id = None;
        docker_local.name = "docker nesta máquina".into();
        docker_local.server_id = None;
        docker_local.remote_path = "C:/publicados/api".into();
        assert!(upsert_deploy_profile(&conn, &docker_local)
            .unwrap_err()
            .contains("nesta máquina"));

        // e o nó git sem referência é recusado pelo validador, não pelo banco
        marca.git_ref = None;
        let erro = marca.validate().unwrap_err();
        assert!(erro.contains("referência"), "{erro}");
    }

    #[test]
    fn a_v9_grava_a_imagem_que_a_reversao_de_docker_precisa_encontrar() {
        let conn = mem_db();
        conn.execute_batch(ESQUEMA_V7).unwrap();
        // A base levada até a v8 de verdade, com o próprio script da v8: assim o
        // histórico fica na forma que ele tinha antes desta etapa — com procedência,
        // sem imagem anterior.
        conn.execute_batch(MIGRATIONS[7]).unwrap();
        conn.pragma_update(None, "user_version", 8).unwrap();

        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();
        let agora = now_rfc3339();
        conn.execute(
            "INSERT INTO deploy_profiles (project_id, name, server_id, strategy, remote_path, \
             created_at, updated_at) VALUES (?1, 'api', NULL, 'script', '/srv/api', ?2, ?2)",
            rusqlite::params![projeto.id, agora],
        )
        .unwrap();
        let modelo_id: i64 = conn
            .query_row("SELECT id FROM deploy_profiles WHERE name = 'api'", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO deploy_runs (profile_id, status, started_at, release_dir, previous_release, detail) \
             VALUES (?1, 'success', ?2, '/srv/api/2026', '2025', 'rodou')",
            rusqlite::params![modelo_id, agora],
        )
        .unwrap();
        let execucao = conn.last_insert_rowid();

        // Sem a coluna não há onde registrar a imagem: é isto que a reversão de um
        // container procuraria e não encontraria em uma base antiga.
        assert!(conn
            .execute("UPDATE deploy_runs SET previous_image = 'sha256:abc'", [])
            .is_err());

        migrate(&conn).unwrap();

        // A execução sobrevive, e a imagem anterior dela é `None` — nada lido, nada
        // inventado: um histórico antigo não passa a ter um destino de reversão.
        let antes = get_run(&conn, execucao).unwrap();
        assert_eq!(antes.status, "success");
        assert_eq!(antes.previous_release.as_deref(), Some("2025"));
        assert_eq!(antes.previous_image, None);

        finish_run(
            &conn,
            execucao,
            "success",
            Some("/srv/api/2026"),
            Some("2025"),
            Some("sha256:abc123"),
            Some("site · running"),
        )
        .unwrap();
        let depois = get_run(&conn, execucao).unwrap();
        assert_eq!(depois.previous_image.as_deref(), Some("sha256:abc123"));
        assert_eq!(depois.detail.as_deref(), Some("site · running"));

        // E um encerramento sem imagem escreve vazio de propósito: uma corrida que não
        // trocou container nenhum não pode herdar a imagem da corrida de antes.
        finish_run(&conn, execucao, "failed", None, None, None, Some("falhou")).unwrap();
        assert_eq!(get_run(&conn, execucao).unwrap().previous_image, None);
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
        servidor_falso.server_id = Some(4242);
        assert!(upsert_deploy_profile(&conn, &servidor_falso)
            .unwrap_err()
            .contains("servidor"));

        // Estratégia que exige host não pode ficar sem servidor.
        let mut sem_servidor_remoto = profile_input_no_servidor(project_id, None);
        sem_servidor_remoto.strategy = "sftp".into();
        assert!(upsert_deploy_profile(&conn, &sem_servidor_remoto)
            .unwrap_err()
            .contains("escolha script"));

        // Sem servidor, releases por symlink não existem.
        let mut sem_servidor_release = profile_input_no_servidor(project_id, None);
        sem_servidor_release.remote_path = "C:/publicados/site".into();
        assert!(upsert_deploy_profile(&conn, &sem_servidor_release)
            .unwrap_err()
            .contains("symlink"));

        // ...e o destino precisa ser um caminho absoluto desta máquina.
        let mut sem_servidor_caminho = profile_input_no_servidor(project_id, None);
        sem_servidor_caminho.releases_enabled = false;
        sem_servidor_caminho.remote_path = "publicados/site".into();
        assert!(upsert_deploy_profile(&conn, &sem_servidor_caminho)
            .unwrap_err()
            .contains("nesta máquina"));

        // Nem uma raiz de disco nem um caminho que escapa por "..".
        for destino in ["C:", "C:/", "/", "//", "../site", "/var/../etc"] {
            let mut escapando = profile_input_no_servidor(project_id, None);
            escapando.releases_enabled = false;
            escapando.remote_path = destino.into();
            assert!(upsert_deploy_profile(&conn, &escapando)
                .unwrap_err()
                .contains("nesta máquina"), "aceitou o destino {destino}");
        }

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

    #[test]
    fn a_marca_terminal_do_no_e_quem_manda_na_mensagem_da_linha() {
        let conn = mem_db();
        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();
        let mut entrada = profile_input_no_servidor(projeto.id, None);
        entrada.releases_enabled = false;
        entrada.remote_path = "C:/publicados/site".into();
        let modelo = upsert_deploy_profile(&conn, &entrada).unwrap();
        let execucao = insert_run(&conn, modelo.id, &proc_de_teste(&modelo.remote_path)).unwrap();

        let grava = |posicao: i64, status: &str, mensagem: Option<&str>| {
            upsert_step(
                &conn,
                &StepUpdate {
                    run_id: execucao,
                    position: posicao,
                    name: "preparo",
                    status,
                    command: None,
                    exit_code: None,
                    message: mensagem,
                    node_key: Some("preparo"),
                    touched_destination: false,
                },
            )
            .unwrap();
        };
        let le = |posicao: i64| -> (String, Option<String>, Option<String>) {
            conn.query_row(
                "SELECT status, message, started_at FROM deploy_run_steps WHERE run_id = ?1 AND position = ?2",
                rusqlite::params![execucao, posicao],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };

        // O texto "em execução" serve enquanto o nó roda, e é essa marca que grava a
        // hora de início — sem ela o histórico do nó de comando não tem duração.
        grava(4, "waiting", None);
        grava(4, "running", Some("em execução"));
        let (estado, mensagem, inicio) = le(4);
        assert_eq!(estado, "running");
        assert_eq!(mensagem.as_deref(), Some("em execução"));
        assert!(inicio.is_some());

        // A linha final de um nó de comando não tem texto próprio para dizer: ela tem de
        // perder o que dizia da execução, e não pode herdar "em execução" para sempre.
        grava(4, "done", None);
        let (estado, mensagem, inicio) = le(4);
        assert_eq!(estado, "done");
        assert_eq!(mensagem, None, "o texto de execução não sobrevive à linha concluída");
        assert!(inicio.is_some(), "a hora de início fica: é o que diz quando o nó começou");

        // Já a etapa do motor escreve a própria linha final com a mensagem do resultado,
        // e nada apaga o que ela disse.
        grava(5, "running", Some("em execução"));
        grava(5, "done", Some("2 arquivo(s) selecionados"));
        assert_eq!(le(5).1.as_deref(), Some("2 arquivo(s) selecionados"));
    }

    #[test]
    fn grafo_salvo_vira_definicao_versionada_com_a_ordem_da_tela() {
        let conn = mem_db();
        let projeto = insert_project(&conn, "site", "C:/projetos/site", true, None, None).unwrap();
        let mut sem_servidor = profile_input_no_servidor(projeto.id, None);
        sem_servidor.releases_enabled = false;
        sem_servidor.remote_path = "C:/publicados/site".into();
        let modelo = upsert_deploy_profile(&conn, &sem_servidor).unwrap();

        let no = |chave: &str, kind: &str, etapa: Option<i64>, deps: &[&str], x: i64| PipelineNodeInput {
            node_key: chave.into(),
            title: chave.into(),
            kind: kind.into(),
            engine_step: etapa,
            executor: Some("local".into()),
            environment: None,
            working_dir: None,
            commands: Some(vec!["echo pronto".into()]),
            git_op: None,
            git_ref: None,
            depends_on: Some(deps.iter().map(|d| d.to_string()).collect()),
            parallel: Some(false),
            timeout_secs: Some(0),
            retry_limit: Some(1),
            condition: Some("ramo=main".into()),
            requires_approval: Some(etapa == Some(10)),
            x: Some(x),
            y: Some(0),
        };

        replace_pipeline_nodes(
            &conn,
            modelo.id,
            &[
                no("migracao", "comando", None, &["coleta"], 10),
                no("coleta", "motor", Some(4), &[], 20),
                no("envio", "motor", Some(10), &["coleta"], 30),
            ],
        )
        .unwrap();

        // lido na ordem do desenho (x), com as arestas e o resto do nó intactos
        let lidos = list_pipeline_nodes(&conn, modelo.id).unwrap();
        assert_eq!(lidos.len(), 3);
        assert_eq!(lidos[0].node_key, "migracao");
        assert_eq!(lidos[0].commands, vec!["echo pronto".to_string()]);
        assert_eq!(lidos[1].depends_on, Vec::<String>::new());
        assert_eq!(lidos[2].depends_on, vec!["coleta".to_string()]);
        assert_eq!(lidos[2].condition.as_deref(), Some("ramo=main"));
        assert!(lidos[2].requires_approval);
        assert!(!lidos[0].requires_approval);
        // tocar o destino é derivado do tipo: a etapa 4 escreve na pasta do projeto,
        // a 10 escreve no destino, um comando local não escreve no destino
        assert!(!lidos[0].touches_destination);
        assert!(!lidos[1].touches_destination);
        assert!(lidos[2].touches_destination);

        // gravar o grafo é mudar a definição: a versão sobe e o retrato traz os nós
        let depois = get_deploy_profile(&conn, modelo.id).unwrap();
        assert_eq!(depois.version, modelo.version + 1);
        let retratos = list_revisions(&conn, modelo.id).unwrap();
        assert_eq!(retratos[0].version, depois.version);
        assert_eq!(retratos[0].spec["nos"].as_array().unwrap().len(), 3);

        // substituir por nada volta à fila de quinze etapas, sem perder a história
        let sem_no = replace_pipeline_nodes(&conn, modelo.id, &[]).unwrap();
        assert!(sem_no.is_empty());
        assert_eq!(list_pipeline_nodes(&conn, modelo.id).unwrap().len(), 0);
        assert_eq!(list_revisions(&conn, modelo.id).unwrap()[0].spec["nos"].as_array().unwrap().len(), 0);

        // e um nó fora do tipo que o motor conhece é recusado pelo SQLite
        let quebrado = no("estranho", "agenda", None, &[], 40);
        assert!(replace_pipeline_nodes(&conn, modelo.id, &[quebrado]).is_err());
    }
}
