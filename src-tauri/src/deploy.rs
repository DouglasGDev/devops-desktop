//! Motor de deploy: executa de verdade o pipeline de 15 etapas do capítulo 10.
//!
//! Nada aqui é simulado. Cada etapa chama o `ssh`/`sftp` nativo, o `git` local
//! ou o shell local, e o que aparece na interface é o código de saída real.
//! Quando uma etapa obrigatória falha, o pipeline para e a execução é
//! registrada como `failed` — nunca como sucesso parcial.
//!
//! Segredos: os valores das variáveis chegam por parâmetro da interface no
//! momento do deploy, vivem só na memória da thread do pipeline e vão ao
//! servidor dentro do script enviado pela stdin. Nenhum valor é gravado no
//! banco, em comando persistido ou em linha de log — tudo passa por
//! [`Secrets::mask`] antes de sair do motor.

use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{
    audit, auth,
    db::{self, DbHandle},
    files, git,
    models::{strategy_needs_server, validate_strategy, DeployProfile, DeployRun, Project, Server},
    security, ssh_test, sshexec,
};

/// Os 15 passos do pipeline, na ordem exata do capítulo 10.
pub const STEPS: [&str; 15] = [
    "Validar configurações e credenciais",
    "Testar conexão SSH",
    "Verificar branch e estado do Git",
    "Identificar arquivos alterados",
    "Verificar segredos e arquivos proibidos",
    "Executar testes, quando configurados",
    "Compilar o projeto, quando necessário",
    "Preparar a publicação",
    "Criar backup ou ponto de recuperação",
    "Enviar arquivos ou artefatos",
    "Executar comandos de atualização",
    "Reiniciar serviços, quando necessário",
    "Realizar testes de saúde",
    "Confirmar o resultado",
    "Registrar o histórico do deploy",
];

/// Limite da varredura local de arquivos.
const MAX_FILES: usize = 20_000;
/// Arquivos por sessão sftp: lotes curtos mantêm o script da stdin pequeno.
const UPLOAD_CHUNK: usize = 40;
/// Linhas de saída reaproveitadas no diagnóstico de uma etapa.
const MAX_TAIL_LINES: usize = 20;
/// Pasta dentro do destino local onde os pontos de recuperação ficam. É sempre
/// ignorada pela cópia, pela limpeza de excedentes e pela conferência final.
const BACKUPS_DIR: &str = ".devops-backups";
/// Chave de configuração com padrões sensíveis adicionais do usuário.
pub const SECRET_PATTERNS_SETTING: &str = "secret_patterns";

/// Nomes das duas etapas que mudam de figura quando não há servidor: a conexão
/// vira a pasta de destino e o envio vira cópia local. As outras treze descrevem
/// o mesmo trabalho nos dois modos.
pub const STEPS_LOCAIS: [(i64, &str); 2] = [
    (2, "Abrir a pasta de destino"),
    (10, "Copiar os arquivos para o destino"),
];

pub fn step_name(position: i64) -> String {
    STEPS
        .get((position - 1).clamp(0, 14) as usize)
        .copied()
        .unwrap_or("Etapa")
        .to_string()
}

pub fn step_name_local(position: i64) -> String {
    STEPS_LOCAIS
        .iter()
        .find(|(p, _)| *p == position)
        .map(|(_, nome)| nome.to_string())
        .unwrap_or_else(|| step_name(position))
}

// ---------- estado e eventos ----------

pub struct ActiveRun {
    label: String,
    cancel: Arc<AtomicBool>,
}

/// Execuções em andamento; é o que permite interromper o pipeline.
#[derive(Clone, Default)]
pub struct DeployState {
    runs: Arc<Mutex<HashMap<i64, ActiveRun>>>,
}

impl DeployState {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(&self, run_id: i64, label: String, cancel: Arc<AtomicBool>) -> Result<(), String> {
        self.runs
            .lock()
            .map_err(|_| "registrador de deploys indisponível".to_string())?
            .insert(
                run_id,
                ActiveRun { label, cancel },
            );
        Ok(())
    }

    fn cancel(&self, run_id: i64) -> Result<String, String> {
        let guard = self
            .runs
            .lock()
            .map_err(|_| "registrador de deploys indisponível".to_string())?;
        let run = guard
            .get(&run_id)
            .ok_or_else(|| "não há deploy em execução com esse identificador".to_string())?;
        run.cancel.store(true, Ordering::SeqCst);
        Ok(run.label.clone())
    }

    fn forget(&self, run_id: i64) {
        if let Ok(mut guard) = self.runs.lock() {
            guard.remove(&run_id);
        }
    }

