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
pub struct EditorInstalado {
    pub id: String,
    pub nome: String,
    /// Executável encontrado neste computador; é por ele que o app abre.
    pub caminho: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolVersions {
    pub ssh: Option<String>,
    pub ssh_add: Option<String>,
    pub agent_running: Option<bool>,
    /// Chaves carregadas no agente (`ssh-add -l`), ou `None` se não deu para perguntar.
    pub agent_keys: Option<i64>,
    pub git: Option<String>,
    /// `user.name` + `user.email` resolvidos pelo git nesta máquina; sem os dois
    /// um commit é recusado.
    pub git_user: Option<String>,
    pub gh: Option<String>,
    /// Conta ativa do `gh auth status` — nunca o token.
    pub gh_account: Option<String>,
    /// Programas que foram chamados e não terminaram dentro do prazo da sonda.
    /// Não é a mesma coisa que "ausente": o executável existe, só não respondeu,
    /// e a detecção não pode esperar por ele.
    pub sem_resposta: Vec<String>,
    /// Editores encontrados no disco. A lista da Configurações vem daqui.
    pub editors: Vec<EditorInstalado>,
}

#[derive(Debug, Clone, Serialize)]
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

/// Estratégias que existem sem host: rodar comandos aqui e copiar artefatos para
/// uma pasta local. As outras três (`sftp`, `git`, `docker`) falam com um servidor.
pub const LOCAL_STRATEGIES: [&str; 2] = ["script", "artifacts"];

pub fn strategy_needs_server(strategy: &str) -> bool {
    !LOCAL_STRATEGIES.contains(&strategy)
}

/// Caminho absoluto de pasta nesta máquina: POSIX (`/srv/site`), unidade Windows
/// (`D:\\publicados\\site`, `D:/site`) ou compartilhamento de rede (`\\\\srv\\site`).
/// Raiz pura (`/`, `D:\\`) e trechos com `..` são rejeitados: o destino de uma
/// publicação tem que ser uma pasta nomeada, nunca o topo de um disco.
fn is_safe_local_path(path: &str) -> bool {
    let raw = path.trim();
    if raw.is_empty()
        || raw.len() > 512
        || raw.contains('\0')
        || raw.contains('\n')
        || raw.contains('\r')
        || raw.contains("..")
    {
        return false;
    }
    let unificado = raw.replace('\\', "/");
    if unificado.starts_with("//") {
        return unificado.split('/').filter(|p| !p.is_empty()).count() >= 2;
    }
    if let Some(rest) = unificado.strip_prefix('/') {
        return !rest.trim_matches('/').is_empty();
    }
    let mut chars = unificado.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(letra), Some(':'), Some('/')) => letra.is_ascii_alphabetic() && !chars.as_str().trim_matches('/').is_empty(),
        _ => false,
    }
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

/// Um modelo de deploy: projeto + destino + estratégia + comandos. O destino é
/// um servidor cadastrado, ou esta mesma máquina quando `server_id` é `None`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProfile {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub server_id: Option<i64>,
    pub server_name: Option<String>,
    pub server_environment: Option<String>,
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
    /// Número da definição salva mais recente. Cada alteração sobe este número e
    /// deixa um retrato do que era a definição em `pipeline_revisions`.
    pub version: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProfileInput {
    pub id: Option<i64>,
    pub project_id: i64,
    pub name: String,
    /// `None` publica nesta máquina: sem SSH, sem SFTP e sem release por symlink.
    pub server_id: Option<i64>,
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
        if self.project_id <= 0 {
            return Err("selecione o projeto do modelo".into());
        }
        if let Some(id) = self.server_id {
            if id <= 0 {
                return Err("escolha o servidor de destino ou o modo sem servidor".into());
            }
        }
        validate_strategy(&self.strategy)?;
        if self.server_id.is_none() {
            if strategy_needs_server(&self.strategy) {
                return Err(format!(
                    "a estratégia {} publica em um servidor. Sem servidor escolha script ou artefatos.",
                    self.strategy
                ));
            }
            if self.releases_enabled {
                return Err(
                    "releases com symlink acontecem no servidor; sem servidor a publicação vai direto para a pasta de destino"
                        .into(),
                );
            }
            if !is_safe_local_path(self.remote_path.trim()) {
                return Err(
                    "a pasta de destino precisa ser um caminho absoluto nesta máquina, como D:\\publicados\\site ou /srv/site"
                        .into(),
                );
            }
        } else if !is_safe_remote_path(self.remote_path.trim()) {
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
    /// O nó do grafo que produziu esta linha. `None` em toda execução linear.
    pub node_key: Option<String>,
    /// Registrado pela própria etapa quando ela toca o destino: é o que faz uma
    /// execução ter de ser conferida antes de qualquer nova tentativa.
    pub touched_destination: bool,
}

/// Um deploy registrado no histórico local.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRun {
    pub id: i64,
    pub profile_id: i64,
    pub project_name: String,
    /// `None` quando o modelo publica nesta máquina — o destino é uma pasta local.
    pub server_name: Option<String>,
    pub strategy: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub release_dir: Option<String>,
    pub previous_release: Option<String>,
    pub detail: Option<String>,
    /// A versão da definição que estava salva quando a execução abriu. Um modelo
    /// editado depois não reescreve a procedência de uma execução antiga.
    pub definition_version: Option<i64>,
    /// Pasta ou diretório de destino, copiado da definição no momento da execução.
    pub destination: Option<String>,
    /// Ambiente do destino (`production`, `staging`…) lido do cadastro; `local`
    /// quando não há servidor.
    pub environment: Option<String>,
    /// Commit de origem lido do git da pasta do projeto ao abrir a execução.
    pub commit_sha: Option<String>,
    pub steps: Vec<DeployRunStep>,
}

