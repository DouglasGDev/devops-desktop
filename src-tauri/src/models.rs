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
            return Err(format!(
                "classificação de comando inválida: {}",
                self.danger
            ));
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
            if !pname
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "nome de parâmetro inválido: {pname} (use letras, números, _ ou -)"
                ));
            }
            if names.contains(&pname) {
                return Err(format!("parâmetro duplicado: {pname}"));
            }
            names.push(pname);
        }
        for ph in command_placeholders(command) {
            if !names.contains(&ph.as_str()) {
                return Err(format!(
                    "o comando usa o parâmetro {{{{{ph}}}}} mas ele não está definido"
                ));
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
        (Some(letra), Some(':'), Some('/')) => {
            letra.is_ascii_alphabetic() && !chars.as_str().trim_matches('/').is_empty()
        }
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
    /// O executor de runtime da publicação: vazio é "nenhum", e as etapas do motor
    /// continuam exatamente como eram. `docker`, `compose` e `pm2` fazem a etapa de
    /// atualização e a de saúde lerem o estado na ferramenta em vez de confiarem no
    /// código de saída do comando.
    pub runtime: String,
    /// Nome da imagem que o build produz (`docker run -t`).
    pub image_name: Option<String>,
    /// Caminho do Dockerfile **dentro** do código publicado.
    pub dockerfile: Option<String>,
    pub compose_file: Option<String>,
    /// Nome do service do compose, do container criado pelo `docker run` ou do app
    /// no `pm2 list` — é sempre a mesma coisa: como a coisa aparece na ferramenta.
    pub service_name: Option<String>,
    /// Publicações de porta no formato do `docker run -p`.
    pub publish_ports: Vec<String>,
}

impl DeployProfile {
    /// O alvo que o motor executa nesta definição. `diretorio` é onde o código está
    /// no destino daquela corrida (a pasta publicada no servidor, a raiz do projeto
    /// nesta máquina) — vem de quem chama porque é a única peça que não é da definição.
    pub fn alvo(&self, diretorio: &str) -> crate::runtime::Alvo {
        crate::runtime::Alvo::da_definicao(
            &crate::runtime::Definicao {
                runtime: &self.runtime,
                imagem: self.image_name.as_deref(),
                dockerfile: self.dockerfile.as_deref(),
                compose_file: self.compose_file.as_deref(),
                servico: self.service_name.as_deref(),
                portas: &self.publish_ports,
            },
            diretorio,
            self.step_timeout_secs,
        )
    }
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
    /// As colunas do executor de runtime. Faltando qualquer uma, `None` é o mesmo
    /// que coluna vazia: nenhum executor, nenhuma mudança de comportamento.
    pub runtime: Option<String>,
    pub image_name: Option<String>,
    pub dockerfile: Option<String>,
    pub compose_file: Option<String>,
    pub service_name: Option<String>,
    pub publish_ports: Option<Vec<String>>,
}

impl DeployProfileInput {
    /// O alvo declarado pelo que a tela enviou — a mesma tradução do modelo salvo,
    /// para a validação conferir exatamente o que o motor vai executar.
    pub fn alvo(&self, diretorio: &str) -> crate::runtime::Alvo {
        crate::runtime::Alvo::da_definicao(
            &crate::runtime::Definicao {
                runtime: self.runtime.as_deref().unwrap_or(""),
                imagem: self.image_name.as_deref(),
                dockerfile: self.dockerfile.as_deref(),
                compose_file: self.compose_file.as_deref(),
                servico: self.service_name.as_deref(),
                portas: self.publish_ports.as_deref().unwrap_or(&[]),
            },
            diretorio,
            self.step_timeout_secs,
        )
    }

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
        validate_list(
            self.publish_ports.as_deref().unwrap_or(&[]),
            "portas publicadas",
            20,
        )?;
        for (rotulo, valor) in [
            ("nome da imagem", self.image_name.as_deref()),
            ("Dockerfile", self.dockerfile.as_deref()),
            ("arquivo do compose", self.compose_file.as_deref()),
            ("nome do serviço", self.service_name.as_deref()),
        ] {
            if let Some(valor) = valor {
                if valor.trim().len() > 200 || valor.contains(['\0', '\n', '\r']) {
                    return Err(format!("{rotulo}: até 200 caracteres, em uma linha só"));
                }
            }
        }

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
                let tem_comando =
                    !self.build_commands.is_empty() || !self.restart_commands.is_empty();
                if !tem_comando && self.runtime.as_deref().unwrap_or("").trim().is_empty() {
                    // Com um executor de runtime declarado, a imagem desta corrida é
                    // construída e subida pelo próprio executor: não pedir comando.
                    return Err("o deploy com Docker precisa de comandos de build ou de atualização dos containers".into());
                }
            }
            _ => {}
        }
        // O executor de runtime é conferido pelo mesmo código que o motor usa: o que
        // a tela aceitou é exatamente o que a etapa sabe executar. O diretório é o
        // destino da publicação, porque é lá que o roteiro roda.
        let alvo = self.alvo(self.remote_path.trim());
        if self.server_id.is_none() && !alvo.runtime.is_empty() {
            return Err(format!(
                "{} é um executor de runtime e roda no servidor; este modelo publica nesta máquina",
                crate::runtime::nome(&alvo.runtime)
            ));
        }
        alvo.conferir()?;
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