    pub fn active(&self) -> Vec<(i64, String)> {
        self.runs
            .lock()
            .map(|g| g.iter().map(|(id, r)| (*id, r.label.clone())).collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    pub run_id: i64,
    pub position: i64,
    pub name: String,
    pub status: String,
    pub command: Option<String>,
    pub exit_code: Option<i64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEvent {
    pub run_id: i64,
    pub position: i64,
    pub stream: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveInfo {
    pub run_id: i64,
    pub label: String,
}

// ---------- peças puras ----------

/// Esconde valores sensíveis de qualquer texto que vá para log, banco ou UI.
pub fn redact(text: &str, values: &[&str]) -> String {
    let mut out = text.to_string();
    for value in values {
        if value.chars().count() < 3 {
            continue;
        }
        out = out.replace(value, "***");
    }
    out
}

/// Um arquivo local escolhido para publicação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    /// caminho relativo com `/` — é o mesmo caminho no servidor.
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
}

/// Padrão de inclusão/exclusão: caminho exato, prefixo de pasta ou sufixo
/// `*.ext`. Simples e previsível, sem glob escondido atrás da interface.
pub fn matches_pattern(rel: &str, raw: &str) -> bool {
    let pattern = normalise_rel(raw);
    if pattern.is_empty() {
        return false;
    }
    let rel = normalise_rel(rel);
    let name = rel.rsplit('/').next().unwrap_or(&rel);
    if let Some(suffix) = pattern.strip_prefix('*') {
        return !suffix.is_empty() && (name.ends_with(suffix) || rel.ends_with(suffix));
    }
    if let Some(head) = pattern.strip_suffix('*') {
        return !head.is_empty() && rel.starts_with(head);
    }
    rel == pattern || rel.starts_with(&format!("{pattern}/"))
}

/// Caminho ou padrão relativo no formato do script: barra à frente, sem `./`,
/// sem `/` sobrando no fim.
fn normalise_rel(raw: &str) -> String {
    raw.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn always_skipped(name: &str) -> bool {
    name == ".git"
}

/// Lista os arquivos de `root` que atendem a `include` (vazio = tudo, menos
/// `.git`) e não caem em `exclude`.
pub fn collect_files(
    root: &Path,
    include: &[String],
    exclude: &[String],
    max_files: usize,
) -> Result<Vec<LocalFile>, String> {
    if !root.is_dir() {
        return Err(format!("a pasta do projeto não existe: {}", root.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = fs::read_dir(&dir).map_err(|e| format!("falha ao ler {}: {e}", dir.display()))?;
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if always_skipped(&name) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() || exclude.iter().any(|p| matches_pattern(&rel, p)) {
                continue;
            }
            if !include.is_empty() && !include.iter().any(|p| matches_pattern(&rel, p)) {
                continue;
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            out.push(LocalFile { rel, abs: path, size });
            if out.len() > max_files {
                return Err(format!(
                    "mais de {max_files} arquivos selecionados — refine os caminhos a enviar"
                ));
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// Manifesto remoto: `caminho -> (crc, tamanho)` lido do `cksum` do servidor.
pub type RemoteManifest = HashMap<String, (u64, u64)>;

/// Interpreta a saída de `cksum` (`crc tamanho caminho`).
pub fn parse_manifest(raw: &str) -> RemoteManifest {
    let mut out = RemoteManifest::new();
    for line in raw.lines() {
        let mut parts = line.trim_end().splitn(3, ' ');
        let (Some(crc), Some(size), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(crc), Ok(size)) = (crc.trim().parse::<u64>(), size.trim().parse::<u64>()) else {
            continue;
        };
        let rel = path.trim().trim_start_matches("./").replace('\\', "/");
        if !rel.is_empty() {
            out.insert(rel, (crc, size));
        }
    }
    out
}

/// Arquivos que precisam mesmo ser enviados: tamanho ou conteúdo diferente, ou
/// ausentes no destino. O hash é cálculo local preguiçoso (só quando o tamanho
/// bate) para não ler o projeto inteiro à toa.
pub fn changed_files(
    local: &[LocalFile],
    manifest: &RemoteManifest,
    mut hash: impl FnMut(&LocalFile) -> Option<u64>,
) -> Vec<LocalFile> {
    local
        .iter()
        .filter(|f| match manifest.get(&f.rel) {
            None => true,
            Some((crc, size)) => {
                if size != &f.size {
                    return true;
                }
                match hash(f) {
                    Some(local_crc) => local_crc != *crc,
                    None => true,
                }
            }
        })
        .cloned()
        .collect()
}

/// CRC-32 de uma vez — serve de referência para o teste validar a versão em
/// blocos ([`crc32_feed`]/[`crc32_finish`], que é a usada na produção porque
/// nunca carrega o arquivo inteiro na memória).
#[cfg(test)]
pub fn crc32(bytes: &[u8]) -> u32 {
    !crc32_feed(0xFFFF_FFFF, bytes)
}

/// Alimenta o CRC-32 em blocos. O estado inicial é `0xFFFFFFFF` e o resultado
/// final passa por [`crc32_finish`].
fn crc32_feed(state: u32, bytes: &[u8]) -> u32 {
    let mut c = state;
    for &b in bytes {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c
}

fn crc32_finish(c: u32) -> u32 {
    !c
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

fn crc_of_file(path: &Path) -> Option<u64> {
    let arquivo = fs::File::open(path).ok()?;
    let mut leitor = io::BufReader::with_capacity(64 * 1024, arquivo);
    let mut estado = 0xFFFF_FFFFu32;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let lido = leitor.read(&mut buffer).ok()?;
        if lido == 0 {
            return Some(crc32_finish(estado) as u64);
        }
        estado = crc32_feed(estado, &buffer[..lido]);
    }
}

/// Junta caminho remoto sem duplicar barra.
pub fn remote_join(base: &str, rest: &str) -> String {
    let base = base.trim_end_matches('/');
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{rest}")
    }
}

/// Nome da release: carimbo UTC + id da execução.
pub fn release_name(stamp: &str, run_id: i64) -> String {
    let clean: String = stamp
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | 'T' | ':'))
        .collect();
    format!("releases/{clean}-{run_id}")
}

fn parse_marker(stdout: &str, marker: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix(marker)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// Últimas linhas úteis de um texto de erro.
fn tail(stderr: &str, stdout: &str) -> String {
    let source = if stderr.trim().is_empty() { stdout } else { stderr };
    let mut linhas = source
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_TAIL_LINES)
        .collect::<Vec<_>>();
    linhas.reverse();
    linhas.join(" | ")
}

/// Variáveis do deploy: apenas nomes são persistidos; os valores vivem aqui.
pub struct Secrets {
    items: Vec<(String, String)>,
}

impl Secrets {
    /// Aceita somente nomes declarados no modelo — a interface não pode injetar
    /// variáveis que o modelo não pediu.
    pub fn new(declared: &[String], provided: &HashMap<String, String>) -> Self {
        let items = declared
            .iter()
            .filter_map(|key| {
                provided
                    .get(key)
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .map(|v| (key.clone(), v))
            })
            .collect();
        Self { items }
    }

    /// Nomes declarados sem valor — a interface precisa pedir antes do deploy.
    pub fn missing(declared: &[String], provided: &HashMap<String, String>) -> Vec<String> {
        declared
            .iter()
            .filter(|key| {
                !provided
                    .get(*key)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// `export` para o topo do script remoto: valores entre aspas simples, só
    /// na stdin daquela execução.
    pub fn prelude(&self) -> String {
        self.items
            .iter()
            .map(|(key, value)| format!("export {key}={}\n", sshexec::shell_quote(value)))
            .collect()
    }

    /// Variáveis para o ambiente de um processo local. No remoto os mesmos
    /// valores entram pelo `export` do script; aqui eles nunca aparecem na linha
    /// de comando, que é o que a interface mostra.
    pub fn env(&self) -> Vec<(String, String)> {
        self.items.clone()
    }

    pub fn mask(&self, text: &str) -> String {
        let values: Vec<&str> = self.items.iter().map(|(_, v)| v.as_str()).collect();
        audit::scrub_secrets(&redact(text, &values))
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }
}

/// Resultado de uma etapa para a interface.
pub struct Info {
    pub message: String,
    pub skipped: bool,
}

impl Info {
    pub fn done(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            skipped: false,
        }
    }

    pub fn skip(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            skipped: true,
        }
    }
}

struct Fail {
    message: String,
    command: Option<String>,
    exit_code: Option<i64>,
    cancelled: bool,
}

impl Fail {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            command: None,
            exit_code: None,
            cancelled: false,
        }
    }

    fn failed(command: impl Into<String>, exit_code: Option<i64>, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            command: Some(command.into()),
            exit_code,
            cancelled: false,
        }
    }

    fn cancelled() -> Self {
        Self {
            message: "interrompido pelo usuário".into(),
            command: None,
            exit_code: None,
            cancelled: true,
        }
    }
}

/// Checagem de segredos sobre a lista atual de arquivos. Devolve os achados
/// para a etapa 5 não varrer o projeto duas vezes.
fn gate(ctx: &mut Ctx, position: i64) -> Result<Vec<security::Finding>, Fail> {
    let found = secret_findings(ctx);
    let bloqueantes: Vec<&security::Finding> = found
        .iter()
        .filter(|f| f.severity == "block")
        .collect();
    if bloqueantes.is_empty() {
        return Ok(found);
    }
    let lista = bloqueantes
        .iter()
        .take(8)
        .map(|f| format!("{} ({})", f.path, f.reason))
        .collect::<Vec<_>>()
        .join("; ");
    if ctx.allow_secrets {
        ctx.log(
            position,
            "out",
            &format!(
                "{} item(ns) sensível(is) mantidos por confirmação explícita: {lista}",
                bloqueantes.len()
            ),
        );
        return Ok(found);
    }
    Err(Fail::new(format!(
        "{} arquivo(s) com segredo seriam publicados: {lista} — exclua os caminhos no modelo ou refaça o deploy confirmando explicitamente",
        bloqueantes.len()
    )))
}

fn secret_findings(ctx: &Ctx) -> Vec<security::Finding> {
    if ctx.files.is_empty() {
        return Vec::new();
    }
    let rels: Vec<String> = ctx.files.iter().map(|f| f.rel.clone()).collect();
    let mut found = security::check_paths(&ctx.root, &rels, &ctx.extra);
    if ctx.project.is_git_repo {
        if let Ok(tracked) = security::tracked_sensitive(&ctx.root, &ctx.extra) {
            found.extend(tracked);
        }
    }
    security::summarize(found)
}

// ---------- contexto ----------

struct Ctx {
    app: AppHandle,
    db: DbHandle,
    run_id: i64,
    cancel: Arc<AtomicBool>,
    touched: AtomicU16,
    profile: DeployProfile,
    /// `None` é o modo sem servidor: o destino é uma pasta desta máquina e todos
    /// os comandos rodam aqui. As etapas remotas têm uma variante local cada.
    server: Option<Server>,
    project: Project,
    root: PathBuf,
    broker: auth::AuthBroker,
    provided: HashMap<String, String>,
    extra: Vec<String>,
    allow_secrets: bool,
    askpass: Option<auth::AskPass>,
    secrets: Secrets,
    shell: String,
    files: Vec<LocalFile>,
    /// Comparação origem↔destino já feita, guardada entre as etapas de backup e
    /// de cópia para o mesmo conteúdo ser lido uma única vez.
    plano: Option<(Vec<LocalFile>, usize)>,
    release: Option<String>,
    previous: Option<String>,
    tools: HashMap<String, Option<String>>,
    branch: String,
    remote_url: String,
    sent: usize,
    unchanged: usize,
    removed: usize,
    backup: Option<String>,
    confirmation: String,
}

impl Ctx {
    fn timeout(&self) -> Duration {
        Duration::from_secs(self.profile.step_timeout_secs.max(10) as u64)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn local_mode(&self) -> bool {
        self.server.is_none()
    }

    fn server_ref(&self) -> Result<&Server, Fail> {
        self.server
            .as_ref()
            .ok_or_else(|| Fail::new("este modelo publica nesta máquina e não tem servidor para contatar"))
    }

    fn has(&self, tool: &str) -> bool {
        self.tools.get(tool).and_then(|v| v.as_ref()).is_some()
    }

    fn base(&self) -> String {
        self.profile
            .remote_path
            .trim()
            .trim_end_matches('/')
            .trim_end_matches('\\')
            .to_string()
    }

    /// O destino como caminho do sistema de arquivos. No modo local é para onde
    /// a cópia vai; no remoto a string continua sendo o caminho POSIX no servidor.
    fn destino_path(&self) -> PathBuf {
        PathBuf::from(self.base())
    }

    /// Grava e publica o estado de uma etapa, mascarando qualquer segredo.
    fn mark(&self, position: i64, status: &str, message: Option<&str>, command: Option<&str>, exit_code: Option<i64>) {
        self.touched.fetch_or(1u16 << position, Ordering::Relaxed);
        let message = message.map(|m| self.secrets.mask(m));
        let command = command.map(|c| self.secrets.mask(c));
        let name = if self.local_mode() {
            step_name_local(position)
        } else {
            step_name(position)
        };
        if let Ok(conn) = self.db.lock() {
            if let Err(e) = db::upsert_step(
                &conn,
                &db::StepUpdate {
                    run_id: self.run_id,
                    position,
                    name: &name,
                    status,
                    command: command.as_deref(),
                    exit_code,
                    message: message.as_deref(),
                },
            ) {
                // Etapa sem registro no histórico é dado perdido: mostra na tela.
                self.log(position, "err", &format!("falha ao registrar a etapa: {e}"));
            }
        }
        let _ = self.app.emit(
            "deploy-step",
            StepEvent {
                run_id: self.run_id,
                position,
                name,
                status: status.to_string(),
                command,
                exit_code,
                message,
            },
        );
    }

    fn log(&self, position: i64, stream: &str, text: &str) {
        let masked = self.secrets.mask(text);
        for line in masked.lines().take(200) {
            if line.trim().is_empty() {
                continue;
            }
            let _ = self.app.emit(
                "deploy-log",
                LogEvent {
                    run_id: self.run_id,
                    position,
                    stream: stream.to_string(),
                    text: line.to_string(),
                },
            );
        }
    }

    fn remote(&mut self, position: i64, script: &str, label: &str) -> Result<sshexec::ExecResult, Fail> {
        let shell = self.shell.clone();
        self.remote_with(position, &shell, script, label)
    }

    fn remote_with(
        &mut self,
        position: i64,
        shell: &str,
        script: &str,
        label: &str,
    ) -> Result<sshexec::ExecResult, Fail> {
        if self.cancelled() {
            return Err(Fail::cancelled());
        }
        let servidor = self.server_ref()?.clone();
        let timeout = self.timeout();
        let result = sshexec::run_script(
            &servidor,
            self.askpass.as_ref(),
            shell,
            script,
            &[],
            timeout,
            &self.cancel,
        )
        .map_err(Fail::new)?;
        self.log(position, "out", &result.stdout);
        self.log(position, "err", &result.stderr);
        if !result.ok() {
            let detail = tail(&result.stderr, &result.stdout);
            let message = if detail.is_empty() {
                result.summary()
            } else {
                format!("{} — {detail}", result.summary())
            };
            return Err(Fail::failed(
                format!("{label} [ssh {}]", servidor.name),
                Some(result.exit_code.unwrap_or(-1) as i64),
                message,
            ));
        }
        Ok(result)
    }

    fn local(&mut self, position: i64, command: &str) -> Result<sshexec::ExecResult, Fail> {
        if self.cancelled() {
            return Err(Fail::cancelled());
        }
        let timeout = self.timeout();
        let env = self.secrets.env();
        let result =
            sshexec::run_local_with_env(command, &self.root, &env, timeout, &self.cancel)
                .map_err(Fail::new)?;
        self.log(position, "out", &result.stdout);
        self.log(position, "err", &result.stderr);
        if !result.ok() {
            let detail = tail(&result.stderr, &result.stdout);
            return Err(Fail::failed(
                command.to_string(),
                Some(result.exit_code.unwrap_or(-1) as i64),
                detail,
            ));
        }
        Ok(result)
    }

    /// Upload em lote numa única sessão sftp, com prazo e cancelamento.
    fn upload(&mut self, pares: &[(PathBuf, String)], label: &str) -> Result<(), Fail> {
        if self.cancelled() {
            return Err(Fail::cancelled());
        }
        let servidor = self.server_ref()?.clone();
        let timeout = self.timeout();
        files::put_chunk(
            &servidor,
            pares,
            self.askpass.as_ref(),
            timeout,
            &self.cancel,
        )
        .map_err(|e| Fail::failed(label.to_string(), None, e))
    }

    /// Script POSIX com falha rápida e as variáveis do deploy.
    fn script(&self, body: &str) -> String {
        format!("set -eu\n{}\n{body}", self.secrets.prelude())
    }

    fn run_commands(&mut self, position: i64, label: &str, commands: &[String]) -> Result<usize, Fail> {
        let linhas: Vec<&str> = commands
            .iter()
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .collect();
        if linhas.is_empty() {
            return Ok(0);
        }
        if self.local_mode() {
            // Sem servidor o lote roda no interpretador desta máquina, um comando
            // de cada vez: não existe o `set -e` de um script POSIX aqui.
            for linha in &linhas {
                self.log(position, "out", &format!("+ {linha}"));
                self.local(position, linha)?;
            }
            return Ok(linhas.len());
        }
        let mut script = self.script(&format!("# {label}\n"));
        for line in &linhas {
            // O eco do comando passa por aspas: apóstrofo no comando não pode
            // abrir nem fechar nada no script.
            script.push_str(&format!("echo {}\n{line}\n", sshexec::shell_quote(&format!("+ {line}"))));
        }
        self.remote(position, &script, &format!("{label} ({} comando(s))", linhas.len()))?;
        Ok(linhas.len())
    }
}

// ---------- orquestração ----------

fn step<F>(ctx: &mut Ctx, position: i64, body: F) -> Result<(), Fail>
where
    F: FnOnce(&mut Ctx) -> Result<Info, Fail>,
{
    if ctx.cancelled() {
        return Err(Fail::cancelled());
    }
    ctx.mark(position, "running", Some("em execução"), None, None);
    match body(ctx) {
        Ok(info) => {
            let status = if info.skipped { "skipped" } else { "done" };
            ctx.mark(position, status, Some(&info.message), None, None);
            Ok(())
        }
        Err(fail) => {
            ctx.mark(
                position,
                "failed",
                Some(&fail.message),
                fail.command.as_deref(),
                fail.exit_code,
            );
            Err(fail)
        }
    }
}

fn pipeline(ctx: &mut Ctx) -> Result<String, Fail> {
    step(ctx, 1, validate_step)?;
    step(ctx, 2, connection_step)?;
    step(ctx, 3, git_step)?;
    step(ctx, 4, files_step)?;
    step(ctx, 5, secrets_step)?;
    step(ctx, 6, tests_step)?;
    step(ctx, 7, build_step)?;
    step(ctx, 8, prepare_step)?;
    step(ctx, 9, backup_step)?;
    step(ctx, 10, send_step)?;
    step(ctx, 11, update_step)?;
    step(ctx, 12, restart_step)?;
    step(ctx, 13, health_step)?;
    step(ctx, 14, confirm_step)?;
    let confirmado = ctx.confirmation.clone();
    step(ctx, 15, register_step)?;
    Ok(confirmado)
}

fn rollback_pipeline(ctx: &mut Ctx) -> Result<String, Fail> {
    step(ctx, 1, validate_step)?;
    step(ctx, 2, connection_step)?;
    for position in 3..=10 {
        ctx.mark(position, "skipped", Some("não se aplica ao rollback"), None, None);
    }
    step(ctx, 11, rollback_switch)?;
    step(ctx, 12, restart_step)?;
    step(ctx, 13, health_step)?;
    step(ctx, 14, confirm_step)?;
    let confirmado = ctx.confirmation.clone();
    step(ctx, 15, register_step)?;
    Ok(confirmado)
}

// ---------- etapas ----------

fn validate_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return validate_local(ctx);
    }
    validate_remoto(ctx)
}

/// O que as duas formas de publicar têm em comum: a pasta do projeto ainda existe
/// e nenhuma variável declarada ficou sem valor.
fn checar_pasta_e_variaveis(ctx: &mut Ctx) -> Result<(), Fail> {
    if !ctx.root.is_dir() {
        return Err(Fail::new(format!(
            "a pasta local do projeto não existe mais: {}",
            ctx.root.display()
        )));
    }
    let faltando = Secrets::missing(&ctx.profile.secret_keys, &ctx.provided);
    if !faltando.is_empty() {
        return Err(Fail::new(format!(
            "informe o valor das variáveis do deploy: {}",
            faltando.join(", ")
        )));
    }
    ctx.secrets = Secrets::new(&ctx.profile.secret_keys, &ctx.provided);
    Ok(())
}

fn validate_remoto(ctx: &mut Ctx) -> Result<Info, Fail> {
    validate_strategy(&ctx.profile.strategy).map_err(Fail::new)?;
    if !ctx.profile.remote_path.trim().starts_with('/') {
        return Err(Fail::new("o diretório remoto precisa ser um caminho absoluto"));
    }
    checar_pasta_e_variaveis(ctx)?;
    let servidor = ctx.server_ref()?;
    let sem_destino = servidor.host.trim().is_empty()
        && servidor.ssh_alias.as_deref().unwrap_or("").trim().is_empty();
    if sem_destino {
        return Err(Fail::new(format!(
            "o servidor “{}” está sem host e sem alias",
            servidor.name
        )));
    }
    // Cada estratégia tem o seu requisito mínimo; o resto cai no `_ => {}` e é
    // validado pelas etapas seguintes.
    match ctx.profile.strategy.as_str() {
        "git" if ctx.profile.git_branch.as_deref().unwrap_or("").trim().is_empty() => {
            return Err(Fail::new("o deploy por Git exige a branch de origem"));
        }
        "script" if ctx.profile.prepare_commands.iter().all(|c| c.trim().is_empty()) => {
            return Err(Fail::new("o deploy por script precisa de pelo menos um comando"));
        }
        "docker" if ctx.profile.build_commands.is_empty() && ctx.profile.restart_commands.is_empty() => {
            return Err(Fail::new(
                "o deploy com Docker precisa de comandos de build ou de atualização",
            ));
        }
        _ => {}
    }

    if ctx.askpass.is_none() {
        let servidor = ctx.server_ref()?.clone();
        ctx.askpass = auth::prepare(&ctx.broker, &ctx.app, &servidor, &format!("Deploy · {}", ctx.profile.name))
            .map_err(Fail::new)?;
    }
    let servidor = ctx.server_ref()?;
    let auth_desc = match &ctx.askpass {
        Some(_) => "senha informada agora (não gravada)".to_string(),
        None if servidor.auth_method == "agent" => "SSH Agent".to_string(),
        None => servidor
            .identity_file
            .as_deref()
            .map(|f| format!("chave {f}"))
            .unwrap_or_else(|| "chave padrão do OpenSSH".to_string()),
    };
    Ok(Info::done(format!(
        "modelo “{}” · {} · destino {} · autenticação por {} · {} variável(is)",
        ctx.profile.name,
        ctx.profile.strategy,
        ctx.base(),
        auth_desc,
        ctx.secrets.count()
    )))
}

/// Modelo sem servidor: tudo acontece nesta máquina, então o que se confere aqui
/// é a pasta de destino e o perigo de ela bater no próprio projeto.
fn validate_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    validate_strategy(&ctx.profile.strategy).map_err(Fail::new)?;
    if strategy_needs_server(&ctx.profile.strategy) {
        return Err(Fail::new(format!(
            "a estratégia {} publica em um servidor, e este modelo não tem servidor. Escolha script ou artefatos.",
            ctx.profile.strategy
        )));
    }
    checar_pasta_e_variaveis(ctx)?;
    let destino = ctx.destino_path();
    if !destino.is_absolute() {
        return Err(Fail::new(format!(
            "a pasta de destino precisa ser um caminho absoluto, e “{}” não é",
            ctx.base()
        )));
    }
    if ctx.profile.strategy == "script"
        && ctx.profile.prepare_commands.iter().all(|c| c.trim().is_empty())
        && ctx.profile.build_commands.iter().all(|c| c.trim().is_empty())
        && ctx.profile.restart_commands.iter().all(|c| c.trim().is_empty())
    {
        return Err(Fail::new(
            "um deploy por script nesta máquina precisa de pelo menos um comando de preparação, build ou reinicialização",
        ));
    }
    // Copiar o projeto para dentro dele mesmo (ou apagar o projeto ao "limpar o
    // destino") é o único jeito de perder trabalho com um modelo sem servidor.
    let origem = ctx.root.canonicalize().unwrap_or_else(|_| ctx.root.clone());
    let alvo = destino.canonicalize().unwrap_or_else(|_| destino.clone());
    if alvo == origem || origem.starts_with(&alvo) {
        return Err(Fail::new(format!(
            "a pasta de destino ({}) é o projeto ou um lugar de onde ele estaria dentro — escolha uma pasta separada",
            ctx.base()
        )));
    }
    // O contrário também atrapalha: destino dentro do projeto significaria copiar
    // arquivos para dentro da própria seleção, e a limpeza do destino apagaria
    // arquivos do projeto.
    if alvo.starts_with(&origem) {
        return Err(Fail::new(format!(
            "a pasta de destino ({}) está dentro do projeto ({}) — publique em uma pasta fora do código-fonte",
            ctx.base(),
            origem.display()
        )));
    }
    Ok(Info::done(format!(
        "modelo “{}” · {} · destino {} · nesta máquina, sem SSH · {} variável(is)",
        ctx.profile.name,
        ctx.profile.strategy,
        ctx.base(),
        ctx.secrets.count()
    )))
}

fn connection_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return destino_step(ctx);
    }
    let servidor = ctx.server_ref()?.clone();
    let identity = servidor.identity_file.clone();
    let alias = servidor.ssh_alias.clone();
    let result = ssh_test::test(
        alias.as_deref(),
        &servidor.host,
        servidor.port,
        &servidor.username,
        identity.as_deref(),
        ctx.askpass.as_ref(),
    );
    ctx.log(2, "out", &result.message);
    if !result.ok {
        return Err(Fail::failed(
            format!("ssh {}", servidor.name),
            result.exit_code,
            format!(
                "{} — sem conexão verificada a publicação não continua",
                result.message
            ),
        ));
    }
    Ok(Info::done(format!(
        "{} ({} ms no servidor “{}”)",
        result.message, result.duration_ms, servidor.name
    )))
}

/// Etapa 2 do modo sem servidor: a pasta de destino precisa existir e aceitar
/// escrita. Se não existir ainda, ela é criada — é o que o `mkdir -p` remoto faz.
fn destino_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    let destino = ctx.destino_path();
    if destino.exists() {
        if !destino.is_dir() {
            return Err(Fail::new(format!(
                "o destino {} existe mas não é uma pasta",
                destino.display()
            )));
        }
    } else {
        fs::create_dir_all(&destino).map_err(|e| {
            Fail::new(format!(
                "não foi possível criar a pasta de destino {}: {e}",
                destino.display()
            ))
        })?;
    }
    // Escrever e apagar um arquivo de teste é a única prova de que ali se publica
    // — checar permissão de pasta no Windows não diz nada confiável.
    let prova = destino.join(".devops-permissao.tmp");
    let gravado = fs::write(&prova, b"devops").map_err(|e| {
        Fail::new(format!(
            "a pasta de destino {} não aceita escrita: {e}",
            destino.display()
        ))
    });
    match gravado {
        Ok(()) => {
            let _ = fs::remove_file(&prova);
        }
        Err(fail) => return Err(fail),
    }
    if ctx.profile.backup_enabled {
        fs::create_dir_all(destino.join(BACKUPS_DIR)).map_err(|e| {
            Fail::new(format!(
                "não foi possível preparar a pasta de backup dentro do destino: {e}"
            ))
        })?;
    }
    Ok(Info::done(format!(
        "destino {} acessível nesta máquina, sem SSH",
        destino.display()
    )))
}

fn git_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if !ctx.project.is_git_repo {
        return Ok(Info::skip("a pasta local não é repositório Git — nada a verificar"));
    }
    let status = git::status(&ctx.root).map_err(Fail::new)?;
    ctx.branch = status
        .branch
        .clone()
        .or_else(|| ctx.project.git_branch.clone())
        .unwrap_or_else(|| "(detached)".to_string());
    ctx.remote_url = ctx.project.git_remote.clone().unwrap_or_default();
    let alterados = status.files.len();
    let por_enviar = format!(
        "branch {} · {} arquivo(s) com mudança local · upstream +{}/-{}",
        ctx.branch, alterados, status.ahead, status.behind
    );