/// Um retrato salvo da definição do pipeline, por versão.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionRevision {
    pub version: i64,
    pub saved_at: String,
    /// O JSON da definição: comandos, caminhos e limites. Nunca contém o **valor**
    /// das variáveis — só os nomes, como a própria tabela de modelos.
    pub spec: serde_json::Value,
}

// ---------- grafo do pipeline (plano CI/CD, etapa 3) ----------

/// Os três tipos de nó que o motor sabe executar. Não há tipo sem execução:
/// `motor` chama uma das quinze etapas reais, `comando` roda comandos pelo
/// executor escolhido e `aprovacao` espera a decisão de quem usa.
pub const NODE_KINDS: [&str; 3] = ["motor", "comando", "aprovacao"];
pub const NODE_EXECUTORS: [&str; 2] = ["local", "ssh"];
/// Limite do grafo salvo: acima disso o construtor deixa de ser legível e o
/// histórico de uma execução vira impossível de conferir.
pub const MAX_NODES: usize = 40;

/// Um nó do grafo, como está salvo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineNode {
    pub id: i64,
    pub profile_id: i64,
    /// Identificador estável dentro da definição — são as arestas que apontam
    /// para ele, então ele não pode mudar quando o nó é arrastado na tela.
    pub node_key: String,
    pub title: String,
    pub kind: String,
    /// Qual das quinze etapas do motor este nó executa (`motor`).
    pub engine_step: Option<i64>,
    pub executor: String,
    /// Nome do ambiente a que o nó se destina; usado pela condição de execução.
    pub environment: Option<String>,
    /// Pasta de trabalho do nó, relativa à raiz do projeto (`local`) ou absoluta
    /// no servidor (`ssh`).
    pub working_dir: Option<String>,
    pub commands: Vec<String>,
    /// As chaves dos nós que precisam terminar antes deste.
    pub depends_on: Vec<String>,
    /// Permite rodar junto com os irmãos que estiverem prontos ao mesmo tempo.
    pub parallel: bool,
    /// 0 = herdar o prazo do modelo.
    pub timeout_secs: i64,
    /// Quantas repetições o nó aceita quando falha. 0 = nenhuma.
    pub retry_limit: i64,
    /// Condição de execução: `ramo=<nome>` ou `ambiente=<nome>`; `None` = sempre.
    pub condition: Option<String>,
    pub requires_approval: bool,
    /// Derivado do tipo e do executor, nunca digitado: é o que diz se uma
    /// execução deste grafo pode ser repetida sem conferir o destino.
    pub touches_destination: bool,
    pub x: i64,
    pub y: i64,
}