/// Uma linha de saída de etapa que sobreviveu no banco. `attempt` é o que separa
/// as tentativas de um nó com repetição: cada tentativa é um bloco próprio, e a
/// ordem dentro do bloco é o `seq`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepLogLine {
    pub attempt: i64,
    pub seq: i64,
    pub stream: String,
    pub ts: String,
    pub text: String,
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
    /// A imagem que estava no ar antes desta corrida, lida do servidor durante a
    /// subida do container. É o único destino possível de uma reversão Docker: sem
    /// ela não há para onde voltar, e a tela diz isso em vez de prometer.
    pub previous_image: Option<String>,
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

/// Os cinco tipos de nó que o motor sabe executar. Não há tipo sem execução:
/// `motor` chama uma das quinze etapas reais, `comando` roda comandos pelo
/// executor escolhido, `aprovacao` espera a decisão de quem usa, `git` faz uma
/// operação real de Git nesta máquina, pelas mesmas funções da aba Git, e
/// `analise` varre o código-fonte pelo mesmo caminho da aba Análise.
pub const NODE_KINDS: [&str; 5] = ["motor", "comando", "aprovacao", "git", "analise"];
pub const NODE_EXECUTORS: [&str; 2] = ["local", "ssh"];
/// O que um nó `git` pode fazer. São operações que existem em `git.rs`, não uma
/// lista de desejos: `pull` atualiza o código, `checkout` alterna para a referência
/// declarada, `tag` marca o commit e, havendo remoto, publica a marca.
pub const NODE_GIT_OPS: [&str; 3] = ["pull", "checkout", "tag"];
/// Os instrumentos de um nó `analise`, e são os mesmos da tela: `embutido` é o
/// analisador do próprio aplicativo, `semgrep` é a ferramenta externa quando ela
/// existe nesta máquina, `ambos` roda os dois e guarda um achado por posição.
pub const NODE_ANALISADORES: [&str; 3] = ["embutido", "semgrep", "ambos"];
/// As severidades que uma análise devolve, da mais alta à mais baixa. É o token
/// que o semgrep usa, então a tela, o banco e a ferramenta dizem o mesmo nome.
pub const SEVERIDADES: [&str; 3] = ["ERROR", "WARNING", "INFO"];
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
    /// A operação de um nó `git`: `pull`, `checkout` ou `tag`.
    pub git_op: Option<String>,
    /// A referência do nó `git`: o ramo a alternar (`checkout`) ou a tag a criar
    /// (`tag`). Vazio em `pull`, que trabalha no ramo atual.
    pub git_ref: Option<String>,
    /// O instrumento de um nó `analise`: `embutido`, `semgrep` ou `ambos`. Vazio
    /// nos outros tipos, do mesmo jeito que a operação de git é vazia aqui.
    pub analyzer: String,
    /// A partir de qual severidade o nó `analise` trava a publicação. É o corte
    /// dele, não uma média: quem desenha decide o que barra.
    pub severity_gate: String,
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
    pub git_op: Option<String>,
    pub git_ref: Option<String>,
    pub analyzer: Option<String>,
    pub severity_gate: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub parallel: Option<bool>,
    pub timeout_secs: Option<i64>,
    pub retry_limit: Option<i64>,
    pub condition: Option<String>,
    pub requires_approval: Option<bool>,
    pub x: Option<i64>,
    pub y: Option<i64>,
}