    if ctx.profile.strategy != "git" {
        return Ok(Info::done(por_enviar));
    }
    let alvo = ctx.profile.git_branch.as_deref().unwrap_or("").trim();
    if ctx.branch != alvo {
        return Err(Fail::new(format!(
            "a branch local é “{}” e o modelo publica “{alvo}” — troque a branch no Git ou ajuste o modelo",
            ctx.branch
        )));
    }
    if ctx.remote_url.is_empty() {
        return Err(Fail::new(
            "o projeto não tem repositório remoto; o deploy por Git exige um origin para o servidor buscar",
        ));
    }
    if status.ahead > 0 {
        return Err(Fail::new(format!(
            "{por_enviar} — publique os {} commit(s) com push antes do deploy por Git, senão o servidor recebe código antigo",
            status.ahead
        )));
    }
    let aviso = if alterados > 0 {
        " (mudanças não commitadas não chegam ao servidor: ele busca do repositório)"
    } else {
        ""
    };
    Ok(Info::done(format!("{por_enviar}{aviso}")))
}

fn files_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    let envia_arquivos = matches!(
        ctx.profile.strategy.as_str(),
        "sftp" | "artifacts" | "docker"
    );
    if !envia_arquivos {
        return Ok(Info::skip(match ctx.profile.strategy.as_str() {
            "git" => "no deploy por Git o servidor busca do repositório; nenhum arquivo local sobe",
            _ => "deploy por script não envia arquivos do disco local",
        }));
    }
    if ctx.profile.strategy == "docker" && ctx.profile.include_paths.is_empty() {
        return Ok(Info::skip(
            "Docker sem caminhos de contexto: o build usa os arquivos que já estão no servidor",
        ));
    }
    let lista = collect_files(
        &ctx.root,
        &ctx.profile.include_paths,
        &ctx.profile.exclude_paths,
        MAX_FILES,
    )
    .map_err(Fail::new)?;
    if lista.is_empty() {
        // Nesta máquina o caminho declarado pode ser exatamente o que a preparação ou o
        // build ainda vai gerar — as etapas 7 e 8 reescaneiam. Segurar o erro até lá dá
        // ao comando a chance de produzir o artefato; se não produzir, ele falha na etapa
        // dele com a mesma indicação de caminho.
        let vai_gerar = ctx.local_mode()
            && ctx
                .profile
                .prepare_commands
                .iter()
                .chain(ctx.profile.build_commands.iter())
                .any(|c| !c.trim().is_empty());
        if !vai_gerar {
            return Err(Fail::new(
                "nenhum arquivo corresponde aos caminhos configurados — revise inclusão e exclusão",
            ));
        }
        ctx.files = lista;
        return Ok(Info::done(
            "nenhum arquivo existe ainda nos caminhos configurados — a preparação ou o build desta máquina pode gerá-los",
        ));
    }
    let bytes: u64 = lista.iter().map(|f| f.size).sum();
    ctx.files = lista;
    Ok(Info::done(format!(
        "{} arquivo(s) selecionados, {} a comparar com o destino",
        ctx.files.len(),
        human_bytes(bytes)
    )))
}