/// O que a interface envia ao salvar o grafo. `id` e `touches_destination` não
/// vêm de lá: o identificador é a própria chave, e tocar o destino é consequência.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineNodeInput {
    pub node_key: String,
    pub title: String,
    pub kind: String,
    pub engine_step: Option<i64>,
    pub executor: Option<String>,
    pub environment: Option<String>,
    pub working_dir: Option<String>,
    pub commands: Option<Vec<String>>,
    pub depends_on: Option<Vec<String>>,
    pub parallel: Option<bool>,
    pub timeout_secs: Option<i64>,
    pub retry_limit: Option<i64>,
    pub condition: Option<String>,
    pub requires_approval: Option<bool>,
    pub x: Option<i64>,
    pub y: Option<i64>,
}

impl PipelineNodeInput {
    pub fn kind(&self) -> String {
        self.kind.trim().to_lowercase()
    }

    pub fn executor(&self) -> String {
        self.executor
            .as_deref()
            .unwrap_or("local")
            .trim()
            .to_lowercase()
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.clone().unwrap_or_default()
    }

    pub fn depends_on(&self) -> Vec<String> {
        self.depends_on.clone().unwrap_or_default()
    }

    /// Se o nó mexe no destino. `motor` a partir da etapa de preparação (8),
    /// que é onde a publicação começa a existir lá; `ssh` em qualquer comando,
    /// porque o que roda no servidor muda o servidor.
    pub fn touches_destination(&self) -> bool {
        match self.kind().as_str() {
            "motor" => self.engine_step.unwrap_or(0) >= 8,
            "comando" => self.executor() == "ssh",
            _ => false,
        }
    }