/// A pasta de trabalho de um nó que roda nesta máquina: sempre um caminho
/// relativo dentro do projeto. Vale para o nó de comando local e para o nó git,
/// que é local por definição — a checagem é uma só porque a regra é uma só.
/// Vazio é "nenhuma pasta", que é a raiz do projeto.
fn conferir_pasta_local(dir: &str, rotulo: &str) -> Result<(), String> {
    if dir.is_empty() {
        return Ok(());
    }
    if dir.starts_with('/') || dir.contains('\\') || dir.contains("..") {
        return Err(format!(
            "{rotulo}: a pasta de trabalho deve ser um caminho relativo dentro do projeto"
        ));
    }
    Ok(())
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

    /// A operação declarada por um nó `git`, já normalizada. Os nós que não são
    /// `git` guardam isto vazio, e é assim que `checar_grafo` distingue um do outro.
    pub fn git_op(&self) -> String {
        self.git_op.as_deref().unwrap_or("").trim().to_lowercase()
    }

    pub fn depends_on(&self) -> Vec<String> {
        self.depends_on.clone().unwrap_or_default()
    }

    /// O instrumento declarado por um nó `analise`, já normalizado; o padrão é o
    /// analisador do aplicativo, porque ele é o único que existe em toda máquina.
    pub fn analyzer(&self) -> String {
        match self.analyzer.as_deref().map(str::trim) {
            None | Some("") => "embutido".to_string(),
            Some(nome) => nome.to_lowercase(),
        }
    }

    /// O corte de severidade do nó `analise`. Vazio é "não barra", porque um nó
    /// desenhado só para informar é uma escolha legítima de quem desenha.
    pub fn severity_gate(&self) -> String {
        self.severity_gate
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_uppercase())
            .unwrap_or_default()
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
            return Err(format!(
                "{rotulo}: o identificador do nó deve ter de 1 a 40 caracteres"
            ));
        }
        if !self
            .node_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "{rotulo}: o identificador aceita letra, número, hífen e sublinhado"
            ));
        }
        let title = self.title.trim();
        if title.is_empty() || title.chars().count() > 120 {
            return Err(format!("{rotulo}: dê um nome ao nó (até 120 caracteres)"));
        }
        let kind = self.kind();
        if !NODE_KINDS.contains(&kind.as_str()) {
            return Err(format!(
                "{rotulo}: tipo de nó inválido “{kind}”; use motor, comando, aprovacao, git ou analise"
            ));
        }
        match kind.as_str() {
            "motor" => {
                let Some(step) = self.engine_step else {
                    return Err(format!(
                        "{rotulo}: escolha qual das quinze etapas do motor ele roda"
                    ));
                };
                if !(1..=15).contains(&step) {
                    return Err(format!("{rotulo}: a etapa do motor vai de 1 a 15"));
                }
            }
            "comando" => {
                let executor = self.executor();
                if !NODE_EXECUTORS.contains(&executor.as_str()) {
                    return Err(format!(
                        "{rotulo}: executor inválido “{executor}”; use local ou ssh"
                    ));
                }
                let commands = self.commands();
                if commands.iter().all(|c| c.trim().is_empty()) {
                    return Err(format!(
                        "{rotulo}: um nó de comando precisa de pelo menos um comando"
                    ));
                }
                validate_list(&commands, &rotulo, 20)?;
                let dir = self.working_dir.as_deref().unwrap_or("").trim();
                if executor == "local" {
                    conferir_pasta_local(dir, &rotulo)?;
                } else if !dir.is_empty() && !is_safe_remote_path(dir) {
                    return Err(format!(
                        "{rotulo}: a pasta de trabalho no servidor precisa ser um caminho absoluto, como /srv/site"
                    ));
                }
            }
            "git" => {
                let op = self.git_op();
                if op.is_empty() {
                    return Err(format!(
                        "{rotulo}: o nó git precisa de uma operação: {}",
                        NODE_GIT_OPS.join(", ")
                    ));
                }
                if !NODE_GIT_OPS.contains(&op.as_str()) {
                    return Err(format!(
                        "{rotulo}: o nó git roda {}, não “{op}”",
                        NODE_GIT_OPS.join(", ")
                    ));
                }
                if !self.commands().is_empty() {
                    return Err(format!(
                        "{rotulo}: o nó git executa a operação escolhida, não comandos soltos"
                    ));
                }
                // O nó git roda nesta máquina, na pasta do projeto: a mesma contenção
                // de caminho do nó de comando local, porque é o mesmo risco.
                conferir_pasta_local(self.working_dir.as_deref().unwrap_or("").trim(), &rotulo)?;
                match op.as_str() {
                    // O pull trabalha no ramo que a pasta já está: sem branch e sem
                    // remoto para escolher, e o git responde pelo nó.
                    "pull" => {
                        if self
                            .git_ref
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .is_some()
                        {
                            return Err(format!(
                                "{rotulo}: o pull pega o ramo atual da pasta; a referência não é dele"
                            ));
                        }
                    }
                    // Alternar e marcar são a referência em si: sem ela o nó não tem
                    // o que fazer, e um nó que não faz nada não entra no grafo.
                    _ => {
                        let Some(referencia) = self.git_ref.as_deref().map(str::trim) else {
                            return Err(format!(
                                "{rotulo}: o nó git de “{op}” precisa da referência (o ramo, ou a tag)"
                            ));
                        };
                        if referencia.is_empty()
                            || referencia.len() > 200
                            || referencia.starts_with('-')
                            || referencia.contains("..")
                            || referencia.contains(char::is_whitespace)
                            || referencia.contains('\0')
                        {
                            return Err(format!(
                                "{rotulo}: referência de git inválida “{referencia}”"
                            ));
                        }
                    }
                }
            }
            "analise" => {
                let instrumento = self.analyzer();
                if !NODE_ANALISADORES.contains(&instrumento.as_str()) {
                    return Err(format!(
                        "{rotulo}: o nó de análise usa “{instrumento}”; use {}",
                        NODE_ANALISADORES.join(", ")
                    ));
                }
                let corte = self.severity_gate();
                if !corte.is_empty() && !SEVERIDADES.contains(&corte.as_str()) {
                    return Err(format!(
                        "{rotulo}: o corte de severidade aceita {}, não “{corte}”",
                        SEVERIDADES.join(", ")
                    ));
                }
                if !self.commands().is_empty() {
                    return Err(format!(
                        "{rotulo}: o nó de análise varre o código do projeto, não comandos soltos"
                    ));
                }
                // Lê o código desta máquina, na pasta do projeto: o executor é
                // sempre local e a contenção de caminho é a mesma do nó de comando.
                if self.executor() != "local" {
                    return Err(format!(
                        "{rotulo}: a análise de código lê a pasta do projeto nesta máquina — escolha o executor local"
                    ));
                }
                conferir_pasta_local(self.working_dir.as_deref().unwrap_or("").trim(), &rotulo)?;
                // O prazo do nó é o prazo da varredura, e a varredura tem o teto
                // dela: um nó desenhado com 21600 segundos falharia ao executar.
                let prazo = self.timeout_secs.unwrap_or(0);
                if prazo > 3600 {
                    return Err(format!(
                        "{rotulo}: o prazo da varredura vai de 0 (o padrão do aplicativo) a 3600 segundos"
                    ));
                }
            }
            _ => {}
        }
        let prazo = self.timeout_secs.unwrap_or(0);
        if !(0..=21_600).contains(&prazo) {
            return Err(format!(
                "{rotulo}: o prazo do nó vai de 0 (herdar o do modelo) a 21600 segundos"
            ));
        }
        let retry = self.retry_limit.unwrap_or(0);
        if !(0..=3).contains(&retry) {
            return Err(format!(
                "{rotulo}: a política de repetição aceita de 0 a 3 novas tentativas"
            ));
        }
        if let Some(raw) = self.condition.as_deref().map(str::trim) {
            if !raw.is_empty() {
                let (chave, valor) = raw.split_once('=').ok_or_else(|| {
                    format!("{rotulo}: a condição deve ser ramo=<nome> ou ambiente=<nome>")
                })?;
                if !["ramo", "ambiente"].contains(&chave.trim()) {
                    return Err(format!(
                        "{rotulo}: condição “{chave}” não existe; o motor conhece ramo e ambiente"
                    ));
                }
                if valor.trim().is_empty() || valor.contains('\n') || valor.len() > 120 {
                    return Err(format!(
                        "{rotulo}: a condição precisa de um valor entre 1 e 120 caracteres"
                    ));
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

// ---------- análise de código-fonte (plano CI/CD, etapa 5) ----------

/// Quantos achados de cada severidade uma varredura deixou. É a contagem do que
/// foi gravado, não uma estimativa: o corte da publicação compara este número
/// com o corte declarado no nó, e os dois vêm da mesma corrida.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisCounts {
    pub error: i64,
    pub warning: i64,
    pub info: i64,
}

impl AnalysisCounts {
    pub fn total(&self) -> i64 {
        self.error + self.warning + self.info
    }
}

/// Uma varredura de código como ela fica guardada. O código de saída da
/// ferramenta é uma coluna e nunca um veredito: medido no semgrep 1.181, uma
/// varredura completa pode voltar com achados e código 2 (um arquivo que ela não
/// interpretou) e com zero achados e código 0. O que barra é a contagem contra o
/// corte, e o que a ferramenta não conseguiu ler é dito como leitura falha.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRun {
    pub id: i64,
    pub project_id: i64,
    /// Nome do cadastro, para a tela não precisar de segunda consulta.
    pub project: String,
    /// A pasta varrida, já absoluta — é o que diz se a corrida foi do projeto
    /// inteiro ou de um subdiretório.
    pub root: String,
    /// O instrumento pedido: `embutido`, `semgrep` ou `ambos`.
    pub analyzer: String,
    /// O instrumento que de fato rodou, com o nome da ferramenta externa quando
    /// ela existia nesta máquina.
    pub tool: String,
    pub tool_version: Option<String>,
    /// As regras usadas, como identificadores separados por vírgula.
    pub rules: String,
    /// O corte de severidade desta corrida; vazio é "nada barra".
    pub severity_gate: String,
    /// `concluida`, `falhou` ou `interrompida`.
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: i64,
    pub files_scanned: i64,
    /// Arquivos que a ferramenta disse ter pulado (grandes demais, binários).
    pub files_skipped: i64,
    pub exit_code: Option<i64>,
    /// Quantos erros a ferramenta reportou fora dos achados.
    pub tool_errors: i64,
    /// Se a corrida barra, pela contagem contra o corte.
    pub blocked: bool,
    pub counts: AnalysisCounts,
    /// O resumo de uma varredura que não deu para confiar: prazo estourado,
    /// ferramenta ausente ou regras inválidas.
    pub note: Option<String>,
}

/// Um achado de análise. O `snippet` é a linha do arquivo **mascarada** por
/// [`crate::security::mascarar`]: o valor de um segredo nunca chega ao banco, à
/// tela nem ao pacote que uma IA lê.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFinding {
    pub id: i64,
    pub run_id: i64,
    pub rule_id: String,
    /// `embutido` ou `semgrep` — quem viu, porque o mesmo arquivo pode ter os
    /// dois instrumentos apontando a mesma linha.
    pub source: String,
    pub severity: String,
    /// Caminho relativo à raiz varrida, com barra.
    pub file: String,
    pub line_start: i64,
    pub line_end: i64,
    pub message: String,
    pub cwe: Option<String>,
    pub snippet: Option<String>,
    /// A identidade do achado entre corridas: regra, arquivo, mensagem e trecho,
    /// sem a linha. A linha muda quando o arquivo cresce; o defeito continua o
    /// mesmo, e é isso que permite dizer "novo", "persistente" e "corrigido".
    pub fingerprint: String,
}