fn secrets_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.files.is_empty() {
        return Ok(Info::skip("não há arquivos locais para verificar nesta estratégia"));
    }
    let found = gate(ctx, 5)?;
    let avisos = found.iter().filter(|f| f.severity == "warn").count();
    Ok(Info::done(format!(
        "nenhum segredo bloqueante entre {} arquivo(s){}",
        ctx.files.len(),
        if avisos > 0 {
            format!(" · {avisos} aviso(s) de cache/dependência rastreados")
        } else {
            String::new()
        }
    )))
}

fn tests_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.profile.test_commands.is_empty() {
        return Ok(Info::skip("nenhum comando de teste configurado no modelo"));
    }
    let n = ctx.profile.test_commands.len();
    for i in 0..n {
        let command = ctx.profile.test_commands[i].clone();
        ctx.local(6, &command)?;
    }
    Ok(Info::done(format!("{n} teste(s) local(is) passaram")))
}

fn build_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    let build_local = matches!(ctx.profile.strategy.as_str(), "sftp" | "artifacts");
    if ctx.profile.build_commands.is_empty() {
        return Ok(Info::skip(if build_local {
            "sem comandos de build — os arquivos vão como estão"
        } else {
            "o build acontece no servidor, na etapa de atualização"
        }));
    }
    if !build_local {
        return Ok(Info::skip(
            "nesta estratégia os comandos de build rodam no servidor (etapa 11)",
        ));
    }
    let n = ctx.profile.build_commands.len();
    for i in 0..n {
        let command = ctx.profile.build_commands[i].clone();
        ctx.local(7, &command)?;
    }
    // O build mudou o conteúdo: refaz a seleção e a checagem de segredos antes
    // de qualquer envio. Sem servidor isso vale também quando a seleção está vazia
    // ("projeto inteiro"): o artefato recém-gerado não estava na varredura da etapa 4.
    if !ctx.profile.include_paths.is_empty() || ctx.local_mode() {
        let lista = collect_files(
            &ctx.root,
            &ctx.profile.include_paths,
            &ctx.profile.exclude_paths,
            MAX_FILES,
        )
        .map_err(Fail::new)?;
        if lista.is_empty() && !ctx.profile.include_paths.is_empty() {
            return Err(Fail::new(
                "depois do build não sobrou nenhum arquivo para enviar — confira os caminhos",
            ));
        }
        ctx.files = lista;
        gate(ctx, 7)?;
        return Ok(Info::done(format!(
            "{n} comando(s) de build local(is) · artefatos: {} arquivo(s)",
            ctx.files.len()
        )));
    }
    Ok(Info::done(format!("{n} comando(s) de build local(is)")))
}

fn prepare_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return prepare_local(ctx);
    }
    let strategy = ctx.profile.strategy.as_str();
    let precisa_git = strategy == "git";
    let precisa_docker = strategy == "docker";
    let precisa_compose = ctx
        .profile
        .build_commands
        .iter()
        .chain(ctx.profile.restart_commands.iter())
        .any(|c| c.contains("docker-compose"));
    let precisa_cksum =
        matches!(strategy, "sftp" | "artifacts" | "docker") && !ctx.profile.releases_enabled && !ctx.files.is_empty();

    // A sonda roda em `sh`: é o único shell garantido antes de sabermos o que o
    // servidor tem.
    let probe = sshexec::probe_script(precisa_git, precisa_docker, precisa_compose, false, precisa_cksum);
    let sonda = ctx.remote_with(8, "sh", &probe, "verificação de requisitos")?;
    ctx.tools = sshexec::parse_probe(&sonda.stdout).into_iter().collect();
    ctx.shell = if ctx.has("bash") { "bash" } else { "sh" }.to_string();

    let mut faltam: Vec<&str> = Vec::new();
    if precisa_git && !ctx.has("git") {
        faltam.push("git");
    }
    if precisa_docker && !ctx.has("docker") {
        faltam.push("docker");
    }
    if precisa_compose && !ctx.has("docker-compose") {
        faltam.push("docker-compose");
    }
    if ctx.profile.backup_enabled && !ctx.has("tar") {
        faltam.push("tar (necessário para o backup)");
    }
    if precisa_cksum && !ctx.has("cksum") {
        faltam.push("cksum (comparação de conteúdo)");
    }
    if !faltam.is_empty() {
        return Err(Fail::new(format!(
            "o servidor não tem: {} — instale a ferramenta ou escolha outra estratégia",
            faltam.join(", ")
        )));
    }
    let resumo: Vec<String> = ctx
        .tools
        .iter()
        .map(|(name, path)| if path.is_some() { format!("{name} ✓") } else { format!("{name} ✗") })
        .collect();
    ctx.log(
        8,
        "out",
        &format!("shell remoto: {} · {}", ctx.shell, resumo.join(" ")),
    );

    let base = ctx.base();
    let mut script = String::from("set -eu\n");
    script.push_str(&format!("mkdir -p {}\n", sshexec::shell_quote(&base)));
    if ctx.profile.releases_enabled {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let nome = release_name(&stamp, ctx.run_id);
        ctx.release = Some(nome.clone());
        script.push_str(&format!(
            "mkdir -p {}\n",
            sshexec::shell_quote(&remote_join(&base, &nome))
        ));
    }
    ctx.remote(8, &script, "criar diretórios")?;

    if ctx.profile.releases_enabled {
        let ler = ctx.script(&format!(
            "cd {} 2>/dev/null || true\nprintf 'PREV:%s\\n' \"$(readlink current 2>/dev/null || true)\"\n",
            sshexec::shell_quote(&base)
        ));
        let atual = ctx.remote(8, &ler, "ler publicação ativa")?;
        ctx.previous = parse_marker(&atual.stdout, "PREV:");
    }

    let total = ctx.profile.prepare_commands.len();
    let executados = ctx.run_commands(8, "preparação", &ctx.profile.prepare_commands.clone())?;
    Ok(Info::done(format!(
        "{} pronto · shell {} · {}/{total} comando(s) de preparação",
        base,
        ctx.shell,
        executados,
    )))
}

/// Etapa 8 sem servidor: não existe sonda de ferramentas em outro host — o que há
/// é o PATH desta máquina, e a pasta de destino já foi aberta e escrita na etapa 2.
/// Sobram os comandos de preparação do modelo, rodando aqui.
fn prepare_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    let total = ctx.profile.prepare_commands.len();
    let executados = ctx.run_commands(8, "preparação", &ctx.profile.prepare_commands.clone())?;
    if total > 0 && ctx.profile.strategy != "script" {
        // A preparação roda antes da cópia e pode ter criado arquivo. A seleção é da
        // etapa 4, então sem reescanear o que a preparação gerou nunca chegaria ao
        // destino. Em `script` não há cópia nenhuma (etapa 4 pula de propósito), e
        // reescanear ali faria a confirmação cobrar no destino um arquivo que a
        // estratégia nunca envia.
        let lista = collect_files(
            &ctx.root,
            &ctx.profile.include_paths,
            &ctx.profile.exclude_paths,
            MAX_FILES,
        )
        .map_err(Fail::new)?;
        if lista.is_empty() && !ctx.profile.include_paths.is_empty() {
            return Err(Fail::new(
                "depois da preparação não sobrou nenhum arquivo para enviar — confira os caminhos",
            ));
        }
        ctx.files = lista;
        ctx.plano = None;
        gate(ctx, 8)?;
    }
    let selecionados = ctx.files.len();
    Ok(Info::done(format!(
        "{} pronto nesta máquina · {executados}/{total} comando(s) de preparação · {selecionados} arquivo(s) para copiar",
        ctx.base()
    )))
}

/// Caminhos de um destino local, com `/` como separador e a guarda de backups de
/// fora — é o espelho do `find . -type f` usado no servidor.
fn listar_arquivos_local(dir: &Path) -> Result<Vec<String>, String> {
    fn andar(pasta: &Path, prefixo: &str, out: &mut Vec<String>) -> Result<(), String> {
        let entradas = fs::read_dir(pasta)
            .map_err(|e| format!("falha ao ler {}: {e}", pasta.display()))?;
        for entrada in entradas {
            let entrada = entrada.map_err(|e| format!("falha ao ler {}: {e}", pasta.display()))?;
            let nome = entrada.file_name().to_string_lossy().to_string();
            let caminho = entrada.path();
            let rel = if prefixo.is_empty() {
                nome.clone()
            } else {
                format!("{prefixo}/{nome}")
            };
            if caminho.is_dir() {
                if nome == BACKUPS_DIR && prefixo.is_empty() {
                    continue;
                }
                andar(&caminho, &rel, out)?;
            } else {
                out.push(rel);
                if out.len() > MAX_FILES {
                    return Err(format!(
                        "o destino tem mais de {MAX_FILES} arquivos — revise a pasta configurada"
                    ));
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.is_dir() {
        andar(dir, "", &mut out)?;
    }
    out.sort();
    Ok(out)
}

impl Ctx {
    /// Compara origem e destino por conteúdo (CRC32, o mesmo critério do `cksum`
    /// remoto) uma única vez e guarda o resultado: a etapa de backup e a de cópia
    /// precisam do mesmo plano.
    fn plano_de_copia(&mut self) -> Result<(Vec<LocalFile>, usize), Fail> {
        if let Some(plano) = &self.plano {
            return Ok(plano.clone());
        }
        let base = self.destino_path();
        let mut a_copiar = Vec::new();
        let mut iguais = 0usize;
        for arquivo in self.files.clone() {
            if self.cancelled() {
                return Err(Fail::cancelled());
            }
            let alvo = base.join(&arquivo.rel);
            let mesmo = alvo.is_file() && crc_of_file(&alvo) == crc_of_file(&arquivo.abs);
            if mesmo {
                iguais += 1;
            } else {
                a_copiar.push(arquivo);
            }
        }
        let plano = (a_copiar, iguais);
        self.plano = Some(plano.clone());
        Ok(plano)
    }
}

fn backup_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return backup_local(ctx);
    }
    if !ctx.profile.backup_enabled {
        return Ok(Info::skip("backup desativado no modelo — nenhum ponto de recuperação novo"));
    }
    let base = ctx.base();
    let origem = if ctx.profile.releases_enabled {
        ctx.previous.clone()
    } else {
        Some(base.clone())
    };
    let Some(origem) = origem.filter(|o| !o.is_empty()) else {
        return Ok(Info::skip(
            "não há publicação ativa anterior: esta release vira o primeiro ponto de recuperação",
        ));
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let destino = remote_join(&base, &format!("backups/antes-{stamp}-{}.tgz", ctx.run_id));
    let script = ctx.script(&format!(
        "mkdir -p {}\nif [ -d {origem} ]; then tar -czf {destino} -C {origem} . && echo 'backup criado'; else echo 'nada a backupar'; fi\n",
        sshexec::shell_quote(&remote_join(&base, "backups")),
        origem = sshexec::shell_quote(&origem),
        destino = sshexec::shell_quote(&destino)
    ));
    ctx.remote(9, &script, "backup")?;
    ctx.backup = Some(destino.clone());
    Ok(Info::done(format!("ponto de recuperação em {destino} (origem {origem})")))
}

/// Ponto de recuperação do modo sem servidor: os arquivos que já estão no destino
/// e vão mudar são copiados para `<destino>/.devops-backups/<execução>/` antes de
/// qualquer sobrescrita. É essa pasta que a ação **Reverter** repõe no lugar.
fn backup_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    if !ctx.profile.backup_enabled {
        return Ok(Info::skip("backup desativado no modelo — nenhum ponto de recuperação novo"));
    }
    if ctx.files.is_empty() {
        return Ok(Info::skip("nenhum arquivo seria copiado — não há o que guardar do destino"));
    }
    let (a_copiar, _) = ctx.plano_de_copia()?;
    if a_copiar.is_empty() && !ctx.profile.delete_extras {
        return Ok(Info::skip(
            "o destino já tem exatamente o conteúdo selecionado — nada a guardar",
        ));
    }
    let base = ctx.destino_path();
    let guarda = base.join(BACKUPS_DIR).join(ctx.run_id.to_string());
    fs::create_dir_all(&guarda).map_err(|e| {
        Fail::new(format!(
            "não foi possível criar a pasta de backup {}: {e}",
            guarda.display()
        ))
    })?;
    let mut guardados = 0usize;
    for arquivo in &a_copiar {
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let atual = base.join(&arquivo.rel);
        if !atual.is_file() {
            continue;
        }
        let copia = guarda.join(&arquivo.rel);
        if let Some(pai) = copia.parent() {
            fs::create_dir_all(pai)
                .map_err(|e| Fail::new(format!("falha ao preparar {}: {e}", pai.display())))?;
        }
        fs::copy(&atual, &copia).map_err(|e| {
            Fail::new(format!(
                "falha ao guardar {} antes de sobrescrever: {e}",
                arquivo.rel
            ))
        })?;
        guardados += 1;
    }
    let caminho = guarda.display().to_string();
    ctx.backup = Some(caminho.clone());
    ctx.previous = Some(caminho.clone());
    if guardados == 0 {
        if ctx.profile.delete_extras {
            return Ok(Info::done(format!(
                "nada a sobrescrever em {} — a guarda {caminho} recebe o que a limpeza do destino remover",
                base.display()
            )));
        }
        return Ok(Info::skip(
            "nenhum dos arquivos selecionados existia no destino: a publicação só acrescenta arquivos",
        ));
    }
    Ok(Info::done(format!(
        "{guardados} arquivo(s) do destino guardados em {caminho}"
    )))
}

fn send_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return send_local(ctx);
    }
    match ctx.profile.strategy.as_str() {
        "git" => send_git(ctx),
        "script" => Ok(Info::skip("deploy por script: só comandos, nenhum arquivo é enviado")),
        _ => send_files(ctx),
    }
}

