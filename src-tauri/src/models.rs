use serde::{Deserialize, Serialize};

const ENVIRONMENTS: [&str; 6] = ["dev", "staging", "prod", "database", "docker", "other"];
const AUTH_METHODS: [&str; 4] = ["key", "agent", "password", "unknown"];

pub fn validate_environment(env: &str) -> Result<(), String> {
    if ENVIRONMENTS.contains(&env) {
        Ok(())
    } else {
        Err(format!("ambiente inválido: {env}"))
    }
}

pub fn validate_auth_method(auth: &str) -> Result<(), String> {
    if AUTH_METHODS.contains(&auth) {
        Ok(())
    } else {
        Err(format!("método de autenticação inválido: {auth}"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_method: String,
    pub identity_file: Option<String>,
    pub ssh_alias: Option<String>,
    pub environment: String,
    pub group_name: Option<String>,
    pub notes: Option<String>,
    pub source: String,
    pub last_status: Option<String>,
    pub last_status_detail: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_ok_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_method: String,
    pub identity_file: Option<String>,
    pub ssh_alias: Option<String>,
    pub environment: String,
    pub group_name: Option<String>,
    pub notes: Option<String>,
}

impl ServerInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("o nome do servidor é obrigatório".into());
        }
        if self.host.trim().is_empty() {
            return Err("o host do servidor é obrigatório".into());
        }
        if self.username.trim().is_empty() {
            return Err("o usuário do servidor é obrigatório".into());
        }
        if !(1..=65535).contains(&self.port) {
            return Err(format!("porta inválida: {}", self.port));
        }
        if self.name.len() > 200 || self.host.len() > 255 {
            return Err("nome ou host excede o tamanho máximo".into());
        }
        validate_environment(&self.environment)?;
        validate_auth_method(&self.auth_method)?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub ok: bool,
    pub exit_code: Option<i64>,
    pub duration_ms: u64,
    pub error_kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSshHost {
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: i64,
    pub identity_file: Option<String>,
    pub environment: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostCandidate {
    pub patterns: Vec<String>,
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<i64>,
    pub identity_files: Vec<String>,
    pub proxy_jump: Option<String>,
    pub source_file: String,
    pub is_concrete: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFile {
    pub path: String,
    pub kind: String,
    pub key_type: Option<String>,
    pub has_pair: bool,
    pub encrypted: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentKey {
    pub fingerprint: String,
    pub key_type: String,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsInfo {
    pub path: String,
    pub total: i64,
    pub hashed: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub available: bool,
    pub keys: Vec<AgentKey>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshDiscoveryReport {
    pub config_files: Vec<String>,
    pub hosts: Vec<SshHostCandidate>,
    pub public_keys: Vec<KeyFile>,
    pub private_keys: Vec<KeyFile>,
    pub known_hosts: Option<KnownHostsInfo>,
    pub agent: AgentInfo,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolVersions {
    pub ssh: Option<String>,
    pub ssh_add: Option<String>,
    pub git: Option<String>,
    pub agent_running: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub local_path: String,
    pub is_git_repo: bool,
    pub git_branch: Option<String>,
    pub git_remote: Option<String>,
    pub default_server_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub ts: String,
    pub level: String,
    pub category: String,
    pub operation: String,
    pub server_name: Option<String>,
    pub project_name: Option<String>,
    pub exit_code: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchServerRow {
    pub id: i64,
    pub name: String,
    pub host: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProjectRow {
    pub id: i64,
    pub name: String,
    pub local_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub servers: Vec<SearchServerRow>,
    pub projects: Vec<SearchProjectRow>,
    pub history: Vec<AuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineParam {
    pub name: String,
    pub label: Option<String>,
    pub required: Option<bool>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub command: String,
    pub danger: String,
    pub params: Vec<RoutineParam>,
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInput {
    pub id: Option<i64>,
    pub name: String,
    pub description: Option<String>,
    pub command: String,
    pub danger: String,
    pub params: Vec<RoutineParam>,
}

impl RoutineInput {
    pub fn validate(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 200 {
            return Err("informe um nome de rotina com até 200 caracteres".into());
        }
        let command = self.command.trim();
        if command.is_empty() {
            return Err("informe o comando da rotina".into());
        }
        if command.len() > 5000 {
            return Err("o comando da rotina excede 5000 caracteres".into());
        }
        if !["read", "write"].contains(&self.danger.as_str()) {
            return Err(format!("classificação de comando inválida: {}", self.danger));
        }
        if self.params.len() > 10 {
            return Err("uma rotina aceita no máximo 10 parâmetros".into());
        }
        let mut names: Vec<&str> = Vec::new();
        for p in &self.params {
            let pname = p.name.trim();
            if pname.is_empty() || pname.len() > 60 {
                return Err("nome de parâmetro vazio ou longo demais".into());
            }
            if !pname.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                return Err(format!("nome de parâmetro inválido: {pname} (use letras, números, _ ou -)"));
            }
            if names.contains(&pname) {
                return Err(format!("parâmetro duplicado: {pname}"));
            }
            names.push(pname);
        }
        for ph in command_placeholders(command) {
            if !names.contains(&ph.as_str()) {
                return Err(format!("o comando usa o parâmetro {{{{{ph}}}}} mas ele não está definido"));
            }
        }
        Ok(())
    }
}

/// Extrai os nomes de parâmetros usados no comando na forma `{{nome}}`.
pub fn command_placeholders(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else { break };
        let name = &rest[..end];
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
        rest = &rest[end + 2..];
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSessionInfo {
    pub id: i64,
    pub server_id: i64,
    pub server_name: String,
    pub environment: String,
    pub username: String,
    pub host: String,
    pub port: i64,
    pub alive: bool,
    pub exit_code: Option<i64>,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalHistoryEntry {
    pub id: i64,
    pub ts: String,
    pub server_id: Option<i64>,
    pub server_name: Option<String>,
    pub command: String,
}

/// Uma entrada de `ls` remoto interpretada pelo explorador de arquivos.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub perms: Option<String>,
    pub size: Option<u64>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub modified: Option<String>,
}

/// As cinco estratégias de publicação do capítulo 9 da especificação.
pub const DEPLOY_STRATEGIES: [&str; 5] = ["sftp", "git", "artifacts", "script", "docker"];

pub fn validate_strategy(strategy: &str) -> Result<(), String> {
    if DEPLOY_STRATEGIES.contains(&strategy) {
        Ok(())
    } else {
        Err(format!("estratégia de deploy desconhecida: {strategy}"))
    }
}

fn is_safe_remote_path(path: &str) -> bool {
    !path.trim().is_empty()
        && path.starts_with('/')
        && !path.contains('\0')
        && !path.contains('\n')
        && !path.contains('\r')
        && path.len() <= 512
}

fn is_env_key_name(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    key.len() <= 80 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Lista de texto configurável (comandos, caminhos, chaves de segredo).
/// Nenhum item pode conter NUL; o tamanho total é limitado para impedir
/// entradas absurdas vindas da interface.
fn validate_list(items: &[String], label: &str, max_items: usize) -> Result<(), String> {
    if items.len() > max_items {
        return Err(format!("{label}: no máximo {max_items} itens"));
    }
    for item in items {
        if item.contains('\0') {
            return Err(format!("{label}: item contém caractere nulo"));
        }
        if item.contains('\n') || item.contains('\r') {
            return Err(format!("{label}: cada item deve ocupar uma única linha"));
        }
        if item.len() > 2000 {
            return Err(format!("{label}: item com mais de 2000 caracteres"));
        }
    }
    Ok(())
}

/// Um modelo de deploy: projeto + servidor + estratégia + comandos.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProfile {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub server_id: i64,
    pub server_name: String,
    pub server_environment: String,
    pub strategy: String,
    pub git_branch: Option<String>,
    pub remote_path: String,
    pub include_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub prepare_commands: Vec<String>,
    pub build_commands: Vec<String>,
    pub restart_commands: Vec<String>,
    pub health_checks: Vec<String>,
    pub rollback_commands: Vec<String>,
    /// Testes executados localmente antes de qualquer mudança remota (etapa 6).
    pub test_commands: Vec<String>,
    /// Apenas os nomes das variáveis. Os valores nunca são persistidos.
    pub secret_keys: Vec<String>,
    pub backup_enabled: bool,
    pub releases_enabled: bool,
    pub delete_extras: bool,
    pub step_timeout_secs: i64,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProfileInput {
    pub id: Option<i64>,
    pub project_id: i64,
    pub name: String,
    pub server_id: i64,
    pub strategy: String,
    pub git_branch: Option<String>,
    pub remote_path: String,
    pub include_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub prepare_commands: Vec<String>,
    pub build_commands: Vec<String>,
    pub restart_commands: Vec<String>,
    pub health_checks: Vec<String>,
    pub rollback_commands: Vec<String>,
    pub test_commands: Vec<String>,
    pub secret_keys: Vec<String>,
    pub backup_enabled: bool,
    pub releases_enabled: bool,
    pub delete_extras: bool,
    pub step_timeout_secs: i64,
    pub is_default: bool,
}

impl DeployProfileInput {
    pub fn validate(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 200 {
            return Err("dê um nome ao modelo de deploy (até 200 caracteres)".into());
        }
        if self.project_id <= 0 || self.server_id <= 0 {
            return Err("selecione o projeto e o servidor de destino".into());
        }
        validate_strategy(&self.strategy)?;
        if !is_safe_remote_path(self.remote_path.trim()) {
            return Err(
                "o diretório remoto deve ser um caminho absoluto, sem quebras de linha".into(),
            );
        }
        if self.step_timeout_secs < 10 || self.step_timeout_secs > 7200 {
            return Err("o tempo limite de cada etapa deve ficar entre 10 e 7200 segundos".into());
        }
        validate_list(&self.include_paths, "arquivos a enviar", 100)?;
        validate_list(&self.exclude_paths, "arquivos ignorados", 100)?;
        validate_list(&self.prepare_commands, "comandos de preparação", 40)?;
        validate_list(&self.build_commands, "comandos de build", 40)?;
        validate_list(&self.restart_commands, "comandos de reinicialização", 40)?;
        validate_list(&self.health_checks, "testes de saúde", 20)?;
        validate_list(&self.rollback_commands, "comandos de rollback", 40)?;
        validate_list(&self.test_commands, "comandos de teste", 40)?;

        for key in &self.secret_keys {
            if !is_env_key_name(key) {
                return Err(format!(
                    "nome de variável inválido: {key} (use letras, números e _ , começando por letra ou _)"
                ));
            }
        }
        if self.secret_keys.len() > 30 {
            return Err("no máximo 30 variáveis por modelo".into());
        }
        let mut seen: Vec<&str> = Vec::new();
        for key in &self.secret_keys {
            if seen.contains(&key.as_str()) {
                return Err(format!("variável duplicada: {key}"));
            }
            seen.push(key.as_str());
        }
        match self.strategy.as_str() {
            "git" => {
                let branch = self.git_branch.as_deref().unwrap_or("").trim();
                if branch.is_empty() {
                    return Err("o deploy por Git exige a branch de origem".into());
                }
                if !branch.is_ascii() || branch.contains(' ') || branch.contains('\0') {
                    return Err(format!("branch inválida: {branch}"));
                }
            }
            "script" => {
                if self.prepare_commands.iter().all(|c| c.trim().is_empty()) {
                    return Err("o deploy por script precisa de pelo menos um comando".into());
                }
            }
            "docker" => {
                let tem_comando = !self.build_commands.is_empty() || !self.restart_commands.is_empty();
                if !tem_comando {
                    return Err("o deploy com Docker precisa de comandos de build ou de atualização dos containers".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Estado de uma etapa do pipeline, conforme o capítulo 10 da especificação.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRunStep {
    pub position: i64,
    pub name: String,
    pub status: String,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub message: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// Um deploy registrado no histórico local.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRun {
    pub id: i64,
    pub profile_id: i64,
    pub project_name: String,
    pub server_name: String,
    pub strategy: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub release_dir: Option<String>,
    pub previous_release: Option<String>,
    pub detail: Option<String>,
    pub steps: Vec<DeployRunStep>,
}