/// O que a tela pede ao listar os achados de uma corrida. Todo campo é opcional:
/// vazio é "não filtra". A busca textual olha arquivo, regra, mensagem e trecho.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AchadosFiltro {
    pub severidade: Option<String>,
    pub origem: Option<String>,
    pub regra: Option<String>,
    pub busca: Option<String>,
    /// `novo` ou `persistente`, quando há corrida anterior comparável.
    pub estado: Option<String>,
    pub limite: Option<i64>,
    pub deslocamento: Option<i64>,
}

/// Um achado com o estado em relação à corrida anterior. `baseline` é `None`
/// quando não existe corrida comparável — e "sem base" não é "novo".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingView {
    #[serde(flatten)]
    pub achado: AnalysisFinding,
    pub baseline: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Faceta {
    pub valor: String,
    pub quantidade: i64,
}

/// As contagens da corrida inteira, antes de qualquer filtro. A tela usa isto
/// para oferecer os filtros com número, em vez de uma lista de valores sem peso.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsFacets {
    pub severidades: Vec<Faceta>,
    pub origens: Vec<Faceta>,
    pub regras: Vec<Faceta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsPage {
    pub itens: Vec<AnalysisFindingView>,
    /// Quantos achados atendem ao filtro, não só aos que cabem nesta página.
    pub total: i64,
    /// O total da corrida sem filtro, para a tela não confundir "filtrado" com "tudo".
    pub total_geral: i64,
    pub deslocamento: i64,
    pub limite: i64,
    pub facetas: FindingsFacets,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineRun {
    pub id: i64,
    pub started_at: String,
    pub tool: String,
    pub tool_version: Option<String>,
}

/// A comparação com a última corrida concluída do mesmo projeto e instrumento.
/// Só entra na conta uma corrida que leu arquivos: uma varredura interrompida
/// não é base para dizer que algo foi corrigido.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisComparison {
    pub baseline: Option<BaselineRun>,
    pub novos: i64,
    pub persistentes: i64,
    pub corrigidos: i64,
    /// Só uma corrida concluída e com leitura efetiva pode dizer que a ausência
    /// de um achado é correção; nas outras, `corrigidos` é zero e isto é falso.
    pub corrigidos_conclusivos: bool,
    /// Uma amostra dos corrigidos, para a revisão saber o que sumiu.
    pub corrigidos_itens: Vec<AnalysisFinding>,
}