/// Cópia local: só o que difere em conteúdo é escrito no destino, do mesmo jeito
/// que o envio por sftp compara o `cksum` antes de transferir.
fn send_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.profile.strategy == "script" {
        return Ok(Info::skip("deploy por script: só comandos, nenhuma pasta é copiada"));
    }
    let (a_copiar, iguais) = ctx.plano_de_copia()?;
    let base = ctx.destino_path();
    let total = a_copiar.len();
    let mut copiados = 0usize;
    for arquivo in &a_copiar {
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let alvo = base.join(&arquivo.rel);
        if let Some(pai) = alvo.parent() {
            fs::create_dir_all(pai)
                .map_err(|e| Fail::new(format!("falha ao preparar {}: {e}", pai.display())))?;
        }
        fs::copy(&arquivo.abs, &alvo).map_err(|e| {
            Fail::failed(
                format!("copiar {}", arquivo.rel),
                None,
                format!("falha ao copiar {} para {}: {e}", arquivo.abs.display(), alvo.display()),
            )
        })?;
        copiados += 1;
        if copiados.is_multiple_of(UPLOAD_CHUNK) || copiados == total {
            ctx.mark(
                10,
                "running",
                Some(&format!("copiando {copiados}/{total} arquivo(s)…")),
                None,
                None,
            );
        }
    }
    ctx.sent = copiados;
    ctx.unchanged = iguais;
    if total == 0 {
        ctx.log(10, "out", "destino já igual à origem: nenhum arquivo precisou ser copiado");
    }
    let mut removidos = 0usize;
    if ctx.profile.delete_extras && !ctx.files.is_empty() {
        removidos = delete_extras_local(ctx, &base, &ctx.files.clone())?;
    }
    ctx.removed = removidos;
    let mut msg = format!(
        "{copiados} arquivo(s) copiado(s) para {} · {iguais} já igual(is)",
        base.display()
    );
    if removidos > 0 {
        msg.push_str(&format!(" · {removidos} removido(s) (exclusão explícita ativada)"));
    }
    Ok(Info::done(msg))
}

/// Apaga no destino local o que não está na seleção — sempre depois de guardar a
/// cópia na pasta de backup, e nunca dentro dela.
fn delete_extras_local(ctx: &mut Ctx, base: &Path, lista: &[LocalFile]) -> Result<usize, Fail> {
    let presentes = listar_arquivos_local(base).map_err(Fail::new)?;
    let selecionados: Vec<&str> = lista.iter().map(|f| f.rel.as_str()).collect();
    let guarda = ctx.backup.as_deref().map(PathBuf::from);
    let mut removidos = 0usize;
    for rel in presentes {
        if selecionados.contains(&rel.as_str()) {
            continue;
        }
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let caminho = base.join(&rel);
        if guarda.as_ref().is_some_and(|g| caminho.starts_with(g)) {
            continue;
        }
        // Com guarda disponível o arquivo é movido para ela, então "Reverter"
        // consegue repô-lo. Sem guarda (backup desativado) a remoção é direta.
        if let Some(g) = &guarda {
            let copia = g.join(&rel);
            if let Some(pai) = copia.parent() {
                fs::create_dir_all(pai)
                    .map_err(|e| Fail::new(format!("falha ao preparar {}: {e}", pai.display())))?;
            }
            if fs::rename(&caminho, &copia).is_err() {
                fs::copy(&caminho, &copia)
                    .map_err(|e| Fail::new(format!("falha ao guardar {rel}: {e}")))?;
                fs::remove_file(&caminho)
                    .map_err(|e| Fail::new(format!("falha ao remover {}: {e}", caminho.display())))?;
            }
        } else {
            fs::remove_file(&caminho)
                .map_err(|e| Fail::new(format!("falha ao remover {}: {e}", caminho.display())))?;
        }
        removidos += 1;
    }
    // Pastas que ficaram vazias depois da limpeza.
    podar_pastas_vazias(base);
    Ok(removidos)
}

/// Remove diretórios vazios abaixo do destino, sem tocar no próprio destino.
fn podar_pastas_vazias(base: &Path) {
    let Ok(entradas) = fs::read_dir(base) else { return };
    for entrada in entradas.flatten() {
        let Ok(tipo) = entrada.file_type() else { continue };
        if !tipo.is_dir() {
            continue;
        }
        let caminho = entrada.path();
        if caminho.file_name().and_then(|n| n.to_str()) == Some(BACKUPS_DIR) {
            continue;
        }
        podar_pastas_vazias(&caminho);
        if fs::read_dir(&caminho).map(|mut d| d.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(&caminho);
        }
    }
}

/// Modo B: o servidor atualiza o checkout a partir do origin e as mudanças
/// locais encontradas lá bloqueiam a sobrescrita em vez de sumirem em silêncio.
fn send_git(ctx: &mut Ctx) -> Result<Info, Fail> {
    let alvo = ctx
        .profile
        .git_branch
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    let dir = ctx.base();
    let url = ctx.remote_url.clone();
    let mut script = ctx.script("# git\n");
    script.push_str(&format!("cd {}\n", sshexec::shell_quote(&dir)));
    script.push_str(&format!(
        "if [ ! -d .git ]; then git clone --branch {branch} {url} .; else sujo=$(git status --porcelain); if [ -n \"$sujo\" ]; then echo 'MUDANÇAS LOCAIS NO SERVIDOR:'; printf '%s\\n' \"$sujo\" | head -n 20; exit 65; fi; git fetch --prune origin; git checkout {branch}; git reset --hard origin/{branch}; fi\n",
        branch = sshexec::shell_quote(&alvo),
        url = sshexec::shell_quote(&url)
    ));
    script.push_str("printf 'HEAD:%s\\n' \"$(git rev-parse --short HEAD)\"\n");
    let result = ctx.remote(10, &script, "atualizar repositório no servidor").map_err(|fail| {
        if fail.exit_code == Some(65) || fail.message.contains("MUDANÇAS LOCAIS NO SERVIDOR") {
            Fail::failed(
                "git status (servidor)".to_string(),
                Some(65),
                format!(
                    "o checkout em {} tem mudanças locais não previstas e nada foi sobrescrito. Revise no terminal do servidor, ou use publicação por releases.",
                    dir
                ),
            )
        } else {
            fail
        }
    })?;
    let head = parse_marker(&result.stdout, "HEAD:").unwrap_or_else(|| "desconhecido".into());
    Ok(Info::done(format!(
        "servidor na branch {alvo} · commit {head} · origem {url}"
    )))
}

/// Modos A e C: compara conteúdo (`cksum`) e envia apenas o que difere.
fn send_files(ctx: &mut Ctx) -> Result<Info, Fail> {
    let base = ctx.base();
    let destino = match &ctx.release {
        Some(rel) => remote_join(&base, rel),
        None => base.clone(),
    };
    let selecionados = ctx.files.clone();
    let mut a_enviar = selecionados.clone();
    if ctx.release.is_none() && ctx.has("cksum") && !selecionados.is_empty() {
        let script = ctx.script(&format!(
            "cd {} 2>/dev/null || exit 0\nfind . -type f -exec cksum {{}} + 2>/dev/null || true\n",
            sshexec::shell_quote(&destino)
        ));
        match ctx.remote(10, &script, "ler manifesto do destino") {
            Ok(result) => {
                let manifest = parse_manifest(&result.stdout);
                let mut lidos = 0usize;
                let mut hash = |f: &LocalFile| {
                    lidos += 1;
                    crc_of_file(&f.abs)
                };
                let precisam = changed_files(&selecionados, &manifest, &mut hash);
                ctx.log(
                    10,
                    "out",
                    &format!(
                        "manifesto do destino: {} entrada(s); {} arquivo(s) conferido(s) por conteúdo",
                        manifest.len(),
                        lidos
                    ),
                );
                ctx.unchanged = selecionados.len() - precisam.len();
                a_enviar = precisam;
            }
            Err(fail) => {
                ctx.log(
                    10,
                    "err",
                    &format!("sem manifesto do destino, envio completo: {}", fail.message),
                );
            }
        }
    }

    let total = a_enviar.len();
    let mut enviados = 0usize;
    for chunk in a_enviar.chunks(UPLOAD_CHUNK) {
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let pares: Vec<(PathBuf, String)> = chunk
            .iter()
            .map(|f| (f.abs.clone(), remote_join(&destino, &f.rel)))
            .collect();
        let label = format!("sftp put ({} arquivo(s))", pares.len());
        ctx.upload(&pares, &label)?;
        enviados += pares.len();
        ctx.sent = enviados;
        ctx.mark(
            10,
            "running",
            Some(&format!("enviando {enviados}/{total} arquivo(s)…")),
            None,
            None,
        );
    }
    if total == 0 {
        ctx.log(10, "out", "destino já igual à origem: nenhum arquivo precisou ser enviado");
    }

    if ctx.profile.delete_extras && ctx.release.is_none() && !selecionados.is_empty() {
        ctx.removed = delete_extras(ctx, &destino, &selecionados)?;
    }
    let mut msg = format!(
        "{enviados} arquivo(s) enviado(s) para {destino} · {} já igual(is)",
        ctx.unchanged
    );
    if ctx.removed > 0 {
        msg.push_str(&format!(" · {} removido(s) (exclusão explícita ativada)", ctx.removed));
    }
    Ok(Info::done(msg))
}