    /// Checagens de um nó sozinho. A forma do grafo (dependências, ciclos,
    /// paralelismo) é conferida em `pipeline::checar_grafo`, com todos os nós.
    pub fn validate(&self) -> Result<(), String> {
        let rotulo = if self.node_key.trim().is_empty() {
            "nó sem identificador".to_string()
        } else {
            format!("nó “{}”", self.node_key.trim())
        };
        if self.node_key.trim().is_empty() || self.node_key.len() > 40 {
            return Err(format!("{rotulo}: o identificador do nó deve ter de 1 a 40 caracteres"));
        }
        if !self
            .node_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("{rotulo}: o identificador aceita letra, número, hífen e sublinhado"));
        }
        let title = self.title.trim();
        if title.is_empty() || title.chars().count() > 120 {
            return Err(format!("{rotulo}: dê um nome ao nó (até 120 caracteres)"));
        }
        let kind = self.kind();
        if !NODE_KINDS.contains(&kind.as_str()) {
            return Err(format!(
                "{rotulo}: tipo de nó inválido “{kind}”; use motor, comando ou aprovacao"
            ));
        }
        match kind.as_str() {
            "motor" => {
                let Some(step) = self.engine_step else {
                    return Err(format!("{rotulo}: escolha qual das quinze etapas do motor ele roda"));
                };
                if !(1..=15).contains(&step) {
                    return Err(format!("{rotulo}: a etapa do motor vai de 1 a 15"));
                }
            }
            "comando" => {
                let executor = self.executor();
                if !NODE_EXECUTORS.contains(&executor.as_str()) {
                    return Err(format!("{rotulo}: executor inválido “{executor}”; use local ou ssh"));
                }
                let commands = self.commands();
                if commands.iter().all(|c| c.trim().is_empty()) {
                    return Err(format!("{rotulo}: um nó de comando precisa de pelo menos um comando"));
                }
                validate_list(&commands, &rotulo, 20)?;
                if let Some(dir) = self.working_dir.as_deref().map(str::trim) {
                    if !dir.is_empty() && executor == "local" {
                        if dir.starts_with('/') || dir.contains('\\') || dir.contains("..") {
                            return Err(format!(
                                "{rotulo}: a pasta de trabalho local deve ser um caminho relativo dentro do projeto"
                            ));
                        }
                    } else if !dir.is_empty() && executor == "ssh" && !is_safe_remote_path(dir) {
                        return Err(format!(
                            "{rotulo}: a pasta de trabalho no servidor precisa ser um caminho absoluto, como /srv/site"
                        ));
                    }
                }
            }
            _ => {}
        }
        let prazo = self.timeout_secs.unwrap_or(0);
        if !(0..=21_600).contains(&prazo) {
            return Err(format!("{rotulo}: o prazo do nó vai de 0 (herdar o do modelo) a 21600 segundos"));
        }
        let retry = self.retry_limit.unwrap_or(0);
        if !(0..=3).contains(&retry) {
            return Err(format!("{rotulo}: a política de repetição aceita de 0 a 3 novas tentativas"));
        }
        if let Some(raw) = self.condition.as_deref().map(str::trim) {
            if !raw.is_empty() {
                let (chave, valor) = raw
                    .split_once('=')
                    .ok_or_else(|| format!("{rotulo}: a condição deve ser ramo=<nome> ou ambiente=<nome>"))?;
                if !["ramo", "ambiente"].contains(&chave.trim()) {
                    return Err(format!(
                        "{rotulo}: condição “{chave}” não existe; o motor conhece ramo e ambiente"
                    ));
                }
                if valor.trim().is_empty() || valor.contains('\n') || valor.len() > 120 {
                    return Err(format!("{rotulo}: a condição precisa de um valor entre 1 e 120 caracteres"));
                }
            }
        }
        if let Some(env) = self.environment.as_deref().map(str::trim) {
            if env.len() > 60 || env.contains('\n') {
                return Err(format!("{rotulo}: nome de ambiente inválido"));
            }
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn entrada(server_id: Option<i64>, strategy: &str, destino: &str) -> DeployProfileInput {
        DeployProfileInput {
            id: None,
            project_id: 1,
            name: "cópia local".into(),
            server_id,
            strategy: strategy.into(),
            git_branch: None,
            remote_path: destino.into(),
            include_paths: vec!["dist".into()],
            exclude_paths: vec![],
            prepare_commands: vec![],
            build_commands: vec!["npm run build".into()],
            restart_commands: vec![],
            health_checks: vec![],
            rollback_commands: vec![],
            test_commands: vec![],
            secret_keys: vec![],
            backup_enabled: true,
            releases_enabled: false,
            delete_extras: false,
            step_timeout_secs: 600,
            is_default: false,
        }
    }

    #[test]
    fn destino_local_e_so_pasta_absoluta() {
        let longo = format!("C:/{}", "a".repeat(600));
        for aceito in [
            "C:/publicados/site",
            r"D:\publicados\site",
            "/srv/site",
            "/var/www/html",
            "//servidor/compartilhado/site",
        ] {
            assert!(is_safe_local_path(aceito), "recusou {aceito}");
        }
        for recusado in [
            "",
            "   ",
            "publicados/site",
            "C:",
            "C:/",
            "/",
            "//",
            "../site",
            "C:/publicados/../windows",
            "/var\0",
            "/var\nrm -rf /",
            longo.as_str(),
        ] {
            assert!(!is_safe_local_path(recusado), "aceitou {recusado}");
        }
    }

    #[test]
    fn sem_servidor_só_libera_estratégias_locais() {
        assert!(!strategy_needs_server("script"));
        assert!(!strategy_needs_server("artifacts"));
        for remota in ["sftp", "git", "docker"] {
            assert!(strategy_needs_server(remota), "{remota} parece local");
        }
    }

    #[test]
    fn valida_modelo_sem_servidor() {
        assert!(entrada(None, "artifacts", r"D:\publicados\site")
            .validate()
            .is_ok());

        // um script local precisa de pelo menos um comando
        let mut sem_comando = entrada(None, "script", "/srv/site");
        sem_comando.build_commands = vec![];
        assert!(sem_comando.validate().unwrap_err().contains("pelo menos um comando"));

        assert!(entrada(None, "sftp", "/srv/site")
            .validate()
            .unwrap_err()
            .contains("escolha script"));

        // releases por symlink só existem no servidor
        let mut com_release = entrada(None, "artifacts", "C:/publicados/site");
        com_release.releases_enabled = true;
        assert!(com_release.validate().unwrap_err().contains("symlink"));

        // destino relativo não é uma pasta absoluta desta máquina
        let relativo = entrada(None, "artifacts", "publicados");
        assert!(relativo.validate().unwrap_err().contains("nesta máquina"));

        // id <= 0 não é um servidor válido
        assert!(entrada(Some(0), "artifacts", "C:/publicados/site")
            .validate()
            .unwrap_err()
            .contains("modo sem servidor"));

        // com servidor, o caminho continua sendo o POSIX do servidor
        assert!(entrada(Some(3), "artifacts", "/var/www/site").validate().is_ok());
        assert!(entrada(Some(3), "artifacts", "C:/publicados/site")
            .validate()
            .unwrap_err()
            .contains("caminho absoluto"));
    }
}