/// O que a tela pede como varredura. `path` é um subdiretório opcional dentro do
/// projeto — a mesma contenção de caminho dos nós de comando locais.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisInput {
    pub project_id: i64,
    pub analyzer: Option<String>,
    pub severity_gate: Option<String>,
    pub path: Option<String>,
    pub timeout_secs: Option<i64>,
}

impl AnalysisInput {
    pub fn analyzer(&self) -> String {
        match self.analyzer.as_deref().map(str::trim) {
            None | Some("") => "embutido".to_string(),
            Some(nome) => nome.to_lowercase(),
        }
    }

    pub fn severity_gate(&self) -> String {
        self.severity_gate
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_uppercase())
            .unwrap_or_default()
    }

    /// Conferência do pedido, antes de qualquer arquivo ser aberto. Devolve a
    /// pasta relativa limpa (`None` = projeto inteiro).
    pub fn conferir(&self) -> Result<Option<String>, String> {
        if !NODE_ANALISADORES.contains(&self.analyzer().as_str()) {
            return Err(format!(
                "instrumento “{}” não existe; use {}",
                self.analyzer(),
                NODE_ANALISADORES.join(", ")
            ));
        }
        let corte = self.severity_gate();
        if !corte.is_empty() && !SEVERIDADES.contains(&corte.as_str()) {
            return Err(format!(
                "o corte de severidade aceita {}, não “{corte}”",
                SEVERIDADES.join(", ")
            ));
        }
        let prazo = self.timeout_secs.unwrap_or(0);
        if !(0..=3600).contains(&prazo) {
            return Err("o prazo da varredura vai de 0 (o padrão) a 3600 segundos".to_string());
        }
        let bruto = self.path.as_deref().map(str::trim).unwrap_or("");
        if bruto.is_empty() {
            return Ok(None);
        }
        // A mesma régua da pasta de trabalho de um nó local: é o mesmo risco, um
        // caminho que dobra para fora do que está cadastrado.
        conferir_pasta_local(bruto, "a varredura")?;
        Ok(Some(bruto.trim_end_matches('/').to_string()))
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
            runtime: None,
            image_name: None,
            dockerfile: None,
            compose_file: None,
            service_name: None,
            publish_ports: None,
        }
    }

    fn no_git(chave: &str, op: &str, referencia: Option<&str>) -> PipelineNodeInput {
        PipelineNodeInput {
            node_key: chave.into(),
            title: chave.into(),
            kind: "git".into(),
            engine_step: None,
            executor: None,
            environment: None,
            working_dir: None,
            commands: None,
            git_op: Some(op.into()),
            git_ref: referencia.map(str::to_string),
            analyzer: None,
            severity_gate: None,
            depends_on: None,
            parallel: None,
            timeout_secs: None,
            retry_limit: None,
            condition: None,
            requires_approval: None,
            x: None,
            y: None,
        }
    }

    fn no_analise(
        chave: &str,
        instrumento: Option<&str>,
        corte: Option<&str>,
    ) -> PipelineNodeInput {
        let mut no = no_git(chave, "pull", None);
        no.kind = "analise".into();
        no.git_op = None;
        no.git_ref = None;
        no.analyzer = instrumento.map(str::to_string);
        no.severity_gate = corte.map(str::to_string);
        no
    }

    #[test]
    fn o_no_git_so_aceita_as_operacoes_que_existem() {
        assert!(no_git("puxa", "pull", None).validate().is_ok());
        assert!(no_git("marca", "tag", Some("v1.2.0")).validate().is_ok());
        assert!(no_git("muda", "checkout", Some("main")).validate().is_ok());
        // um tipo que o motor não roda não entra no grafo
        let inventado = no_git("deplod", "deploy", Some("producao"))
            .validate()
            .unwrap_err();
        assert!(inventado.contains("não “deploy”"), "{inventado}");
        // alternar e marcar sem referência não têm o que fazer
        assert!(no_git("marca", "tag", None)
            .validate()
            .unwrap_err()
            .contains("referência"));
        let sem_operacao = no_git("perdido", "", Some("main")).validate().unwrap_err();
        assert!(
            sem_operacao.contains("precisa de uma operação"),
            "{sem_operacao}"
        );
        // o pull trabalha no ramo da pasta: referência sobrando é recusada
        let puxa_com_ramo = no_git("puxa", "pull", Some("main")).validate().unwrap_err();
        assert!(puxa_com_ramo.contains("ramo atual"), "{puxa_com_ramo}");
        // e um nó git não leva comandos: quem executa é a operação
        let mut com_comando = no_git("puxa", "pull", None);
        com_comando.commands = Some(vec!["git pull".into()]);
        assert!(com_comando
            .validate()
            .unwrap_err()
            .contains("comandos soltos"));
        // referência com o que o git não aceita nunca viria para cá
        for ruim in ["-t", "a..b", "dois ramos", "muito".repeat(50).as_str()] {
            let erro = no_git("marca", "tag", Some(ruim)).validate().unwrap_err();
            assert!(erro.contains("inválida"), "{ruim} passou: {erro}");
        }
        // a pasta do nó git é dentro do projeto: caminho absoluto ou com `..` não
        // entra na definição — é a mesma regra do nó de comando local.
        for fora in ["/etc", "..\\segredo", "C:\\Windows"] {
            let mut no = no_git("puxa", "pull", None);
            no.working_dir = Some(fora.into());
            let erro = no.validate().unwrap_err();
            assert!(
                erro.contains("relativo dentro do projeto"),
                "{fora} passou: {erro}"
            );
        }
        let mut ramal = no_git("puxa", "pull", None);
        ramal.working_dir = Some("apps/api".into());
        ramal.validate().unwrap();
        // o nó git mexe na pasta do projeto, não no destino da publicação
        assert!(!no_git("marca", "tag", Some("v1")).touches_destination());
    }

    #[test]
    fn o_no_de_analise_e_desenhado_so_com_o_que_o_motor_varre() {
        // sem instrumento e sem corte é a varredura padrão: embutido, informativa
        no_analise("varre", None, None).validate().unwrap();
        no_analise("varre", Some("Ambos"), Some("warning"))
            .validate()
            .unwrap();
        // um instrumento que não existe varreria nada e o nó aprovaria tudo
        let inventado = no_analise("varre", Some("bandit"), None)
            .validate()
            .unwrap_err();
        assert!(
            inventado.contains("embutido, semgrep, ambos"),
            "{inventado}"
        );
        // um corte inventado barraria a publicação para sempre
        let corte = no_analise("varre", None, Some("CRITICO"))
            .validate()
            .unwrap_err();
        assert!(corte.contains("ERROR, WARNING, INFO"), "{corte}");
        // o nó varre o código; comandos soltos não são o que ele executa
        let mut com_comando = no_analise("varre", None, None);
        com_comando.commands = Some(vec!["npm audit".into()]);
        assert!(com_comando
            .validate()
            .unwrap_err()
            .contains("comandos soltos"));
        // ele lê a pasta do projeto nesta máquina — um executor ssh leria o servidor
        let mut remoto = no_analise("varre", None, None);
        remoto.executor = Some("ssh".into());
        assert!(remoto.validate().unwrap_err().contains("executor local"));
        // e a pasta varrida é dentro do projeto, a mesma régua dos outros nós locais
        for fora in ["/etc", "..\\segredo"] {
            let mut no = no_analise("varre", None, None);
            no.working_dir = Some(fora.into());
            let erro = no.validate().unwrap_err();
            assert!(
                erro.contains("relativo dentro do projeto"),
                "{fora} passou: {erro}"
            );
        }
        // o prazo do nó é o prazo da varredura: acima do teto a varredura falharia
        let mut longo = no_analise("varre", None, None);
        longo.timeout_secs = Some(21_600);
        assert!(longo.validate().unwrap_err().contains("prazo da varredura"));
        longo.timeout_secs = Some(1_800);
        longo.validate().unwrap();
        // ler código-fonte não mexe no destino da publicação
        assert!(!no_analise("varre", None, None).touches_destination());
    }

    #[test]
    fn o_executor_do_modelo_e_conferido_pela_mesma_checagem_do_motor() {
        let mut d = entrada(Some(3), "artifacts", "/srv/api");
        d.runtime = Some("docker".into());
        // Docker sem imagem não tem o que construir
        assert!(d.validate().unwrap_err().contains("nome da imagem"));
        d.image_name = Some("meuapp".into());
        d.validate().unwrap();
        assert_eq!(d.alvo("/srv/api").imagem_tag(), "meuapp:atual");
        let mut caminho = d.clone();
        caminho.dockerfile = Some("/etc/Dockerfile".into());
        assert!(caminho.validate().unwrap_err().contains("caminho absoluto"));
        let mut porta = d.clone();
        porta.publish_ports = Some(vec!["8080:oitenta".into()]);
        assert!(porta.validate().unwrap_err().contains("porta"));

        let mut c = entrada(Some(3), "artifacts", "/srv/api");
        c.runtime = Some("compose".into());
        c.validate().unwrap(); // o arquivo tem default, o diretório é o destino
        let mut com_porta = c.clone();
        com_porta.publish_ports = Some(vec!["8080:80".into()]);
        assert!(com_porta
            .validate()
            .unwrap_err()
            .contains("arquivo do compose"));

        let mut p = entrada(Some(3), "artifacts", "/srv/api");
        p.runtime = Some("pm2".into());
        assert!(p.validate().unwrap_err().contains("pm2 list"));
        p.service_name = Some("api".into());
        p.validate().unwrap();

        // inventado não entra, e vazio continua sendo exatamente o que era antes
        let mut kubernetes = d.clone();
        kubernetes.runtime = Some("kubernetes".into());
        assert!(kubernetes.validate().unwrap_err().contains("desconhecido"));
        assert!(d.alvo("/srv/api").roteiro_build().is_some());
        let nenhum = entrada(Some(3), "artifacts", "/srv/api");
        assert!(nenhum.alvo("/srv/api").roteiro_build().is_none());
        assert!(nenhum.validate().is_ok());

        // Estratégia docker sem comando próprio continua pedindo comando — até o ponto
        // em que há um executor: aí quem constrói e sobe a imagem é o executor, e a
        // tela não pode recusar o que o motor executa.
        let mut sem_comando = entrada(Some(3), "docker", "/srv/api");
        sem_comando.build_commands = vec![];
        assert!(sem_comando
            .validate()
            .unwrap_err()
            .contains("comandos de build"));
        sem_comando.runtime = Some("docker".into());
        sem_comando.image_name = Some("meuapp".into());
        sem_comando.validate().unwrap();

        // Executor de runtime num modelo sem servidor: os roteiros são POSIX e rodam
        // no destino, e nesta máquina não há docker ni pm2 garantidos.
        let mut local = entrada(None, "artifacts", "C:/publicados/api");
        local.runtime = Some("pm2".into());
        local.service_name = Some("api".into());
        let erro = local.validate().unwrap_err();
        assert!(erro.contains("roda no servidor"), "{erro}");
        local.runtime = None;
        local.service_name = None;
        local.validate().unwrap();
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
        assert!(sem_comando
            .validate()
            .unwrap_err()
            .contains("pelo menos um comando"));

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
        assert!(entrada(Some(3), "artifacts", "/var/www/site")
            .validate()
            .is_ok());
        assert!(entrada(Some(3), "artifacts", "C:/publicados/site")
            .validate()
            .unwrap_err()
            .contains("caminho absoluto"));
    }
}