/// Remove no destino só o que não está na lista enviada, comparando por um
/// manifesto transferido por sftp — nunca por lista montada no argv.
fn delete_extras(ctx: &mut Ctx, destino: &str, lista: &[LocalFile]) -> Result<usize, Fail> {
    let tmp = std::env::temp_dir().join(format!("devops-manifest-{}.txt", ctx.run_id));
    let conteudo = lista
        .iter()
        .map(|f| f.rel.clone())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&tmp, format!("{conteudo}\n")).map_err(|e| {
        Fail::new(format!("falha ao preparar o manifesto local: {e}"))
    })?;
    let remoto = remote_join(destino, ".devops-manifest.txt");
    let enviado = ctx.upload(&[(tmp.clone(), remoto)], "sftp put (manifesto)");
    let resultado = if let Err(fail) = enviado {
        let _ = fs::remove_file(&tmp);
        return Err(fail);
    } else {
        let script = ctx.script(&format!(
            "cd {}\nfind . -type f ! -path './.devops-manifest.txt' | sed 's|^\\./||' > .devops-remoto.txt\ngrep -vxF -f .devops-manifest.txt .devops-remoto.txt > .devops-extras.txt 2>/dev/null || true\nremovidos=$(wc -l < .devops-extras.txt | tr -d ' ')\nwhile IFS= read -r f; do [ -n \"$f\" ] && rm -f -- \"$f\"; done < .devops-extras.txt\nrm -f .devops-manifest.txt .devops-remoto.txt .devops-extras.txt\nprintf 'REMOVED:%s\\n' \"$removidos\"\n",
            sshexec::shell_quote(destino)
        ));
        let r = ctx.remote(10, &script, "remover arquivos a mais no destino");
        let _ = fs::remove_file(&tmp);
        r?
    };
    Ok(parse_marker(&resultado.stdout, "REMOVED:")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0))
}

fn update_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return update_local(ctx);
    }
    let base = ctx.base();
    let mut operacoes = 0usize;
    if let Some(rel) = ctx.release.clone() {
        let script = ctx.script(&format!(
            "cd {}\nln -sfn {} .devops-current.tmp\nmv -Tf .devops-current.tmp current 2>/dev/null || {{ rm -f current; mv .devops-current.tmp current; }}\nprintf 'ACTIVE:%s\\n' \"$(readlink current)\"\n",
            sshexec::shell_quote(&base),
            sshexec::shell_quote(&rel)
        ));
        let result = ctx.remote(11, &script, "trocar diretório ativo")?;
        let ativa = parse_marker(&result.stdout, "ACTIVE:").unwrap_or_default();
        if ativa != rel {
            return Err(Fail::failed(
                "troca do symlink current".to_string(),
                Some(1),
                format!("o diretório ativo aponta para {ativa}, esperado {rel} — a publicação não está no ar"),
            ));
        }
        operacoes += 1;
    }
    if matches!(ctx.profile.strategy.as_str(), "git" | "script" | "docker") {
        operacoes += ctx.run_commands(11, "build/atualização no servidor", &ctx.profile.build_commands.clone())?;
    }
    if operacoes == 0 {
        return Ok(Info::skip(
            "nenhum comando de atualização remoto e nenhuma troca de release configurada",
        ));
    }
    Ok(Info::done(format!("{operacoes} operação(ões) de atualização concluída(s)")))
}

/// Etapa 11 sem servidor: não há symlink de release para trocar (o modelo local
/// não pode usar esse mecanismo), sobram os comandos de build do deploy por script,
/// que aqui rodariam "no servidor".
fn update_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.profile.strategy != "script" {
        return Ok(Info::skip(
            "sem servidor a publicação vai direto para a pasta de destino: não há troca de release nem comando remoto",
        ));
    }
    let n = ctx.run_commands(11, "build/atualização nesta máquina", &ctx.profile.build_commands.clone())?;
    if n == 0 {
        return Ok(Info::skip("nenhum comando de build ou atualização configurado no modelo"));
    }
    Ok(Info::done(format!("{n} operação(ões) de atualização concluída(s) nesta máquina")))
}

fn restart_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.profile.restart_commands.is_empty() {
        return Ok(Info::skip(
            "sem comandos de reinicialização — esta configuração não reinicia serviço",
        ));
    }
    let n = ctx.run_commands(12, "reinicialização", &ctx.profile.restart_commands.clone())?;
    Ok(Info::done(format!("{n} comando(s) de reinicialização aplicados")))
}

fn health_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.profile.health_checks.is_empty() {
        return Ok(Info::skip(
            "nenhum teste de saúde configurado — confirme o serviço manualmente",
        ));
    }
    let n = ctx.run_commands(13, "saúde", &ctx.profile.health_checks.clone())?;
    Ok(Info::done(format!("{n} teste(s) de saúde passaram")))
}

fn confirm_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return confirm_local(ctx);
    }
    let base = ctx.base();
    let ativo = match &ctx.release {
        Some(rel) => remote_join(&base, rel),
        None => base.clone(),
    };
    let script = ctx.script(&format!(
        "cd {}\nativos=$(find . -type f | wc -l | tr -d ' ')\nprintf 'LIVE:%s\\n' \"$(pwd)\"\nprintf 'FILES:%s\\n' \"$ativos\"\n",
        sshexec::shell_quote(&ativo)
    ));
    let result = ctx.remote(14, &script, "confirmar publicação")?;
    let live = parse_marker(&result.stdout, "LIVE:").unwrap_or_default();
    let count = parse_marker(&result.stdout, "FILES:")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if live.is_empty() {
        return Err(Fail::failed(
            "confirmar publicação".to_string(),
            Some(1),
            "o servidor não devolveu o diretório ativo — resultado não confirmado",
        ));
    }
    if count == 0 {
        return Err(Fail::failed(
            "confirmar publicação".to_string(),
            Some(1),
            format!("o diretório ativo {live} está vazio — a publicação não produziu arquivos"),
        ));
    }
    ctx.confirmation = format!("{live} no ar com {count} arquivo(s) · {} enviado(s)", ctx.sent);
    let mut msg = format!("publicação confirmada em {live} · {count} arquivo(s) no destino");
    if ctx.profile.releases_enabled {
        msg.push_str(&format!(
            " · release {} · anterior {}",
            ctx.release.clone().unwrap_or_default(),
            ctx.previous.clone().unwrap_or_else(|| "nenhuma".to_string())
        ));
    }
    if let Some(b) = ctx.backup.clone() {
        msg.push_str(&format!(" · backup em {b}"));
    }
    Ok(Info::done(msg))
}

/// Conferência do modo sem servidor: cada arquivo selecionado é relido no destino
/// e comparado com a origem. Não é "assumir que copiou" — é o oposto do `find` +
/// contagem que a etapa remota faz por SSH.
fn confirm_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.files.is_empty() {
        // Um rollback por reposição da guarda já escreveu a própria confirmação
        // na etapa 11; aqui ela é apenas repetida.
        if !ctx.confirmation.is_empty() {
            return Ok(Info::done(ctx.confirmation.clone()));
        }
        ctx.confirmation =
            "nenhum arquivo foi copiado: as etapas de comando desta execução são a confirmação"
                .to_string();
        return Ok(Info::skip(
            "nenhum arquivo foi copiado — as etapas de comando acima são a confirmação desta publicação",
        ));
    }
    let base = ctx.destino_path();
    let mut divergentes: Vec<String> = Vec::new();
    for arquivo in ctx.files.clone() {
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let alvo = base.join(&arquivo.rel);
        let igual = alvo.is_file() && crc_of_file(&alvo) == crc_of_file(&arquivo.abs);
        if !igual {
            divergentes.push(arquivo.rel);
        }
    }
    if !divergentes.is_empty() {
        let mostradas: Vec<&String> = divergentes.iter().take(5).collect();
        return Err(Fail::failed(
            format!("conferir {}", base.display()),
            Some(1),
            format!(
                "{} arquivo(s) não bateram com a origem no destino: {}{}",
                divergentes.len(),
                mostradas.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                if divergentes.len() > 5 { " …" } else { "" }
            ),
        ));
    }
    let presentes = listar_arquivos_local(&base).map_err(Fail::new)?;
    if presentes.is_empty() {
        return Err(Fail::failed(
            format!("conferir {}", base.display()),
            Some(1),
            "a pasta de destino está vazia — a publicação não produziu arquivos".to_string(),
        ));
    }
    ctx.confirmation = format!(
        "{} no ar com {} arquivo(s) · {} copiado(s) nesta execução",
        base.display(),
        presentes.len(),
        ctx.sent
    );
    let mut msg = format!(
        "publicação confirmada em {} · {} arquivo(s) no destino",
        base.display(),
        presentes.len()
    );
    if let Some(b) = ctx.backup.clone() {
        msg.push_str(&format!(" · ponto de recuperação em {b}"));
    }
    Ok(Info::done(msg))
}

fn register_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        let guarda = match &ctx.backup {
            Some(b) => format!("ponto de recuperação em {b}"),
            None => "sem guarda de arquivos".to_string(),
        };
        return Ok(Info::done(format!(
            "{} copiado(s), {} inalterado(s), {} de fora do destino · {} · {guarda}",
            ctx.sent,
            ctx.unchanged,
            ctx.removed,
            ctx.base(),
        )));
    }
    Ok(Info::done(format!(
        "{} enviado(s), {} inalterado(s), {} removido(s), publicação em {}, anterior {}",
        ctx.sent,
        ctx.unchanged,
        ctx.removed,
        ctx.release.clone().unwrap_or_else(|| ctx.base()),
        ctx.previous.clone().unwrap_or_else(|| "nenhuma".to_string())
    )))
}

fn rollback_switch(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return rollback_local(ctx);
    }
    let base = ctx.base();
    if !ctx.profile.rollback_commands.is_empty() {
        let n = ctx.run_commands(11, "rollback", &ctx.profile.rollback_commands.clone())?;
        return Ok(Info::done(format!("{n} comando(s) de rollback executados")));
    }
    let anterior = ctx.previous.clone().ok_or_else(|| {
        Fail::new(
            "não há publicação anterior registrada nem comandos de rollback configurados — reversão automática não é possível",
        )
    })?;
    let alvo = if anterior.starts_with('/') {
        anterior.clone()
    } else {
        remote_join(&base, &anterior)
    };
    let relativo = alvo
        .strip_prefix(&base)
        .unwrap_or(&alvo)
        .trim_start_matches('/')
        .to_string();
    let script = ctx.script(&format!(
        "cd {}\ntest -d {} || {{ echo 'a release anterior não existe mais'; exit 66; }}\nln -sfn {} .devops-rollback.tmp\nmv -Tf .devops-rollback.tmp current 2>/dev/null || {{ rm -f current; mv .devops-rollback.tmp current; }}\nprintf 'ACTIVE:%s\\n' \"$(readlink current)\"\n",
        sshexec::shell_quote(&base),
        sshexec::shell_quote(&alvo),
        sshexec::shell_quote(&relativo)
    ));
    ctx.remote(11, &script, "reverter para a publicação anterior")?;
    ctx.release = Some(relativo);
    Ok(Info::done(format!("diretório ativo voltou para {alvo}")))
}

/// Reversão do modo sem servidor: os arquivos voltam da pasta de guarda daquela
/// execução, ou os comandos de rollback do modelo rodam nesta máquina. Nada é
/// apagado para reverter — o que existe na guarda é reposto no lugar.
fn rollback_local(ctx: &mut Ctx) -> Result<Info, Fail> {
    if !ctx.profile.rollback_commands.is_empty() {
        let n = ctx.run_commands(11, "rollback", &ctx.profile.rollback_commands.clone())?;
        return Ok(Info::done(format!(
            "{n} comando(s) de rollback executados nesta máquina"
        )));
    }
    let anterior = ctx.previous.clone().ok_or_else(|| {
        Fail::new(
            "esta execução não guardou arquivos do destino e o modelo não tem comandos de rollback — não há para onde voltar",
        )
    })?;
    let guarda = PathBuf::from(&anterior);
    if !guarda.is_dir() {
        return Err(Fail::new(format!(
            "a guarda {} não existe mais — nada foi alterado no destino",
            guarda.display()
        )));
    }
    let lista = listar_arquivos_local(&guarda).map_err(Fail::new)?;
    if lista.is_empty() {
        return Err(Fail::new(format!(
            "a guarda {} está vazia — nada foi alterado no destino",
            guarda.display()
        )));
    }
    let base = ctx.destino_path();
    let mut repostos = 0usize;
    for rel in &lista {
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let origem = guarda.join(rel);
        let alvo = base.join(rel);
        if let Some(pai) = alvo.parent() {
            fs::create_dir_all(pai)
                .map_err(|e| Fail::new(format!("falha ao preparar {}: {e}", pai.display())))?;
        }
        fs::copy(&origem, &alvo).map_err(|e| {
            Fail::new(format!("falha ao repor {} no destino: {e}", rel))
        })?;
        repostos += 1;
    }
    ctx.sent = repostos;
    ctx.confirmation = format!(
        "{} arquivo(s) reposto(s) em {} desde a guarda",
        repostos,
        base.display()
    );
    Ok(Info::done(format!(
        "{repostos} arquivo(s) voltaram ao lugar desde {}",
        guarda.display()
    )))
}

// ---------- entrada pública ----------

/// Carrega modelo/servidor/projeto, abre a linha de execução e dispara o
/// pipeline em uma thread própria.
pub fn start(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    profile_id: i64,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
) -> Result<i64, String> {
    let (profile, servidor, project, root, extra, run_id) = {
        let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
        let profile = db::get_deploy_profile(&conn, profile_id)?;
        let servidor = servidor_do_modelo(&conn, &profile)?;
        let project = db::get_project(&conn, profile.project_id)?;
        let extra = security::extra_patterns(
            db::setting_value(&conn, SECRET_PATTERNS_SETTING)?.as_deref(),
        );
        let root = PathBuf::from(&project.local_path);
        let run_id = db::insert_run(&conn, profile_id)?;
        let _ = audit::log(
            &conn,
            "info",
            "deploy",
            "Deploy iniciado",
            servidor.as_ref().map(|s| s.name.as_str()),
            Some(&project.name),
            None,
            Some(&format!(
                "modelo {} · estratégia {} · execução #{run_id}{}",
                profile.name,
                profile.strategy,
                if servidor.is_none() { " · nesta máquina" } else { "" }
            )),
        );
        (profile, servidor, project, root, extra, run_id)
    };
    run(
        app,
        state,
        db,
        broker,
        run_id,
        profile,
        servidor,
        project,
        root,
        extra,
        false,
        secret_values,
        allow_secrets,
        None,
    )
}

/// O servidor cadastrado do modelo, quando ele tem um. A ausência é o modo sem
/// servidor: a publicação acontece nas pastas e no shell desta máquina.
fn servidor_do_modelo(conn: &rusqlite::Connection, profile: &DeployProfile) -> Result<Option<Server>, String> {
    match profile.server_id {
        Some(id) => db::get_server(conn, id).map(Some),
        None => Ok(None),
    }
}

/// Rollback de uma execução registrada: volta o symlink para a release anterior
/// daquela execução ou roda os comandos de rollback do modelo.
pub fn rollback(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    run_id: i64,
    secret_values: HashMap<String, String>,
) -> Result<i64, String> {
    let (profile, servidor, project, root, extra, previous, new_run) = {
        let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
        let origem = db::get_run(&conn, run_id)?;
        if origem.status != "success" && origem.status != "failed" {
            return Err(format!(
                "a execução #{run_id} terminou com status “{}” — não há o que reverter",
                origem.status
            ));
        }
        let profile = db::get_deploy_profile(&conn, origem.profile_id)?;
        let servidor = servidor_do_modelo(&conn, &profile)?;
        let project = db::get_project(&conn, profile.project_id)?;
        let extra = security::extra_patterns(
            db::setting_value(&conn, SECRET_PATTERNS_SETTING)?.as_deref(),
        );
        let root = PathBuf::from(&project.local_path);
        let novo = db::insert_run(&conn, profile.id)?;
        let _ = audit::log(
            &conn,
            "warning",
            "deploy",
            "Rollback iniciado",
            servidor.as_ref().map(|s| s.name.as_str()),
            Some(&project.name),
            None,
            Some(&format!(
                "execução #{run_id} → #{novo} (destino {})",
                origem
                    .previous_release
                    .clone()
                    .unwrap_or_else(|| "comandos do modelo".to_string())
            )),
        );
        (
            profile,
            servidor,
            project,
            root,
            extra,
            origem.previous_release,
            novo,
        )
    };
    run(
        app,
        state,
        db,
        broker,
        new_run,
        profile,
        servidor,
        project,
        root,
        extra,
        true,
        secret_values,
        true,
        previous,
    )
}

#[allow(clippy::too_many_arguments)]
fn run(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    run_id: i64,
    profile: DeployProfile,
    servidor: Option<Server>,
    project: Project,
    root: PathBuf,
    extra: Vec<String>,
    rollback: bool,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
    previous: Option<String>,
) -> Result<i64, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let operacao = if rollback { "Rollback" } else { "Deploy" };
    let destino = servidor
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| profile.remote_path.clone());
    let label = format!("{operacao} {} → {destino} (#{run_id})", profile.name);
    state.register(run_id, label.clone(), Arc::clone(&cancel))?;

    let ctx = Ctx {
        app: app.clone(),
        db: db.clone(),
        run_id,
        cancel: Arc::clone(&cancel),
        touched: AtomicU16::new(0),
        provided: secret_values,
        secrets: Secrets::new(&profile.secret_keys, &HashMap::new()),
        askpass: None,
        shell: "sh".to_string(),
        files: Vec::new(),
        plano: None,
        release: None,
        previous,
        tools: HashMap::new(),
        branch: String::new(),
        remote_url: String::new(),
        sent: 0,
        unchanged: 0,
        removed: 0,
        backup: None,
        confirmation: String::new(),
        allow_secrets,
        extra,
        profile,
        server: servidor,
        project,
        root,
        broker,
    };
    // todas as etapas aparecem como "aguardando" assim que o clique acontece
    for position in 1..=15 {
        ctx.mark(position, "waiting", None, None, None);
    }
    let thread_state = state.clone();
    let spawned = thread::Builder::new()
        .name(format!("deploy-{run_id}"))
        .spawn(move || execute(ctx, thread_state, rollback));
    if let Err(e) = spawned {
        state.forget(run_id);
        if let Ok(conn) = db.lock() {
            let _ = db::finish_run(
                &conn,
                run_id,
                "failed",
                None,
                None,
                Some(&format!("falha ao iniciar a thread do pipeline: {e}")),
            );
        }
        return Err(format!("falha ao iniciar o pipeline: {e}"));
    }
    Ok(run_id)
}

fn execute(ctx: Ctx, state: DeployState, rollback: bool) {
    let mut ctx = ctx;
    let resultado = if rollback {
        rollback_pipeline(&mut ctx)
    } else {
        pipeline(&mut ctx)
    };
    let flags = ctx.touched.load(Ordering::Relaxed);
    for position in 1..=15 {
        if flags & (1u16 << position) == 0 {
            ctx.mark(position, "skipped", Some("etapa não alcançada"), None, None);
        }
    }
    let (status, detail) = match &resultado {
        Ok(resumo) => ("success", resumo.clone()),
        Err(fail) => (
            if fail.cancelled { "cancelled" } else { "failed" },
            fail.message.clone(),
        ),
    };
    finalize(&ctx, status, &detail, rollback);
    state.forget(ctx.run_id);
}

/// Grava o desfecho, audita e entrega o objeto completo para a interface.
fn finalize(ctx: &Ctx, status: &str, detail: &str, rollback: bool) {
    let detail = ctx.secrets.mask(detail);
    let run = {
        let Ok(conn) = ctx.db.lock() else { return };
        let ativa = match &ctx.release {
            Some(r) => Some(remote_join(&ctx.base(), r)),
            // Sem servidor não há release: o que fica registrado como publicação
            // ativa é a própria pasta de destino.
            None if ctx.local_mode() => Some(ctx.base()),
            None => None,
        };
        let _ = db::finish_run(
            &conn,
            ctx.run_id,
            status,
            ativa.as_deref(),
            ctx.previous.as_deref(),
            Some(&detail),
        );
        let _ = audit::log(
            &conn,
            match status {
                "success" => "success",
                "cancelled" => "warning",
                _ => "error",
            },
            "deploy",
            if rollback { "Rollback" } else { "Deploy" },
            ctx.server.as_ref().map(|s| s.name.as_str()),
            Some(&ctx.project.name),
            None,
            Some(&format!(
                "modelo {} · estratégia {} · execução #{} · {}",
                ctx.profile.name, ctx.profile.strategy, ctx.run_id, detail
            )),
        );
        db::get_run(&conn, ctx.run_id)
    };
    if let Ok(run) = run {
        let _ = ctx.app.emit("deploy-done", FinalEvent { run_id: ctx.run_id, run });
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalEvent {
    pub run_id: i64,
    pub run: DeployRun,
}

pub fn cancel(state: &DeployState, run_id: i64) -> Result<String, String> {
    state.cancel(run_id)
}

pub fn active(state: &DeployState) -> Vec<ActiveInfo> {
    state
        .active()
        .into_iter()
        .map(|(run_id, label)| ActiveInfo { run_id, label })
        .collect()
}

/// Pré-vistoria local (sem rede): quantos arquivos subiriam e o que a ache de
/// segredos encontraria. A interface usa isso para pedir confirmação explícita
/// antes de autorizar um deploy que tocaria arquivos sensíveis.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub files: usize,
    pub bytes: u64,
    pub sample: Vec<String>,
    pub findings: Vec<security::Finding>,
    pub blocked: usize,
    pub warnings: usize,
    pub message: String,
}

pub fn preflight(db: &DbHandle, profile_id: i64) -> Result<Preflight, String> {
    let (profile, project, extra) = {
        let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
        let profile = db::get_deploy_profile(&conn, profile_id)?;
        let project = db::get_project(&conn, profile.project_id)?;
        let extra = security::extra_patterns(db::setting_value(&conn, SECRET_PATTERNS_SETTING)?.as_deref());
        (profile, project, extra)
    };
    let root = PathBuf::from(&project.local_path);
    if !root.is_dir() {
        return Err(format!("a pasta do projeto não existe: {}", root.display()));
    }
    let envia = matches!(profile.strategy.as_str(), "sftp" | "artifacts" | "docker")
        && !(profile.strategy == "docker" && profile.include_paths.is_empty());
    let mut findings = Vec::new();
    let mut lista: Vec<LocalFile> = Vec::new();
    if envia {
        lista = collect_files(&root, &profile.include_paths, &profile.exclude_paths, MAX_FILES)?;
        let rels: Vec<String> = lista.iter().map(|f| f.rel.clone()).collect();
        findings.extend(security::check_paths(&root, &rels, &extra));
    }
    if project.is_git_repo {
        findings.extend(security::tracked_sensitive(&root, &extra).unwrap_or_default());
    }
    let bytes: u64 = lista.iter().map(|f| f.size).sum();
    let findings = security::summarize(findings);
    let blocked = findings.iter().filter(|f| f.severity == "block").count();
    let warnings = findings.iter().filter(|f| f.severity == "warn").count();
    let message = if !envia {
        format!(
            "estratégia {}: nenhum arquivo local é enviado; {blocked} item(ns) sensível(is) no índice do Git",
            profile.strategy
        )
    } else if blocked > 0 {
        format!(
            "{} arquivo(s), {} — {blocked} item(ns) bloqueante(s): confirme explicitamente ou exclua os caminhos",
            lista.len(),
            human_bytes(bytes)
        )
    } else {
        format!(
            "{} arquivo(s), {} prontos para publicação",
            lista.len(),
            human_bytes(bytes)
        )
    };
    Ok(Preflight {
        sample: lista.iter().take(12).map(|f| f.rel.clone()).collect(),
        files: lista.len(),
        bytes,
        findings,
        blocked,
        warnings,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(rel: &str, size: u64) -> LocalFile {
        LocalFile {
            rel: rel.to_string(),
            abs: PathBuf::from(rel),
            size,
        }
    }

    #[test]
    fn padrao_de_inclusao_e_previsivel() {
        assert!(matches_pattern("dist/index.html", "dist"));
        assert!(matches_pattern("dist/assets/app.js", "dist/"));
        assert!(!matches_pattern("src/dist/index.html", "dist"));
        assert!(matches_pattern("logs/error.log", "*.log"));
        assert!(matches_pattern("a.log", "*.log"));
        assert!(!matches_pattern("a.logx", "*.log"));
        assert!(matches_pattern("src/a/b.ts", "src/*"));
        assert!(matches_pattern("./README.md", "README.md"));
        assert!(!matches_pattern("README.md", ""));
    }

    #[test]
    fn crc32_bate_com_o_cksum_posix() {
        // vetor conhecido: CRC-32 de "123456789" = 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn crc_do_arquivo_lido_em_blocos() {
        // O envio compara por conteúdo lendo o arquivo em pedaços: o resultado
        // tem de bater com o CRC do buffer inteiro, inclusive atravessando o
        // bloco de 64 KiB.
        let dir = std::env::temp_dir().join(format!("devops-crc-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let caminho = dir.join("grande.bin");
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&caminho, &bytes).unwrap();
        assert_eq!(crc_of_file(&caminho), Some(crc32(&bytes) as u64));
        assert_eq!(crc_of_file(&dir.join("nao-existe.bin")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifesto_lendo_linhas_de_cksum() {
        let m = parse_manifest("123 45 ./index.html\n9 1 assets/app.js\nlixo\n");
        assert_eq!(m.get("index.html"), Some(&(123u64, 45u64)));
        assert_eq!(m.get("assets/app.js"), Some(&(9u64, 1u64)));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn compara_conteudo_antes_de_enviar() {
        let local = vec![s("a.js", 10), s("b.js", 20), s("novo.js", 5)];
        let mut manifest = RemoteManifest::new();
        // tamanho igual e crc igual → não envia
        manifest.insert("a.js".into(), (crc32(b"conteudo-a") as u64, 10));
        // tamanho igual e crc diferente → envia
        manifest.insert("b.js".into(), (999, 20));
        let enviados = changed_files(&local, &manifest, |f| {
            match f.rel.as_str() {
                "a.js" => Some(crc32(b"conteudo-a") as u64),
                "b.js" => Some(crc32(b"mudou") as u64),
                _ => None,
            }
        });
        let nomes: Vec<&str> = enviados.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(nomes, vec!["b.js", "novo.js"]);
    }

    #[test]
    fn junta_caminhos_sem_duplicar_barra() {
        assert_eq!(remote_join("/var/www/site", "releases/x"), "/var/www/site/releases/x");
        assert_eq!(remote_join("/var/www/site/", "/index.html"), "/var/www/site/index.html");
        assert_eq!(remote_join("/var/www", ""), "/var/www");
    }

    #[test]
    fn nome_de_release_so_tem_caracteres_de_argomento() {
        let nome = release_name("2026-10-09T12:00:00Z", 7);
        assert_eq!(nome, "releases/2026-10-09T12:00:00Z-7");
        assert!(nome.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-' | ':')));
    }

    #[test]
    fn markers_vem_da_saida_real() {
        assert_eq!(
            parse_marker("ok\nPREV:releases/1\n", "PREV:").as_deref(),
            Some("releases/1")
        );
        assert_eq!(parse_marker("PREV:\n", "PREV:"), None);
        assert_eq!(parse_marker("HEAD:abc123\n", "HEAD:").as_deref(), Some("abc123"));
    }

    #[test]
    fn redact_esconde_valores_do_deploy() {
        let texto = "curl -H 'Authorization: Bearer sup3r-segredo' https://api";
        assert!(redact(texto, &["sup3r-segredo"]).contains("***"));
        assert!(!redact(texto, &["sup3r-segredo"]).contains("sup3r-segredo"));
        // valores curtos demais não são mascarados para não destruir o log
        assert_eq!(redact("token ab ok", &["ab"]), "token ab ok");
    }

    #[test]
    fn secrets_usam_somente_nomes_declarados() {
        let mut provided = HashMap::new();
        provided.insert("SENTRY_TOKEN".to_string(), "abc123".to_string());
        provided.insert("NAO_DECLARADO".to_string(), "zzz".to_string());
        provided.insert("VAZIA".to_string(), "   ".to_string());
        let declared = vec!["SENTRY_TOKEN".to_string(), "VAZIA".to_string()];
        let secrets = Secrets::new(&declared, &provided);
        assert_eq!(secrets.count(), 1);
        let prelude = secrets.prelude();
        assert!(prelude.contains("export SENTRY_TOKEN='abc123'"), "{prelude}");
        assert!(!prelude.contains("NAO_DECLARADO"));
        assert!(secrets.mask("chamada com abc123 no cabeçalho").contains("***"));
        assert_eq!(Secrets::missing(&declared, &HashMap::new()).len(), 2);
    }

    #[test]
    fn aspas_simples_protegem_o_valor_no_export() {
        let mut provided = HashMap::new();
        provided.insert("K".to_string(), "a'b; rm -rf /".to_string());
        let secrets = Secrets::new(&["K".to_string()], &provided);
        let prelude = secrets.prelude();
        assert!(prelude.contains("'a'\\''b; rm -rf /'"), "{prelude}");
        assert!(!prelude.contains("export K=a"));
    }

    #[test]
    fn coletar_arquivos_respeita_exclusao_e_pula_git() {
        let dir = std::env::temp_dir().join(format!("devops-deploy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dist/assets")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("dist/index.html"), "x").unwrap();
        fs::write(dir.join("dist/assets/app.js"), "yyy").unwrap();
        fs::write(dir.join(".git/config"), "nao deve subir").unwrap();

        let todos = collect_files(&dir, &[], &[], 100).unwrap();
        let nomes: Vec<&str> = todos.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(nomes, vec!["dist/assets/app.js", "dist/index.html"]);

        let com_exclusao = collect_files(&dir, &[], &["*.js".to_string()], 100).unwrap();
        assert_eq!(com_exclusao.len(), 1);

        let so_assets = collect_files(&dir, &["dist/assets".to_string()], &[], 100).unwrap();
        assert_eq!(so_assets.len(), 1);
        assert_eq!(so_assets[0].size, 3);

        let limite = collect_files(&dir, &[], &[], 0);
        assert!(limite.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lista_vazia_de_include_nao_esconde_nada() {
        assert!(collect_files(Path::new(":"), &[], &[], 10).is_err());
    }

    #[test]
    fn etapa_um_a_quinze_tem_nome() {
        assert_eq!(step_name(1), "Validar configurações e credenciais");
        assert_eq!(step_name(15), "Registrar o histórico do deploy");
        assert_eq!(step_name(0), "Validar configurações e credenciais");
        assert_eq!(step_name(99), "Registrar o histórico do deploy");
        assert_eq!(STEPS.len(), 15);
    }

    #[test]
    fn cancelamento_chegna_na_flag_da_execucao() {
        let state = DeployState::new();
        let cancel = Arc::new(AtomicBool::new(false));
        state
            .register(3, "Deploy modelo → server (#3)".into(), Arc::clone(&cancel))
            .unwrap();
        assert_eq!(state.active().len(), 1);
        let label = state.cancel(3).unwrap();
        assert!(label.contains("#3"));
        assert!(label.starts_with("Deploy "));
        assert!(cancel.load(Ordering::SeqCst));
        assert!(state.cancel(99).is_err());
        state.forget(3);
        assert!(state.active().is_empty());
    }

    #[test]
    fn tail_pega_as_ultimas_linhas() {
        let texto = tail("linha 1\n\nlinha 2\nfalhou de verdade", "");
        assert_eq!(texto, "linha 1 | linha 2 | falhou de verdade");
        assert_eq!(tail("", "uma saída"), "uma saída");
    }

    #[test]
    fn formata_tamanhos_para_a_interface() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn caminhos_remotos_vao_sempre_entre_aspas_simples() {
        // O destino é dados, nunca sintaxe: apóstrofo e espaço não abrem o script.
        let quoted = sshexec::shell_quote("/var/www/minha 'site'");
        let script = format!("mkdir -p {quoted}\n");
        assert_eq!(
            script,
            "mkdir -p '/var/www/minha '\\''site'\\'''\n"
        );
        let destino = "/var/www/backups/antes.tgz";
        let tar = format!("tar -czf {} -C {} .\n", sshexec::shell_quote(destino), sshexec::shell_quote("/var/www/site/current"));
        assert!(tar.starts_with("tar -czf '/var/www/backups/antes.tgz' -C '/var/www/site/current'"), "{tar}");
    }

    #[test]
    fn prelude_exporta_variaveis_sem_tocar_no_argv() {
        let mut provided = HashMap::new();
        provided.insert("DB_URL".to_string(), "postgres://u:p@h/db".to_string());
        provided.insert("TAG".to_string(), "v1.2".to_string());
        let declared = vec!["DB_URL".to_string(), "TAG".to_string()];
        let secrets = Secrets::new(&declared, &provided);
        let prelude = secrets.prelude();
        assert_eq!(prelude, "export DB_URL='postgres://u:p@h/db'\nexport TAG='v1.2'\n");
        // os nomes aparecem, os valores só saem mascarados
        assert!(secrets.mask("connect postgres://u:p@h/db ok").contains("***"));
    }

    #[test]
    fn variaveis_locais_vao_para_o_ambiente_do_processo() {
        let mut provided = HashMap::new();
        provided.insert("TOKEN".to_string(), "abc123".to_string());
        provided.insert("SO".to_string(), "linux".to_string());
        let declared = vec!["TOKEN".to_string(), "SO".to_string(), "AUSENTE".to_string()];
        let secrets = Secrets::new(&declared, &provided);
        let pares = secrets.env();
        // só o que tem valor vai para o ambiente, na ordem declarada
        assert_eq!(
            pares,
            vec![
                ("TOKEN".to_string(), "abc123".to_string()),
                ("SO".to_string(), "linux".to_string()),
            ]
        );
    }

    #[test]
    fn etapa_local_muda_só_o_nome_das_etapas_de_servidor() {
        for position in 1..=15 {
            let remoto = step_name(position);
            let local = step_name_local(position);
            if position == 2 || position == 10 {
                assert_ne!(remoto, local, "etapa {position} deveria mudar de nome");
            } else {
                assert_eq!(remoto, local, "etapa {position} mudou sem precisar");
            }
        }
        assert_eq!(step_name_local(2), "Abrir a pasta de destino");
        assert_eq!(step_name_local(10), "Copiar os arquivos para o destino");
    }

    #[test]
    fn listar_arquivos_local_espelha_o_find_do_servidor() {
        let base = pasta_de_teste("listar");
        fs::create_dir_all(base.join("assets/img")).unwrap();
        fs::write(base.join("index.html"), b"oi").unwrap();
        fs::write(base.join("assets/app.js"), b"js").unwrap();
        fs::write(base.join("assets/img/logo.svg"), b"svg").unwrap();
        fs::create_dir_all(base.join(BACKUPS_DIR).join("7")).unwrap();
        fs::write(base.join(BACKUPS_DIR).join("7/antigo.html"), b"velho").unwrap();

        let lista = listar_arquivos_local(&base).unwrap();
        assert_eq!(
            lista,
            vec!["assets/app.js".to_string(), "assets/img/logo.svg".to_string(), "index.html".to_string()]
        );
        // destino que ainda não existe é pasta vazia, não erro
        assert!(listar_arquivos_local(&base.join("ausente")).unwrap().is_empty());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn podar_pastas_vazias_limpa_debaixo_do_destino_sem_tocar_na_guarda() {
        let base = pasta_de_teste("podar");
        fs::create_dir_all(base.join("vazia/interna")).unwrap();
        fs::create_dir_all(base.join(BACKUPS_DIR).join("9")).unwrap();
        fs::write(base.join(BACKUPS_DIR).join("9/guardado.html"), b"g").unwrap();
        fs::write(base.join("index.html"), b"i").unwrap();

        podar_pastas_vazias(&base);
        assert!(!base.join("vazia").exists(), "a pasta vazia deveria sumir");
        assert!(base.join(BACKUPS_DIR).join("9").is_dir(), "a guarda não pode ser podada");
        assert!(base.join("index.html").is_file());
        fs::remove_dir_all(&base).unwrap();
    }

    /// Pasta exclusiva do teste dentro do temporário do sistema.
    fn pasta_de_teste(nome: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("devops-teste-{}-{nome}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("criar pasta de teste");
        dir
    }
}
