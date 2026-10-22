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
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU16, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{
    analysis, audit, auth,
    db::{self, DbHandle},
    files, git, git_probe,
    models::{
        strategy_needs_server, validate_strategy, AnalysisInput, AnalysisRun, DeployProfile,
        DeployRun, PipelineNode, Project, Server,
    },
    runtime::{self},
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
/// O teto de linhas de saída gravadas por tentativa de uma etapa. Passou do teto,
/// uma última linha `system` declara o corte: bloco truncado em silêncio é igual a
/// bloco que não existe, e quem lê precisa saber que aquilo não é o fim. É o mesmo
/// número que a leitura usa como tope, para ninguém pedir mais do que existe.
pub const TETO_LINHAS_POR_TENTATIVA: i64 = 2_000;
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
    approvals: Arc<ApprovalBroker>,
}

impl DeployState {
    pub fn new() -> Self {
        Self {
            runs: Arc::new(Mutex::new(HashMap::new())),
            approvals: Arc::new(ApprovalBroker::default()),
        }
    }

    /// A porta da aprovação manual: a interface responde aqui e o nó que espera
    /// continua (ou para) com a decisão.
    pub fn approvals(&self) -> &ApprovalBroker {
        &self.approvals
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
    /// O nó do grafo que esta linha representa; `None` no pipeline linear.
    pub node_key: Option<String>,
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

// ---------- aprovação manual ----------

/// O pedido que a janela mostra enquanto um nó espera. Vem com o destino e o
/// ambiente porque é exatamente isso que quem aprova está autorizando.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: i64,
    pub run_id: i64,
    pub node_key: String,
    pub title: String,
    pub definition: String,
    pub environment: String,
    pub destination: String,
    pub waits_secs: u64,
}

/// O que a janela precisa ver para poder decidir: qual corrida, qual nó e o que
/// ele vai tocar. É uma coisa só, e por isso viaja junto.
#[derive(Debug, Clone)]
pub struct PedidoDeAprovacao {
    pub run_id: i64,
    pub node_key: String,
    pub title: String,
    pub definition: String,
    pub environment: String,
    pub destination: String,
}

/// Como um nó para de esperar. `Denied` e `Expired` são diferentes de `Cancelled`:
/// a primeira é uma decisão, a segunda é a ausência dela, e a terceira veio de quem
/// interrompeu a corrida inteira.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Denied,
    Expired,
    Cancelled,
}

/// Tempo máximo de espera por uma decisão na janela.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Fatia de espera: é o que permite perceber o cancelamento sem perder a resposta.
const APPROVAL_SLICE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct ApprovalInner {
    pending: Mutex<HashMap<i64, Sender<Decision>>>,
    next_id: AtomicI64,
}

/// Aprovação manual de um nó do grafo. Ninguém aprova por conta própria: sem a
/// decisão explícita na janela o nó não toca o destino, e a espera tem prazo.
#[derive(Clone, Default)]
pub struct ApprovalBroker {
    inner: Arc<ApprovalInner>,
}

impl ApprovalBroker {
    /// Emite o pedido e espera. Só retorna com uma decisão, com o prazo vencido
    /// ou com a corrida interrompida — nunca "aprovado porque ninguém respondeu".
    pub fn wait(&self, app: &AppHandle, pedido: PedidoDeAprovacao, cancel: &AtomicBool) -> Decision {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let Ok(mut pending) = self.inner.pending.lock() else {
                return Decision::Cancelled;
            };
            pending.insert(id, tx);
        }
        let _ = app.emit(
            "pipeline-approval",
            ApprovalRequest {
                id,
                run_id: pedido.run_id,
                node_key: pedido.node_key,
                title: pedido.title,
                definition: pedido.definition,
                environment: pedido.environment,
                destination: pedido.destination,
                waits_secs: APPROVAL_TIMEOUT.as_secs(),
            },
        );
        let inicio = std::time::Instant::now();
        let decision = loop {
            match rx.recv_timeout(APPROVAL_SLICE) {
                Ok(decision) => break decision,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Decision::Cancelled,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if cancel.load(Ordering::SeqCst) {
                        break Decision::Cancelled;
                    }
                    if inicio.elapsed() >= APPROVAL_TIMEOUT {
                        break Decision::Expired;
                    }
                }
            }
        };
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.remove(&id);
        }
        decision
    }

    /// A decisão de quem usa na janela.
    pub fn respond(&self, id: i64, approved: bool) -> Result<(), String> {
        let tx = self
            .inner
            .pending
            .lock()
            .map_err(|_| "o registro de aprovações está indisponível")?
            .remove(&id)
            .ok_or_else(|| "não há mais nenhum nó esperando esta aprovação".to_string())?;
        tx.send(if approved { Decision::Approved } else { Decision::Denied })
            .map_err(|_| "o nó que esperava a aprovação já encerrou".to_string())
    }

    /// Quantos nós estão parados esperando decisão.
    pub fn pending(&self) -> usize {
        self.inner
            .pending
            .lock()
            .map(|p| p.len())
            .unwrap_or_default()
    }
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
#[derive(Clone)]
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

#[derive(Debug)]
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

/// O bloco que uma chamada de log manda para o banco: o nome de stream do esquema
/// (`out`/`err` viram `stdout`/`stderr`, o resto é `system`), as linhas que têm
/// texto, e — quando o teto da tentativa é cruzado — uma última linha `system`
/// declarando o corte. Separado da gravação para o corte poder ser conferido sem
/// motor nenhum, e para a gravação receber só texto já mascarado. Visível ao módulo
/// do banco porque é aqui, e não na tabela, que o corte e o nome do fluxo significam
/// alguma coisa.
pub(crate) fn bloco_de_log(stream: &str, masked: &str, gravadas: i64) -> Vec<(String, String)> {
    if gravadas >= TETO_LINHAS_POR_TENTATIVA {
        // O teto já foi declarado no bloco; repetir o aviso linha a linha seria o
        // corte fingindo tamanho.
        return Vec::new();
    }
    let nome = match stream {
        "out" => "stdout",
        "err" => "stderr",
        _ => "system",
    };
    let livres = (TETO_LINHAS_POR_TENTATIVA - gravadas) as usize;
    let visiveis: Vec<&str> = masked
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let mut bloco: Vec<(String, String)> = visiveis
        .iter()
        .take(livres)
        .map(|text| (nome.to_string(), text.to_string()))
        .collect();
    if visiveis.len() > livres {
        bloco.push((
            "system".to_string(),
            format!(
                "… truncado em {TETO_LINHAS_POR_TENTATIVA} linha(s) por tentativa — {} linha(s) além do teto não foram gravadas",
                visiveis.len() - livres
            ),
        ));
    }
    bloco
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
    /// Porta da aprovação manual pedida por um nó do grafo. Vive no
    /// [`DeployState`] porque é estado de execução, como o cancelamento.
    approvals: Arc<ApprovalBroker>,
    provided: HashMap<String, String>,
    extra: Vec<String>,
    allow_secrets: bool,
    askpass: Option<Arc<auth::AskPass>>,
    secrets: Secrets,
    shell: String,
    files: Vec<LocalFile>,
    /// Comparação origem↔destino já feita, guardada entre as etapas de backup e
    /// de cópia para o mesmo conteúdo ser lido uma única vez.
    plano: Option<(Vec<LocalFile>, usize)>,
    release: Option<String>,
    previous: Option<String>,
    tools: HashMap<String, Option<String>>,
    /// O que o executor de runtime do modelo deixa no ar: a imagem que estava
    /// rodando antes desta corrida, lida do `ANTERIOR:` que o roteiro imprimiu. É o
    /// único destino possível de uma reversão de container, e numa corrida de
    /// reversão chega do histórico como o valor a voltar.
    imagem_anterior: Option<String>,
    branch: String,
    remote_url: String,
    sent: usize,
    unchanged: usize,
    removed: usize,
    backup: Option<String>,
    confirmation: String,
    /// A linha do histórico que o nó do grafo em execução ocupa. `None` em
    /// qualquer execução linear: é assim que `mark` sabe se escreve na posição da
    /// etapa do motor ou na posição do nó.
    node_row: Option<NodeRow>,
    /// Prazo do nó em execução; `None` herda o prazo por etapa do modelo.
    node_timeout: Option<Duration>,
    /// Uma corrida de reversão não pôs código novo no destino: as etapas que mudam
    /// o runtime ficam quietas e só a leitura de saúde confere o resultado.
    reversao: bool,
    /// Qual tentativa desta etapa está no ar, começando em 1. É o que separa o bloco
    /// de log de uma tentativa do bloco da seguinte: sem ele, a repetição de um nó
    /// escrevia por cima da falha que a motivou.
    attempt: i64,
}

impl Ctx {
    fn timeout(&self) -> Duration {
        self.node_timeout
            .unwrap_or_else(|| Duration::from_secs(self.profile.step_timeout_secs.max(10) as u64))
    }

    /// Cópia do contexto para um nó que roda ao mesmo tempo que um irmão. Cada
    /// ramal tem o seu `Ctx` e a sua linha no histórico, e é por isso que só um
    /// tipo de nó pode ser paralelo: quem escreve no contexto (as etapas do motor,
    /// que produzem a lista de arquivos, a release e o backup) nunca divide ramal.
    fn branch(&self, row: NodeRow, node_timeout: Option<Duration>) -> Ctx {
        Ctx {
            app: self.app.clone(),
            db: self.db.clone(),
            run_id: self.run_id,
            cancel: Arc::clone(&self.cancel),
            touched: AtomicU16::new(self.touched.load(Ordering::Relaxed)),
            profile: self.profile.clone(),
            server: self.server.clone(),
            project: self.project.clone(),
            root: self.root.clone(),
            broker: self.broker.clone(),
            approvals: Arc::clone(&self.approvals),
            provided: self.provided.clone(),
            extra: self.extra.clone(),
            allow_secrets: self.allow_secrets,
            askpass: self.askpass.clone(),
            secrets: self.secrets.clone(),
            shell: self.shell.clone(),
            files: self.files.clone(),
            plano: None,
            release: self.release.clone(),
            previous: self.previous.clone(),
            tools: self.tools.clone(),
            imagem_anterior: self.imagem_anterior.clone(),
            branch: self.branch.clone(),
            remote_url: self.remote_url.clone(),
            sent: self.sent,
            unchanged: self.unchanged,
            removed: self.removed,
            backup: self.backup.clone(),
            confirmation: self.confirmation.clone(),
            node_row: Some(row),
            node_timeout,
            reversao: self.reversao,
            // O ramal é outro nó: a tentativa dele recomeça em 1.
            attempt: 1,
        }
    }

    /// O executor de runtime desta corrida: as colunas do modelo traduzidas uma vez
    /// só (`DeployProfile::alvo`), com o diretório onde o código está no destino
    /// agora. A etiqueta da imagem é o número da corrida: duas execuções da mesma
    /// definição não se atropelam no depósito de imagens, e a reversão tem um nome
    /// para voltar.
    fn executor(&self) -> runtime::Alvo {
        let alvo = pasta_do_executor(&self.base(), self.profile.releases_enabled, self.local_mode());
        self.profile
            .alvo(&alvo)
            .com_tag(&format!("r{}", self.run_id))
    }

    /// O ambiente que as condições dos nós comparam: o do servidor cadastrado,
    /// ou `local` quando a publicação acontece nesta máquina.
    fn environment(&self) -> String {
        self.server
            .as_ref()
            .map(|s| s.environment.clone())
            .unwrap_or_else(|| "local".to_string())
    }

    /// O que uma corrida linear preenche desde o clique: as quinze etapas do motor,
    /// cada uma na sua posição fixa no histórico.
    fn mark_waiting_do_motor(&self) -> usize {
        for position in 1..=15 {
            self.mark(position, "waiting", None, None, None);
        }
        15
    }

    /// O que uma corrida de grafo preenche desde o clique: as linhas dos nós que
    /// ela percorre, na ordem salva. Devolve quantas são — zero é um grafo sem
    /// nada para executar, e a corrida não abre.
    fn mark_waiting_dos_nos(
        &self,
        nos: &[PipelineNode],
        alvo: Option<&str>,
        com_dependencias: bool,
    ) -> usize {
        let lista = escopo_do_grafo(nos, alvo, com_dependencias);
        for &i in &lista {
            self.marca_no(&NodeRow::of(i, &nos[i]), "waiting", None);
        }
        lista.len()
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
    ///
    /// Numa execução de grafo a posição que o motor conhece (a etapa 10, por
    /// exemplo) é traduzida para a linha do nó em execução, com o nome e o
    /// identificador dele — assim uma etapa do motor funciona igual na linha
    /// salvante e dentro de um grafo desenhado na tela.
    fn mark(&self, position: i64, status: &str, message: Option<&str>, command: Option<&str>, exit_code: Option<i64>) {
        // O registro de quais etapas do motor foram alcançadas só existe para as
        // quinze posições do pipeline linear; um nó de grafo fora delas não tem bit.
        if (1..=15).contains(&position) {
            self.touched.fetch_or(1u16 << position, Ordering::Relaxed);
        }
        let linha = match &self.node_row {
            Some(row) => Linha::from(row),
            None => Linha {
                position,
                nome: if self.local_mode() { step_name_local(position) } else { step_name(position) },
                chave: None,
                tocando: false,
            },
        };
        self.registra(&linha, status, message, command, exit_code);
    }

    /// Marca a linha de um nó do grafo. Nó sem etapa do motor por trás (um comando,
    /// uma aprovação, um pulado por dependência) passa por aqui, não por `mark`.
    fn marca_no(&self, row: &NodeRow, status: &str, message: Option<&str>) {
        self.registra(&Linha::from(row), status, message, None, None);
    }

    /// O destino de toda etapa: histórico no banco e evento na tela, com os
    /// segredos mascaras antes de sair do motor.
    fn registra(&self, linha: &Linha, status: &str, message: Option<&str>, command: Option<&str>, exit_code: Option<i64>) {
        let message = message.map(|m| self.secrets.mask(m));
        let command = command.map(|c| self.secrets.mask(c));
        let falha_de_registro = if let Ok(conn) = self.db.lock() {
            db::upsert_step(
                &conn,
                &db::StepUpdate {
                    run_id: self.run_id,
                    position: linha.position,
                    name: &linha.nome,
                    status,
                    command: command.as_deref(),
                    exit_code,
                    message: message.as_deref(),
                    node_key: linha.chave.as_deref(),
                    touched_destination: linha.tocando,
                },
            )
            .err()
        } else {
            None
        };
        // Fora do lock: `log` também escreve no banco, e o mutex não é reentrante.
        if let Some(e) = falha_de_registro {
            // Etapa sem registro no histórico é dado perdido: mostra na tela.
            self.log(linha.position, "err", &format!("falha ao registrar a etapa: {e}"));
        }
        let _ = self.app.emit(
            "deploy-step",
            StepEvent {
                run_id: self.run_id,
                position: linha.position,
                name: linha.nome.clone(),
                status: status.to_string(),
                command,
                exit_code,
                message,
                node_key: linha.chave.clone(),
            },
        );
    }

    fn log(&self, position: i64, stream: &str, text: &str) {
        let masked = self.secrets.mask(text);
        // O log pertence à mesma linha do histórico que a etapa está escrevendo.
        let linha = self
            .node_row
            .as_ref()
            .map(|row| row.position)
            .unwrap_or(position);
        self.gravar_log(linha, stream, &masked);
        for line in masked.lines().take(200) {
            if line.trim().is_empty() {
                continue;
            }
            let _ = self.app.emit(
                "deploy-log",
                LogEvent {
                    run_id: self.run_id,
                    position: linha,
                    stream: stream.to_string(),
                    text: line.to_string(),
                },
            );
        }
    }

    /// A mesma saída mascarada que a tela recebe fica no banco, antes do evento: é
    /// assim que reabrir a janela tem o que reler, e o que a tela mostra passa a ser
    /// o que está gravado. A máscara é a de `Secrets::mask` — o valor de cada
    /// variável desta corrida some primeiro, depois os formatos conhecidos de
    /// segredo — por isso nenhum valor chega ao disco. Um bloco por tentativa: a
    /// repetição de um nó não apaga a falha que a motivou.
    fn gravar_log(&self, position: i64, stream: &str, masked: &str) {
        let gravacao = || -> Result<(), String> {
            let conn = self
                .db
                .lock()
                .map_err(|e| format!("o banco está indisponível: {e}"))?;
            let gravadas = db::count_step_logs(&conn, self.run_id, position, self.attempt)?;
            let bloco = bloco_de_log(stream, masked, gravadas);
            let linhas: Vec<db::LinhaDeLog<'_>> = bloco
                .iter()
                .map(|(s, t)| db::LinhaDeLog {
                    stream: s.as_str(),
                    text: t.as_str(),
                })
                .collect();
            db::append_step_logs(&conn, self.run_id, position, self.attempt, &linhas)
        };
        if let Err(e) = gravacao() {
            let _ = self.app.emit(
                "deploy-log",
                LogEvent {
                    run_id: self.run_id,
                    position,
                    stream: "err".to_string(),
                    text: format!("falha ao gravar o log da etapa: {e}"),
                },
            );
        }
    }

    /// A senha por operação é preparada uma única vez por corrida. Na fila de quinze
    /// etapas isso acontece na validação; num grafo desenhado sem essa etapa, é o
    /// primeiro contato com o servidor que prepara.
    fn garantir_askpass(&mut self) -> Result<(), Fail> {
        if self.askpass.is_some() || self.local_mode() {
            return Ok(());
        }
        let servidor = self.server_ref()?.clone();
        self.askpass = auth::prepare(
            &self.broker,
            &self.app,
            &servidor,
            &format!("Deploy · {}", self.profile.name),
        )
        .map_err(Fail::new)?
        .map(Arc::new);
        Ok(())
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
        self.garantir_askpass()?;
        let servidor = self.server_ref()?.clone();
        let timeout = self.timeout();
        let result = sshexec::run_script(
            &servidor,
            self.askpass.as_deref(),
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
                if detail.is_empty() {
                    result.summary()
                } else {
                    detail
                },
            ));
        }
        Ok(result)
    }

    /// Upload em lote numa única sessão sftp, com prazo e cancelamento.
    fn upload(&mut self, pares: &[(PathBuf, String)], label: &str) -> Result<(), Fail> {
        if self.cancelled() {
            return Err(Fail::cancelled());
        }
        self.garantir_askpass()?;
        let servidor = self.server_ref()?.clone();
        let timeout = self.timeout();
        files::put_chunk(
            &servidor,
            pares,
            self.askpass.as_deref(),
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

// ---------- grafo do pipeline (plano CI/CD, etapa 3) ----------

/// A linha do histórico que um nó ocupa durante a corrida. Com ela, `mark` escreve
/// no nó e não na posição fixa da etapa do motor, e uma mesma etapa do motor serve
/// ao pipeline linear e ao grafo desenhado na tela.
#[derive(Debug, Clone)]
struct NodeRow {
    position: i64,
    key: String,
    title: String,
    touched: bool,
}

impl NodeRow {
    fn of(index: usize, no: &PipelineNode) -> Self {
        Self {
            position: index as i64 + 1,
            key: no.node_key.clone(),
            title: no.title.clone(),
            touched: no.touches_destination,
        }
    }
}

/// A linha do histórico que uma marcação escreve: posição, nome exibido, o nó do
/// grafo por trás dela (se houver) e se ela toca o destino.
#[derive(Debug, Clone)]
struct Linha {
    position: i64,
    nome: String,
    chave: Option<String>,
    tocando: bool,
}

impl From<&NodeRow> for Linha {
    fn from(row: &NodeRow) -> Self {
        Self {
            position: row.position,
            nome: row.title.clone(),
            chave: Some(row.key.clone()),
            tocando: row.touched,
        }
    }
}

/// Uma etapa do motor: a função que a janela não conhece, só o número dela.
type Etapa = fn(&mut Ctx) -> Result<Info, Fail>;

/// As quinze etapas do motor, endereçadas pelo número que a definição usa. É a
/// única tabela de execução: o grafo não conhece comando nenhum, só estas funções.
fn etapa_do_motor(position: i64) -> Option<Etapa> {
    Some(match position {
        1 => validate_step,
        2 => connection_step,
        3 => git_step,
        4 => files_step,
        5 => secrets_step,
        6 => tests_step,
        7 => build_step,
        8 => prepare_step,
        9 => backup_step,
        10 => send_step,
        11 => update_step,
        12 => restart_step,
        13 => health_step,
        14 => confirm_step,
        15 => register_step,
        _ => return None,
    })
}

/// O que uma etapa do motor precisa ter rodado antes, porque lê o que aquela
/// etapa produziu no contexto. É o que impede um grafo desenhado na tela de pedir
/// "copiar os arquivos" sem que ninguém tenha decidido quais são os arquivos.
pub fn prequisitos(position: i64) -> &'static [i64] {
    match position {
        5..=8 => &[4],
        9..=15 => &[3, 4, 8],
        _ => &[],
    }
}

/// O diretório de trabalho de um nó local, conferido dentro da raiz do projeto.
/// Um caminho que dobra para fora da pasta cadastrada não é aceito — é a mesma
/// contenção que o destino do pipeline faz, na mesma régua canônica e sob o mesmo
/// prazo: abrir a pasta é uma espera de disco, e um volume que não acorda devolve
/// o motivo em vez de pendurar a corrida inteira.
fn dir_do_no(ctx: &Ctx, no: &PipelineNode) -> Result<PathBuf, Fail> {
    let bruto = no.working_dir.as_deref().unwrap_or("").trim();
    if bruto.is_empty() {
        return Ok(ctx.root.clone());
    }
    dir_local(&ctx.root, bruto, &no.title, git_probe::PRAZO_SONDA)
}

fn dir_local(raiz: &Path, bruto: &str, titulo: &str, prazo: Duration) -> Result<PathBuf, Fail> {
    let candidato = raiz.join(bruto.replace('\\', "/"));
    let raiz_real = match git_probe::canon(raiz, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(e) => {
            return Err(Fail::new(format!("não foi possível abrir a pasta do projeto: {e}")))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(Fail::new(motivo)),
    };
    let real = match git_probe::canon(&candidato, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(e) => {
            return Err(Fail::new(format!(
                "a pasta de trabalho “{}” do nó “{}” não existe dentro do projeto: {e}",
                bruto, titulo
            )))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(Fail::new(motivo)),
    };
    if !real.starts_with(&raiz_real) {
        return Err(Fail::new(format!(
            "a pasta de trabalho “{}” do nó “{}” sai do projeto — o motor só roda dentro da pasta cadastrada",
            bruto, titulo
        )));
    }
    Ok(real)
}

/// Roda os comandos de um nó no executor que ele escolheu. Nenhum comando é
/// montado por concatenação com entrada solta: localmente cada linha vai ao
/// interpretador desta máquina com o diretório e as variáveis do deploy; no
/// servidor o lote vira um script POSIX com falha rápida pela stdin da sessão.
fn rodar_comandos(ctx: &mut Ctx, no: &PipelineNode, posicao: i64) -> Result<usize, Fail> {
    let linhas: Vec<&str> = no
        .commands
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if linhas.is_empty() {
        return Err(Fail::new(format!(
            "o nó “{}” ficou sem comando",
            no.title
        )));
    }
    // Nas quinze etapas em fila é `step` quem marca a linha como em execução, e é essa
    // marca que grava a hora de início. Um nó de comando não passa por `step`, então
    // marca aqui — sem isso o histórico do nó não tem início nem duração.
    ctx.mark(posicao, "running", Some("em execução"), None, None);
    if no.executor == "ssh" {
        let mut script = ctx.script(&format!("# {}\n", no.title));
        if let Some(dir) = no.working_dir.as_deref().map(str::trim) {
            if !dir.is_empty() {
                script.push_str(&format!("cd {}\n", sshexec::shell_quote(dir)));
            }
        }
        for linha in &linhas {
            script.push_str(&format!(
                "echo {}\n{linha}\n",
                sshexec::shell_quote(&format!("+ {linha}"))
            ));
        }
        ctx.remote(
            posicao,
            &script,
            &format!("{} ({} comando(s))", no.title, linhas.len()),
        )?;
        return Ok(linhas.len());
    }
    let dir = dir_do_no(ctx, no)?;
    for linha in &linhas {
        ctx.log(posicao, "out", &format!("+ {linha}"));
        if ctx.cancelled() {
            return Err(Fail::cancelled());
        }
        let timeout = ctx.timeout();
        let env = ctx.secrets.env();
        let resultado = sshexec::run_local_with_env(linha, &dir, &env, timeout, &ctx.cancel)
            .map_err(Fail::new)?;
        ctx.log(posicao, "out", &resultado.stdout);
        ctx.log(posicao, "err", &resultado.stderr);
        if !resultado.ok() {
            let detalhe = tail(&resultado.stderr, &resultado.stdout);
            return Err(Fail::failed(
                linha.to_string(),
                Some(resultado.exit_code.unwrap_or(-1) as i64),
                // Um comando que falha sem escrever nada em lugar nenhum não pode
                // deixar a linha do histórico sem motivo: fica o resumo do processo.
                if detalhe.is_empty() {
                    resultado.summary()
                } else {
                    detalhe
                },
            ));
        }
    }
    Ok(linhas.len())
}

/// A linha de um nó que não traz mensagem própria: o que ele era, não o que o
/// motor acha dele.
fn resumo_do_no(no: &PipelineNode) -> String {
    match no.kind.as_str() {
        "aprovacao" => "aprovado na janela".to_string(),
        _ => format!(
            "{} comando(s) {}",
            no.commands.iter().filter(|c| !c.trim().is_empty()).count(),
            if no.executor == "ssh" { "no servidor" } else { "nesta máquina" }
        ),
    }
}

/// A primeira linha que o git escreveu, lida da resposta dele. Um nó que terminou
/// "Already up to date." diz isso no histórico; um nó que diz "atualizado" sem ter
/// lido nada mente.
fn frase_do_git(saida: &str) -> String {
    saida
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("o git não respondeu nada")
        .to_string()
}

/// A operação pedida por um nó `git`, separada do contexto para poder ser conferida
/// num repositório real nos testes. O git é o desta máquina — o mesmo da aba Git —
/// e o que volta é a resposta dele, lida depois da operação.
fn git_do_no(dir: &Path, operacao: &str, referencia: &str) -> Result<String, String> {
    match operacao {
        // `--ff-only`: um nó de publicação não inventa merge no histórico do projeto.
        // Se não dá para avançar reto, é o git que diz e o nó para.
        "pull" => {
            let saida = git::pull(dir, "ff-only")?;
            Ok(frase_do_git(&saida))
        }
        "checkout" => {
            git::checkout(dir, referencia, false, None)?;
            // A cabeça lida depois, não a que foi pedida: alternar para um tag deixa
            // o repositório destacado, e dizer o contrário seria falso.
            let estado = git::status(dir)?;
            let onde = match (estado.branch, estado.detached_at) {
                (Some(ramo), _) => Some(ramo),
                (None, Some(c)) => Some(format!("cabeça destacada em {}", &c[..c.len().min(7)])),
                (None, None) => None,
            };
            Ok(match onde {
                Some(texto) => format!("em {texto}"),
                None => "alternado, mas a pasta não informou onde está".to_string(),
            })
        }
        "tag" => {
            git::create_tag(dir, referencia, None, None)?;
            let lidas = git::tags(dir)?;
            if !lidas.iter().any(|t| t.name == referencia) {
                return Err(format!("o git não registrou a tag {referencia}"));
            }
            // Publicar a marca só tem sentido onde há para onde publicar; sem remoto
            // a tag fica nesta máquina, e a linha diz isso em vez de emendar.
            let remotos = git::remotes(dir)?;
            match remotos.iter().find(|r| r.name == "origin").or_else(|| remotos.first()) {
                None => Ok(format!("tag {referencia} criada nesta máquina — sem remoto cadastrado")),
                Some(remoto) => {
                    let envio = git::push_tag(dir, &remoto.name, referencia)?;
                    Ok(format!(
                        "tag {referencia} publicada em {} · {}",
                        remoto.name,
                        frase_do_git(&envio)
                    ))
                }
            }
        }
        outro => Err(format!(
            "o nó git roda {}, não “{outro}”",
            crate::models::NODE_GIT_OPS.join(", ")
        )),
    }
}

/// O nó `git` roda nesta máquina, na pasta do projeto (contida por `dir_do_no`): é
/// o repositório de onde a publicação sai, não o destino. Nenhum comando é montado
/// por texto — a operação vem da definição e o git obedece aos próprios argumentos.
fn rodar_no_git(ctx: &mut Ctx, no: &PipelineNode, posicao: i64) -> Result<String, Fail> {
    let operacao = no.git_op.as_deref().unwrap_or("").trim();
    if operacao.is_empty() {
        return Err(Fail::new(format!("o nó “{}” ficou sem operação de git", no.title)));
    }
    let referencia = no.git_ref.as_deref().unwrap_or("").trim();
    let dir = dir_do_no(ctx, no)?;
    ctx.mark(posicao, "running", Some("em execução"), None, None);
    ctx.log(posicao, "out", &format!("+ git {operacao} {}", referencia));
    let feito = git_do_no(&dir, operacao, referencia)
        .map_err(|e| Fail::failed(format!("git {operacao}"), None, e))?;
    ctx.log(posicao, "out", &feito);
    Ok(feito)
}

/// O nó `analise` varre o código do projeto nesta máquina e responde a uma única
/// pergunta: o corte declarado na tela se cumpriu. O código de saída do semgrep
/// não entra nessa conta — o que barra é a contagem lida do banco contra o corte
/// do nó. E uma varredura que não conseguiu ler nada também para o nó: "não sei"
/// nunca é um "está tudo bem". A varredura em si é a mesma da tela de Análise
/// ([`crate::analysis::executar`]), chamada pela segunda porta.
fn rodar_no_analise(ctx: &mut Ctx, no: &PipelineNode, posicao: i64) -> Result<String, Fail> {
    let entrada = AnalysisInput {
        project_id: ctx.project.id,
        analyzer: Some(no.analyzer.clone()),
        severity_gate: Some(no.severity_gate.clone()),
        path: no.working_dir.clone(),
        timeout_secs: Some(no.timeout_secs),
    };
    let corte = entrada.severity_gate();
    let dados = db::pasta_de_dados(&ctx.app).map_err(Fail::new)?;
    let raiz = analysis::raiz_do_projeto(&ctx.project).map_err(Fail::new)?;
    ctx.mark(posicao, "running", Some("em execução"), None, None);
    ctx.log(
        posicao,
        "out",
        &format!(
            "+ análise de código · {} · {}",
            entrada.analyzer(),
            if corte.is_empty() { "sem corte".to_string() } else { format!("corte {corte}") }
        ),
    );
    let run = {
        let conn = ctx
            .db
            .lock()
            .map_err(|_| Fail::new("o banco não respondeu durante a varredura"))?;
        analysis::executar(&conn, &dados, &ctx.project, &raiz, &entrada, &ctx.cancel)
            .map_err(Fail::new)?
    };
    ctx.log(
        posicao,
        "out",
        &format!(
            "{} arquivo(s) lido(s) · {} achado(s): {} erro, {} aviso, {} info",
            run.files_scanned, run.counts.total(), run.counts.error, run.counts.warning, run.counts.info
        ),
    );
    if let Some(nota) = run.note.as_deref() {
        ctx.log(posicao, "err", nota);
    }
    veredito_do_no(&run)
}

/// O veredito que o nó `analise` dá a uma corrida já gravada: barra quando o corte
/// foi atingido, devolve o cancelamento quando a varredura foi cortada no meio,
/// falha quando ela não pôde ser lida, e só então conclui com a linha que fica no
/// histórico. Separado do motor porque depende apenas da corrida — é a mesma
/// resposta que a tela de Análise mostra, e assim ela pode ser conferida sem uma
/// janela nem um semgrep na máquina.
fn veredito_do_no(run: &AnalysisRun) -> Result<String, Fail> {
    let corte = if run.severity_gate.is_empty() {
        "sem corte".to_string()
    } else {
        run.severity_gate.clone()
    };
    let texto = format!(
        "{} achado(s) com {} arquivo(s) lidos — {}",
        run.counts.total(),
        run.files_scanned,
        if run.blocked {
            format!("o corte {corte} foi atingido")
        } else if run.severity_gate.is_empty() {
            "nada barra".to_string()
        } else {
            format!("dentro do corte {corte}")
        }
    );
    // Uma varredura interrompida não terminou: é o mesmo estado de um comando
    // cortado, e o motor tem de tratá-la como cancelamento — senão o nó fica
    // "concluído" com metade da árvore lida e os dependentes seguem.
    if run.status == "interrompida" {
        return Err(Fail::cancelled());
    }
    if run.blocked {
        return Err(Fail::failed(
            format!("análise de código (corte {corte})"),
            None,
            format!(
                "{} · erro {}, aviso {}, info {} — a varredura de {} barra a publicação",
                run.tool, run.counts.error, run.counts.warning, run.counts.info, run.root
            ),
        ));
    }
    // Sem corte o nó informa, mas uma varredura que falhou — ou que não chegou a
    // um estado terminal — não informa coisa nenhuma: é falha dita com a nota de
    // quem varreu, nunca um sucesso presumido.
    if run.status != "concluida" {
        return Err(Fail::failed(
            format!("análise de código ({})", run.tool),
            None,
            run.note.clone().unwrap_or_else(|| match run.status.as_str() {
                "falhou" => "a varredura não conseguiu ler o código".to_string(),
                outro => format!("a varredura terminou no estado “{outro}”, que não é uma conclusão"),
            }),
        ));
    }
    Ok(texto)
}

/// Como um nó terminou. `Pulado` é diferente de falha: a condição do nó não se
/// cumpriu, então nada foi executado e nada está errado.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fim {
    /// O texto que a linha do histórico mostra. `None` é o resumo padrão do tipo do
    /// nó; um nó `git` traz a resposta do próprio git, que não cabe num resumo.
    Concluido(Option<String>),
    Pulado,
}

/// Roda um nó até o fim: condição, aprovação, execução e repetição. Marca a linha
/// do histórico correspondente e devolve `Err` quando o nó falhou.
fn executar_no(ctx: &mut Ctx, index: usize, no: &PipelineNode) -> Result<Fim, Fail> {
    let row = NodeRow::of(index, no);
    ctx.node_row = Some(row.clone());
    ctx.attempt = 1;
    ctx.node_timeout = if no.timeout_secs > 0 {
        Some(Duration::from_secs(no.timeout_secs as u64))
    } else {
        None
    };
    let resultado = executar_conteudo(ctx, no, &row);
    ctx.node_row = None;
    ctx.node_timeout = None;
    // A linha final do nó é escrita a partir do próprio nó: `mark` nesta altura já não
    // tem `node_row`, e pela posição do grafo cairia na tabela das quinze etapas — um nó
    // de comando chamado `preparo` ganharia o nome de uma etapa do motor.
    match resultado {
        Ok(fim @ Fim::Concluido(_)) => {
            // Uma etapa do motor já escreveu a própria linha final dentro de `step`,
            // com nome, status e a mensagem do resultado — reescrever aqui apagaria a
            // mensagem dela e transformaria um "pulada" em "concluída". Os nós sem
            // etapa por trás terminam por aqui, com a mensagem que o corpo devolveu
            // (o git responde pela própria linha) ou com o resumo dos comandos.
            if no.kind != "motor" {
                let texto = match &fim {
                    Fim::Concluido(Some(mensagem)) => mensagem.clone(),
                    _ => resumo_do_no(no),
                };
                ctx.registra(&Linha::from(&row), "done", Some(&texto), None, None);
            }
            Ok(fim)
        }
        // A linha já foi marcada como pulada, com o motivo.
        Ok(Fim::Pulado) => Ok(Fim::Pulado),
        Err(fail) => {
            let mensagem = if no.retry_limit > 0 {
                format!("{} — {} tentativa(s) usada(s)", fail.message, no.retry_limit + 1)
            } else {
                fail.message.clone()
            };
            ctx.registra(
                &Linha::from(&row),
                "failed",
                Some(&mensagem),
                fail.command.as_deref(),
                fail.exit_code,
            );
            Err(fail)
        }
    }
}

/// A parte de um nó que pode falhar e ser repetida, separada para o laço de
/// repetição não repetir a aprovação nem a checagem de condição.
fn executar_conteudo(ctx: &mut Ctx, no: &PipelineNode, row: &NodeRow) -> Result<Fim, Fail> {
    if ctx.cancelled() {
        return Err(Fail::cancelled());
    }
    if !condicao_atendida(ctx, no) {
        ctx.mark(
            row.position,
            "skipped",
            Some(&format!(
                "condição “{}” não atendida",
                no.condition.as_deref().unwrap_or("")
            )),
            None,
            None,
        );
        return Ok(Fim::Pulado);
    }
    let precisa_aprovar = no.requires_approval || no.kind == "aprovacao";
    if precisa_aprovar {
        ctx.mark(row.position, "running", Some("esperando aprovação"), None, None);
        let decisao = ctx.approvals.wait(
            &ctx.app,
            PedidoDeAprovacao {
                run_id: ctx.run_id,
                node_key: no.node_key.clone(),
                title: no.title.clone(),
                definition: ctx.profile.name.clone(),
                environment: ctx.environment(),
                destination: ctx.base(),
            },
            &ctx.cancel,
        );
        registrar_aprovacao(ctx, no, decisao);
        match decisao {
            Decision::Approved => {}
            Decision::Denied => {
                return Err(Fail::new(format!(
                    "o nó “{}” foi rejeitado na janela — nada que ele faria foi executado",
                    no.title
                )))
            }
            Decision::Expired => {
                return Err(Fail::new(format!(
                    "ninguém respondeu ao pedido de aprovação do nó “{}” dentro do prazo de {} minutos",
                    no.title,
                    APPROVAL_TIMEOUT.as_secs() / 60
                )))
            }
            Decision::Cancelled => return Err(Fail::cancelled()),
        }
    }
    if no.kind == "aprovacao" {
        return Ok(Fim::Concluido(None));
    }
    let mut tentativa = 0;
    loop {
        // O número da tentativa é do bloco de log, não do laço: cada repetição
        // escreve o seu, e o aviso de "repetindo o nó" fica no bloco da tentativa
        // que falhou, que é onde ele faz sentido.
        ctx.attempt = tentativa + 1;
        let resultado: Result<Option<String>, Fail> = match no.kind.as_str() {
            "motor" => {
                let Some(number) = no.engine_step else {
                    return Err(Fail::new(format!(
                        "o nó “{}” não aponta para nenhuma etapa do motor",
                        no.title
                    )));
                };
                let Some(etapa) = etapa_do_motor(number) else {
                    return Err(Fail::new(format!(
                        "o nó “{}” aponta para a etapa {number}, que o motor não tem",
                        no.title
                    )));
                };
                // `step` é a mesma função que o pipeline linear usa: marca a linha
                // do nó, escreve a mensagem da etapa e devolve o erro real.
                step(ctx, number, etapa).map(|_| None)
            }
            "comando" => rodar_comandos(ctx, no, row.position).map(|_| None),
            "git" => rodar_no_git(ctx, no, row.position).map(Some),
            "analise" => rodar_no_analise(ctx, no, row.position).map(Some),
            // Um tipo que o motor não sabe executar não pode terminar como concluído:
            // é o nó que faz nada e sorri.
            outro => Err(Fail::new(format!(
                "o nó “{}” é do tipo “{outro}”, que o motor não sabe executar",
                no.title
            ))),
        };
        match resultado {
            Ok(mensagem) => return Ok(Fim::Concluido(mensagem)),
            Err(fail) if fail.cancelled => return Err(fail),
            Err(fail) if tentativa < no.retry_limit => {
                tentativa += 1;
                ctx.log(
                    row.position,
                    "err",
                    &format!(
                        "{} — repetindo o nó (tentativa {}/{})",
                        fail.message, tentativa, no.retry_limit
                    ),
                );
            }
            Err(fail) => return Err(fail),
        }
    }
}

/// O resultado da espera de aprovação vira auditoria: decisão, hora e o que foi
/// autorizado. O nome de quem decidiu é o de quem está diante da janela.
fn registrar_aprovacao(ctx: &Ctx, no: &PipelineNode, decisao: Decision) {
    let texto = match decisao {
        Decision::Approved => "aprovado na janela",
        Decision::Denied => "rejeitado na janela",
        Decision::Expired => "sem resposta dentro do prazo",
        Decision::Cancelled => "interrompido",
    };
    if let Ok(conn) = ctx.db.lock() {
        let _ = audit::log(
            &conn,
            match decisao {
                Decision::Approved => "info",
                Decision::Cancelled => "warning",
                _ => "error",
            },
            "deploy",
            "Aprovação de etapa",
            ctx.server.as_ref().map(|s| s.name.as_str()),
            Some(&ctx.project.name),
            None,
            Some(&format!(
                "modelo {} · nó “{}” · {} · execução #{}",
                ctx.profile.name, no.title, texto, ctx.run_id
            )),
        );
    }
}

/// A condição do nó, avaliada com o que o motor sabe da corrida: o ramo em que o
/// Git está e o ambiente do destino.
fn condicao_atendida(ctx: &Ctx, no: &PipelineNode) -> bool {
    let bruto = match no.condition.as_deref() {
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
        _ => return true,
    };
    let (chave, valor) = match bruto.split_once('=') {
        Some(pair) => pair,
        None => return true,
    };
    match chave.trim() {
        "ramo" => ctx.branch == valor.trim(),
        "ambiente" => ctx.environment().eq_ignore_ascii_case(valor.trim()),
        _ => true,
    }
}

/// Situação de um nó dentro da corrida que está rodando agora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sit {
    Feito,
    /// A condição do nó não se cumpriu: nada foi executado e nada está errado.
    Pulado,
    Falho,
    /// Não rodou porque uma dependência sua falhou — direto ou através de outro nó
    /// travado. É diferente de `Pulado`: aqui a corrida parou antes de chegar nele,
    /// e por isso um nó travado trava os nós abaixo dele também.
    Bloqueado,
}

/// Os índices que a corrida percorre: o grafo inteiro, ou um nó e — quando pedido
/// em modo de diagnóstico — a corrente de dependências dele. Índice é a ordem
/// salva, que é a mesma ordem da tela e a posição da linha no histórico.
fn escopo_do_grafo(nos: &[PipelineNode], alvo: Option<&str>, com_dependencias: bool) -> Vec<usize> {
    let Some(chave) = alvo else {
        return (0..nos.len()).collect();
    };
    let Some(posicao) = nos.iter().position(|n| n.node_key == chave) else {
        return Vec::new();
    };
    if !com_dependencias {
        return vec![posicao];
    }
    let mut visitas = vec![posicao];
    let mut fila = vec![posicao];
    while let Some(atual) = fila.pop() {
        for dep in &nos[atual].depends_on {
            if let Some(i) = nos.iter().position(|n| &n.node_key == dep) {
                if !visitas.contains(&i) {
                    visitas.push(i);
                    fila.push(i);
                }
            }
        }
    }
    visitas.sort_unstable();
    visitas
}

/// Todo dependente do escopo já terminou (bem ou pulado por condição) e o nó pode
/// rodar. Dependência fora do escopo não trava nada: é o caso da execução isolada
/// de um nó de comando, em que quem testa escolhe rodar aquele nó sozinho.
fn pronto(
    no: &PipelineNode,
    situacao: &HashMap<usize, Sit>,
    indice_por_chave: &HashMap<&str, usize>,
    dentro: &HashSet<usize>,
) -> bool {
    no.depends_on.iter().all(|dep| {
        let Some(i) = indice_por_chave.get(dep.as_str()).copied() else {
            return true;
        };
        if !dentro.contains(&i) {
            return true;
        }
        matches!(situacao.get(&i), Some(Sit::Feito) | Some(Sit::Pulado))
    })
}

/// Dependência que falhou — ou que travou por outra que falhou: o nó não roda, e
/// isso é registrado como travado com o motivo, nunca como sucesso. Sem o
/// `Bloqueado` na conta, a falha pararia só o filho direto e o neto do nó que
/// falhou publicaria do mesmo jeito.
fn bloqueado(
    no: &PipelineNode,
    situacao: &HashMap<usize, Sit>,
    indice_por_chave: &HashMap<&str, usize>,
    dentro: &HashSet<usize>,
) -> bool {
    no.depends_on.iter().any(|dep| {
        let quem_ela_espera = indice_por_chave
            .get(dep.as_str())
            .copied()
            .filter(|i| dentro.contains(i))
            .and_then(|i| situacao.get(&i));
        matches!(
            quem_ela_espera,
            Some(&Sit::Falho) | Some(&Sit::Bloqueado)
        )
    })
}

/// O escalonador do grafo: cada nó roda quando as dependências dele terminaram,
/// ramais paralelos ao mesmo tempo, e nada depois de uma dependência falhar. Não
/// há um segundo motor — um nó do tipo `motor` chama a mesma função das quinze
/// etapas, e um nó `comando` usa o mesmo executor de comandos do contexto.
fn grafo(
    ctx: &mut Ctx,
    nos: &[PipelineNode],
    alvo: Option<&str>,
    com_dependencias: bool,
) -> Result<String, Fail> {
    let lista = escopo_do_grafo(nos, alvo, com_dependencias);
    if lista.is_empty() {
        return Err(Fail::new(match alvo {
            Some(chave) => format!("o nó “{chave}” não pertence a esta definição"),
            None => "a definição não tem nenhum nó para executar".to_string(),
        }));
    }
    for &i in &lista {
        ctx.marca_no(&NodeRow::of(i, &nos[i]), "waiting", None);
    }
    let dentro: HashSet<usize> = lista.iter().copied().collect();
    let indice_por_chave: HashMap<&str, usize> = nos
        .iter()
        .enumerate()
        .map(|(i, n)| (n.node_key.as_str(), i))
        .collect();
    let mut situacao: HashMap<usize, Sit> = HashMap::new();
    let mut restantes: Vec<usize> = lista.clone();
    let mut falha: Option<Fail> = None;
    let mut maior_paralelo = 1usize;

    while !restantes.is_empty() {
        if ctx.cancelled() {
            for &i in &restantes {
                ctx.marca_no(&NodeRow::of(i, &nos[i]), "skipped", Some("corrida interrompida"));
            }
            return Err(Fail::cancelled());
        }
        // A cascada é fechada até não mudar mais: um nó travado por falha trava os
        // nós abaixo dele, e isso só aparece na volta seguinte se não for resolvido
        // aqui dentro.
        let mut apareceram_travados = true;
        while apareceram_travados {
            let bloqueados: Vec<usize> = restantes
                .iter()
                .copied()
                .filter(|&i| bloqueado(&nos[i], &situacao, &indice_por_chave, &dentro))
                .collect();
            apareceram_travados = !bloqueados.is_empty();
            for i in bloqueados {
                ctx.marca_no(
                    &NodeRow::of(i, &nos[i]),
                    "skipped",
                    Some("uma dependência sua falhou nesta corrida"),
                );
                situacao.insert(i, Sit::Bloqueado);
                restantes.retain(|&j| j != i);
            }
        }
        let prontos: Vec<usize> = restantes
            .iter()
            .copied()
            .filter(|&i| pronto(&nos[i], &situacao, &indice_por_chave, &dentro))
            .collect();
        if prontos.is_empty() {
            for &i in &restantes {
                ctx.marca_no(
                    &NodeRow::of(i, &nos[i]),
                    "skipped",
                    Some("as dependências deste nó nunca ficaram prontas"),
                );
            }
            return Err(Fail::new(
                "o grafo parou: existe nó com dependência que não se completa",
            ));
        }
        // Paralelo só entre nós de comando marcados como paralelos. Uma etapa do
        // motor produz estado para as seguintes (arquivos, release, backup) e por
        // isso nunca divide ramal.
        let em_paralelo = prontos.len() > 1
            && prontos
                .iter()
                .all(|&i| nos[i].parallel && nos[i].kind == "comando");
        if em_paralelo {
            maior_paralelo = maior_paralelo.max(prontos.len());
            let mut ramos = Vec::new();
            for &i in &prontos {
                let no = nos[i].clone();
                let row = NodeRow::of(i, &no);
                let prazo = prazo_do_no(&no);
                let mut ramal = ctx.branch(row, prazo);
                let spawned = thread::Builder::new()
                    .name(format!("deploy-{}-no-{}", ctx.run_id, i + 1))
                    .spawn(move || executar_no(&mut ramal, i, &no));
                match spawned {
                    Ok(handle) => ramos.push((i, handle)),
                    Err(e) => {
                        ctx.marca_no(
                            &NodeRow::of(i, &nos[i]),
                            "failed",
                            Some(&format!("não foi possível abrir o ramal: {e}")),
                        );
                        situacao.insert(i, Sit::Falho);
                        restantes.retain(|&j| j != i);
                        falha.get_or_insert_with(|| {
                            Fail::new(format!("falha ao iniciar o ramal paralelo: {e}"))
                        });
                    }
                }
            }
            for (i, handle) in ramos {
                let titulo = nos[i].title.clone();
                let resultado = handle.join().unwrap_or_else(|_| {
                    Err(Fail::new(format!("o ramal “{titulo}” quebrou durante a execução")))
                });
                registrar_saida(&mut situacao, &mut restantes, &mut falha, i, resultado);
            }
            continue;
        }
        let i = prontos[0];
        let resultado = executar_no(ctx, i, &nos[i]);
        if matches!(resultado, Err(ref f) if f.cancelled) {
            for &j in &restantes {
                ctx.marca_no(&NodeRow::of(j, &nos[j]), "skipped", Some("corrida interrompida"));
            }
            return Err(Fail::cancelled());
        }
        registrar_saida(&mut situacao, &mut restantes, &mut falha, i, resultado);
    }

    if let Some(fail) = falha {
        return Err(fail);
    }
    let total = lista.len();
    let feitos = situacao.values().filter(|s| **s == Sit::Feito).count();
    let pulados = situacao.values().filter(|s| **s == Sit::Pulado).count();
    let travados = situacao.values().filter(|s| **s == Sit::Bloqueado).count();
    let mut extra = String::new();
    if pulados > 0 {
        extra.push_str(&format!(" · {pulados} pulado(s) por condição"));
    }
    if travados > 0 {
        extra.push_str(&format!(" · {travados} travado(s) por falha"));
    }
    let ramais = if maior_paralelo > 1 {
        format!(" · até {maior_paralelo} ramais ao mesmo tempo")
    } else {
        String::new()
    };
    Ok(format!("grafo · {feitos} de {total} nós{extra}{ramais}"))
}

/// Prazo do nó, ou `None` para herdar o prazo por etapa do modelo.
fn prazo_do_no(no: &PipelineNode) -> Option<Duration> {
    if no.timeout_secs > 0 {
        Some(Duration::from_secs(no.timeout_secs as u64))
    } else {
        None
    }
}

fn registrar_saida(
    situacao: &mut HashMap<usize, Sit>,
    restantes: &mut Vec<usize>,
    falha: &mut Option<Fail>,
    i: usize,
    resultado: Result<Fim, Fail>,
) {
    match resultado {
        Ok(Fim::Concluido(_)) => situacao.insert(i, Sit::Feito),
        Ok(Fim::Pulado) => situacao.insert(i, Sit::Pulado),
        Err(fail) => {
            if falha.is_none() {
                *falha = Some(fail);
            }
            situacao.insert(i, Sit::Falho)
        }
    };
    restantes.retain(|&j| j != i);
}

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

/// Os dois lados da régua que impede um modelo sem servidor de copiar o projeto
/// para dentro dele mesmo. Cada lado vem na forma canônica que o disco respondeu,
/// resolvido até a pasta mais funda que existe — o destino da primeira publicação
/// ainda não foi criado por ninguém, e é justamente aí que a comparação precisa
/// significar alguma coisa. O que não é opção é esperar sem prazo: abrir a pasta é
/// uma espera de disco, e um volume adormecido devolve o motivo, parando a
/// validação ali em vez de girar para sempre.
fn origem_e_destino(raiz: &Path, destino: &Path, prazo: Duration) -> Result<(PathBuf, PathBuf), String> {
    let origem = match git_probe::canon_ou_ancestral(raiz, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(motivo) => {
            return Err(format!("a pasta local do projeto não pôde ser lida: {motivo}"))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(motivo),
    };
    let alvo = match git_probe::canon_ou_ancestral(destino, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(motivo) => {
            return Err(format!("a pasta de destino não pôde ser lida: {motivo}"))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(motivo),
    };
    Ok((origem, alvo))
}

/// As checagens da etapa 1 que dependem só do que está gravado na definição — mais
/// a leitura dos dois caminhos que a contenção do destino compara, cada uma com
/// prazo. Nada de rede, nada de segredo digitado. São as mesmas que o motor executa
/// antes de qualquer trabalho, e é por isso que o contrato de pipeline
/// (`pipeline_validate`) chama esta função em vez de ter uma lista própria de regras:
/// dois lugares com regras diferentes é como uma tela passa a dizer "ok" para algo
/// que o motor depois recusa.
pub fn checar_definicao(
    profile: &DeployProfile,
    raiz: &Path,
    servidor: Option<&Server>,
) -> Result<(), String> {
    validate_strategy(&profile.strategy)?;
    // O executor de runtime é conferido aqui, pela mesma função que a tela chama ao
    // salvar: o que a tela aceita é exatamente o que o motor executa. E ele roda no
    // destino — um modelo sem servidor publica comandos nesta máquina, não containers
    // em um docker que pode nem existir por aqui.
    let alvo = profile.alvo(profile.remote_path.trim());
    if servidor.is_none() && !alvo.runtime.is_empty() {
        return Err(format!(
            "{} é um executor de runtime e roda no servidor; este modelo publica nesta máquina",
            runtime::nome(&alvo.runtime)
        ));
    }
    alvo.conferir()?;
    match servidor {
        None => {
            if strategy_needs_server(&profile.strategy) {
                return Err(format!(
                    "a estratégia {} publica em um servidor, e este modelo não tem servidor. Escolha script ou artefatos.",
                    profile.strategy
                ));
            }
            let destino = PathBuf::from(
                profile
                    .remote_path
                    .trim()
                    .trim_end_matches('/')
                    .trim_end_matches('\\'),
            );
            if !destino.is_absolute() {
                return Err(format!(
                    "a pasta de destino precisa ser um caminho absoluto, e “{}” não é",
                    profile.remote_path.trim()
                ));
            }
            if profile.strategy == "script"
                && profile.prepare_commands.iter().all(|c| c.trim().is_empty())
                && profile.build_commands.iter().all(|c| c.trim().is_empty())
                && profile.restart_commands.iter().all(|c| c.trim().is_empty())
            {
                return Err(
                    "um deploy por script nesta máquina precisa de pelo menos um comando de preparação, build ou reinicialização"
                        .into(),
                );
            }
            // Copiar o projeto para dentro dele mesmo (ou apagar o projeto ao "limpar o
            // destino") é o único jeito de perder trabalho com um modelo sem servidor.
            let (origem, alvo) = origem_e_destino(raiz, &destino, git_probe::PRAZO_SONDA)?;
            if alvo == origem || origem.starts_with(&alvo) {
                return Err(format!(
                    "a pasta de destino ({}) é o projeto ou um lugar de onde ele estaria dentro — escolha uma pasta separada",
                    profile.remote_path.trim()
                ));
            }
            if alvo.starts_with(&origem) {
                return Err(format!(
                    "a pasta de destino ({}) está dentro do projeto ({}) — publique em uma pasta fora do código-fonte",
                    profile.remote_path.trim(),
                    origem.display()
                ));
            }
        }
        Some(s) => {
            if !profile.remote_path.trim().starts_with('/') {
                return Err("o diretório remoto precisa ser um caminho absoluto".into());
            }
            if s.host.trim().is_empty() && s.ssh_alias.as_deref().unwrap_or("").trim().is_empty() {
                return Err(format!("o servidor “{}” está sem host e sem alias", s.name));
            }
            // Cada estratégia tem o seu requisito mínimo; o resto cai no `_ => {}` e é
            // validado pelas etapas seguintes.
            match profile.strategy.as_str() {
                "git" if profile.git_branch.as_deref().unwrap_or("").trim().is_empty() => {
                    return Err("o deploy por Git exige a branch de origem".into());
                }
                "script" if profile.prepare_commands.iter().all(|c| c.trim().is_empty()) => {
                    return Err("o deploy por script precisa de pelo menos um comando".into());
                }
                "docker"
                    if alvo.runtime.is_empty()
                        && profile.build_commands.is_empty()
                        && profile.restart_commands.is_empty() =>
                {
                    return Err(
                        "o deploy com Docker precisa de comandos de build ou de atualização".into(),
                    );
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_remoto(ctx: &mut Ctx) -> Result<Info, Fail> {
    let servidor = ctx.server_ref()?;
    checar_definicao(&ctx.profile, &ctx.root, Some(servidor)).map_err(Fail::new)?;
    checar_pasta_e_variaveis(ctx)?;
    ctx.garantir_askpass()?;
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
    checar_pasta_e_variaveis(ctx)?;
    checar_definicao(&ctx.profile, &ctx.root, None).map_err(Fail::new)?;
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
        ctx.askpass.as_deref(),
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

/// O executor do modelo, conferido antes de qualquer roteiro dele rodar. É a mesma
/// checagem que a tela faz ao salvar e que a etapa 1 faz na definição — num grafo
/// desenhado sem a etapa 1, é aqui que um modelo incompleto para a corrida em vez de
/// mandar para o servidor um script com a imagem vazia.
/// Onde o código do executor está no destino agora. Com releases ativadas é o symlink
/// `current`, que a etapa 11 acabou de trocar — construir a imagem a partir da pasta
/// antiga publicaria código velho. Sem releases, e nesta máquina, é o próprio
/// destino.
fn pasta_do_executor(base: &str, releases_enabled: bool, local: bool) -> String {
    if releases_enabled && !local {
        remote_join(base, "current")
    } else {
        base.to_string()
    }
}

fn executor_pronto(ctx: &Ctx) -> Result<runtime::Alvo, Fail> {
    let alvo = ctx.executor();
    alvo.conferir().map_err(Fail::new)?;
    Ok(alvo)
}

/// Manda um roteiro do executor ao destino e devolve o veredito lido do estado,
/// junto com a saída bruta para quem precisa de um marcador além do estado (a
/// imagem anterior, por exemplo). O código de saída do comando não decide nada:
/// `docker run` devolve zero para um container que morre dois segundos depois, e
/// aqui o que vale é a linha que o `docker inspect` do próprio roteiro imprimiu.
fn executar_rota(
    ctx: &mut Ctx,
    posicao: i64,
    roteiro: &str,
    rotulo: &str,
) -> Result<(String, String), Fail> {
    let script = ctx.script(roteiro);
    let saida = ctx.remote(posicao, &script, rotulo)?;
    let bruto = saida.stdout.clone();
    let alvo = executor_pronto(ctx)?;
    let estado =
        runtime::estado_no_ar(&alvo, &bruto).map_err(|motivo| Fail::failed(rotulo, Some(1), motivo))?;
    Ok((estado, bruto))
}

/// O que falta no servidor, dado o que ele respondeu à sonda. Pura porque a decisão
/// é uma só: parar antes de tocar no destino, e nomear exatamente o que a corrida
/// pediu.
fn faltas_do_servidor<'a>(
    exigidas: &[(&'a str, &'a str)],
    tem: impl Fn(&str) -> bool,
) -> Vec<&'a str> {
    exigidas
        .iter()
        .filter(|(tool, _)| !tem(tool))
        .map(|(_, rotulo)| *rotulo)
        .collect()
}

/// A recusa que vai para o histórico. Sem executor declarado a falta é da estratégia,
/// e a saída é instalar a ferramenta ou escolher outra; com executor, o nome dele é
/// dito — quem lê não precisa adivinhar por que `pm2` apareceu na lista.
fn recusa_de_ferramentas(faltam: &[&str], runtime: &str) -> String {
    let motivo = if runtime.is_empty() {
        " — instale a ferramenta ou escolha outra estratégia".to_string()
    } else {
        format!(" — o executor {} roda com elas", runtime::nome(runtime))
    };
    format!("o servidor não tem: {}{motivo}", faltam.join(", "))
}

fn prepare_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    if ctx.local_mode() {
        return prepare_local(ctx);
    }
    let strategy = ctx.profile.strategy.as_str();
    // O executor de runtime do modelo é lido antes da sonda: as ferramentas que ele
    // usam entram na checagem de requisitos, e um servidor sem `pm2` é dito aqui,
    // não descoberto no meio de uma etapa que já teria mexido no destino.
    let executor = executor_pronto(ctx)?;
    let do_executor = runtime::ferramentas(&executor.runtime);
    let precisa_git = strategy == "git";
    let precisa_docker = strategy == "docker" || do_executor.contains(&"docker");
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
    let probe = sshexec::probe_script(
        precisa_git,
        precisa_docker,
        precisa_compose,
        false,
        precisa_cksum,
        do_executor,
    );
    let sonda = ctx.remote_with(8, "sh", &probe, "verificação de requisitos")?;
    ctx.tools = sshexec::parse_probe(&sonda.stdout).into_iter().collect();
    ctx.shell = if ctx.has("bash") { "bash" } else { "sh" }.to_string();

    // O que a corrida exige, com o rótulo que a recusa mostra: as mesmas ferramentas
    // que a sonda pediu são as que se conferem aqui, sem contagem por fora.
    let mut exigidas: Vec<(&str, &str)> = Vec::new();
    if precisa_git {
        exigidas.push(("git", "git"));
    }
    if precisa_docker {
        exigidas.push(("docker", "docker"));
    }
    if precisa_compose {
        exigidas.push(("docker-compose", "docker-compose"));
    }
    if ctx.profile.backup_enabled {
        exigidas.push(("tar", "tar (necessário para o backup)"));
    }
    if precisa_cksum {
        exigidas.push(("cksum", "cksum (comparação de conteúdo)"));
    }
    // As ferramentas do executor entram sem repetir o que a estratégia já pediu:
    // um modelo `docker` com executor docker diz "docker" uma vez na recusa.
    for ferramenta in do_executor {
        if !exigidas.iter().any(|(t, _)| *t == *ferramenta) {
            exigidas.push((ferramenta, ferramenta));
        }
    }
    let faltam = faltas_do_servidor(&exigidas, |t| ctx.has(t));
    if !faltam.is_empty() {
        return Err(Fail::new(recusa_de_ferramentas(&faltam, &executor.runtime)));
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
    // Com executor declarado, a imagem desta corrida é construída a partir do código
    // que a etapa 10 acabou de enviar — por isso vem depois do envio e da troca de
    // release, não antes. O id lido do depósito de imagens é a prova do que existe;
    // "o comando terminou bem" não provaria nada.
    let executor = executor_pronto(ctx)?;
    let mut imagem_construida = None;
    if let Some(roteiro) = executor.roteiro_build() {
        let script = ctx.script(&roteiro);
        let saida = ctx.remote(11, &script, "construir a imagem desta corrida")?;
        operacoes += 1;
        if executor.runtime == "docker" {
            imagem_construida = Some(runtime::parse_imagem(&saida.stdout).ok_or_else(|| {
                Fail::failed(
                    "construir a imagem desta corrida",
                    Some(1),
                    "o build não devolveu o id da imagem — nada novo foi construído no destino",
                )
            })?);
        }
    }
    if operacoes == 0 {
        return Ok(Info::skip(
            "nenhum comando de atualização remoto e nenhuma troca de release configurada",
        ));
    }
    let mut msg = format!("{operacoes} operação(ões) de atualização concluída(s)");
    if let Some(imagem) = imagem_construida {
        msg.push_str(&format!(" · imagem {imagem}"));
    }
    Ok(Info::done(msg))
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
    let executor = executor_pronto(ctx)?;
    // Os comandos de reinicialização do modelo continuam sendo o primeiro passo,
    // como sempre foram. O executor entra depois deles, e é ele que põe o runtime no
    // ar quando o modelo declara um — sem executor declarado, nada aqui muda.
    let comandos = if ctx.profile.restart_commands.is_empty() {
        None
    } else {
        Some(ctx.run_commands(12, "reinicialização", &ctx.profile.restart_commands.clone())?)
    };
    if ctx.reversao {
        // Uma reversão não traz código novo: quem trocou o estado foi a etapa de
        // atualização, e a próxima relê. Subir aqui a imagem desta corrida desfaria a
        // reversão que acabou de acontecer.
        return match comandos {
            Some(n) => Ok(Info::done(format!("{n} comando(s) de reinicialização aplicados"))),
            None => Ok(Info::skip(
                "reversão: nada é subido aqui — o runtime voltou na etapa de atualização e o estado é lido na próxima",
            )),
        };
    }
    if let Some(roteiro) = executor.roteiro_subir() {
        let (estado, bruto) = executar_rota(ctx, 12, &roteiro, "subir o runtime do destino")?;
        // A imagem que estava no ar antes desta corrida é lida do que o próprio
        // roteiro imprimiu, antes de o container antigo sair: é o único destino
        // possível de uma reversão de container, e vai no registro da execução.
        ctx.imagem_anterior = runtime::parse_anterior(&bruto).map(|(_, imagem)| imagem);
        let inicio = match comandos {
            Some(n) => format!("{n} comando(s) de reinicialização · "),
            None => String::new(),
        };
        return Ok(Info::done(format!("{inicio}{estado}")));
    }
    match comandos {
        Some(n) => Ok(Info::done(format!("{n} comando(s) de reinicialização aplicados"))),
        None => Ok(Info::skip(
            "sem comandos de reinicialização e sem executor de runtime — esta configuração não reinicia serviço",
        )),
    }
}

fn health_step(ctx: &mut Ctx) -> Result<Info, Fail> {
    let executor = executor_pronto(ctx)?;
    let feitos = if ctx.profile.health_checks.is_empty() {
        0
    } else {
        ctx.run_commands(13, "saúde", &ctx.profile.health_checks.clone())?
    };
    // Com executor, a saúde é o estado relido da ferramenta — a mesma leitura que
    // decidiu a subida, refeita agora, sem mudar nada no destino.
    if let Some(roteiro) = executor.roteiro_saude() {
        let (estado, _) = executar_rota(ctx, 13, &roteiro, "ler o estado do runtime")?;
        return Ok(Info::done(if feitos == 0 {
            format!("saúde lida do destino: {estado}")
        } else {
            format!("{feitos} teste(s) de saúde passaram · {estado}")
        }));
    }
    if feitos == 0 {
        return Ok(Info::skip(
            "nenhum teste de saúde configurado — confirme o serviço manualmente",
        ));
    }
    Ok(Info::done(format!("{feitos} teste(s) de saúde passaram")))
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
    let mut feitos: Vec<String> = Vec::new();
    // Reversão num destino com Docker: o que serve o conteúdo é o container, e o id
    // que estava no ar antes desta corrida só existe no histórico lido da máquina —
    // nenhum comando de rollback do modelo sabe dele.
    let executor = executor_pronto(ctx)?;
    if executor.runtime == "docker" {
        let imagem = ctx.imagem_anterior.clone().ok_or_else(|| {
            Fail::new(
                "a execução que se quer reverter não registrou a imagem que estava no ar — sem esse id não há para onde voltar o container",
            )
        })?;
        let roteiro = executor.roteiro_reverter(&imagem);
        let (estado, bruto) =
            executar_rota(ctx, 11, &roteiro, "voltar o container para a imagem anterior")?;
        // O container que sai também é lido: esta corrida deslocou uma imagem, e uma
        // reversão dela tem para onde voltar do mesmo jeito.
        ctx.imagem_anterior = runtime::parse_anterior(&bruto).map(|(_, id)| id);
        feitos.push(format!(
            "container {} voltou para a imagem {imagem} · {estado}",
            executor.container()
        ));
    }
    if !ctx.profile.rollback_commands.is_empty() {
        let n = ctx.run_commands(11, "rollback", &ctx.profile.rollback_commands.clone())?;
        feitos.push(format!("{n} comando(s) de rollback executados"));
        return Ok(Info::done(feitos.join(" · ")));
    }
    let anterior = match ctx.previous.clone() {
        Some(anterior) => anterior,
        None => {
            if feitos.is_empty() {
                return Err(Fail::new(
                    "não há publicação anterior registrada nem comandos de rollback configurados — reversão automática não é possível",
                ));
            }
            return Ok(Info::done(feitos.join(" · ")));
        }
    };
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
    feitos.push(format!("diretório ativo voltou para {alvo}"));
    Ok(Info::done(feitos.join(" · ")))
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
/// O que fica gravado na execução sobre a sua origem. É lido no instante em que a
/// execução abre: a definição pode mudar depois e o histórico não muda junto. O
/// commit vem do git da pasta do projeto — `None` quando a pasta não é repositório
/// ou não tem commit, e a tela mostra isso como "sem commit", não como inventado.
fn procedencia(profile: &DeployProfile, servidor: &Option<Server>, root: &Path) -> db::Procedencia {
    db::Procedencia {
        definition_version: profile.version,
        destination: profile.remote_path.clone(),
        environment: servidor
            .as_ref()
            .map(|s| s.environment.clone())
            .unwrap_or_else(|| "local".to_string()),
        commit_sha: git::head_sha(root),
    }
}

/// O pedido de uma corrida, do jeito que a janela o faz: de qual definição, o que
/// percorrer — a definição inteira ou um nó de diagnóstico — e com que autorização.
pub struct PedidoDeCorrida {
    pub profile_id: i64,
    pub alvo: Option<String>,
    pub com_dependencias: bool,
    pub secret_values: HashMap<String, String>,
    pub allow_secrets: bool,
}

pub fn start(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    profile_id: i64,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
) -> Result<i64, String> {
    corrida(
        app,
        state,
        db,
        broker,
        PedidoDeCorrida {
            profile_id,
            alvo: None,
            com_dependencias: false,
            secret_values,
            allow_secrets,
        },
    )
}

/// Execução de diagnóstico de um nó: ele sozinho, ou ele e a corrente de
/// dependências que produz o estado que ele lê. Serve para conferir um ramo do grafo
/// sem publicar nada além dele — e por isso o nó que toca o destino exige a mesma
/// autorização explícita de segredos de uma corrida inteira.
pub fn run_no(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    mut pedido: PedidoDeCorrida,
) -> Result<i64, String> {
    let alvo = pedido.alvo.take().unwrap_or_default();
    let alvo = alvo.trim();
    if alvo.is_empty() {
        return Err("diga qual nó você quer executar".to_string());
    }
    pedido.alvo = Some(alvo.to_string());
    corrida(app, state, db, broker, pedido)
}

/// Toda corrida que publica abre por aqui. O que roda é o que a tela mostra: uma
/// definição com grafo salvo executa o grafo, e sem nós ela permanece o pipeline
/// linear de quinze etapas — a etapa 3 acrescenta nós, não um segundo caminho.
fn corrida(
    app: AppHandle,
    state: &DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    pedido: PedidoDeCorrida,
) -> Result<i64, String> {
    let alvo = pedido.alvo.clone();
    let com_dependencias = pedido.com_dependencias;
    let profile_id = pedido.profile_id;
    let (profile, servidor, project, root, extra, run_id, nos) = {
        let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
        let profile = db::get_deploy_profile(&conn, profile_id)?;
        let servidor = servidor_do_modelo(&conn, &profile)?;
        let project = db::get_project(&conn, profile.project_id)?;
        let nos = db::list_pipeline_nodes(&conn, profile_id)?;
        if alvo.is_some() && nos.is_empty() {
            return Err(format!(
                "a definição “{}” não tem grafo salvo — não há nó para executar sozinho",
                profile.name
            ));
        }
        let extra = security::extra_patterns(
            db::setting_value(&conn, SECRET_PATTERNS_SETTING)?.as_deref(),
        );
        let root = PathBuf::from(&project.local_path);
        let run_id = db::insert_run(&conn, profile_id, &procedencia(&profile, &servidor, &root))?;
        let _ = audit::log(
            &conn,
            "info",
            "deploy",
            match &alvo {
                Some(_) => "Diagnóstico de nó iniciado",
                None => "Deploy iniciado",
            },
            servidor.as_ref().map(|s| s.name.as_str()),
            Some(&project.name),
            None,
            Some(&format!(
                "modelo {} · estratégia {} · execução #{run_id}{}{}",
                profile.name,
                profile.strategy,
                if servidor.is_none() { " · nesta máquina" } else { "" },
                match &alvo {
                    Some(chave) if com_dependencias =>
                        format!(" · nó “{chave}” com dependências"),
                    Some(chave) => format!(" · nó “{chave}”"),
                    None if nos.is_empty() => String::new(),
                    None => format!(" · grafo com {} nó(s)", nos.len()),
                }
            )),
        );
        (profile, servidor, project, root, extra, run_id, nos)
    };
    let percurso = if nos.is_empty() {
        Percurso::Linear
    } else {
        Percurso::Grafo {
            nos,
            alvo,
            com_dependencias,
        }
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
        percurso,
        pedido.secret_values,
        pedido.allow_secrets,
        // Uma corrida nova não herda nada do histórico: não há destino de reversão
        // antes dela.
        Heranca::default(),
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
    let (profile, servidor, project, root, extra, heranca, new_run) = {
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
        let novo = db::insert_run(&conn, profile.id, &procedencia(&profile, &servidor, &root))?;
        let heranca = Heranca {
            previous: origem.previous_release.clone(),
            imagem: origem.previous_image.clone(),
        };
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
                heranca
                    .previous
                    .clone()
                    .or_else(|| heranca.imagem.as_ref().map(|i| format!("imagem {i}")))
                    .unwrap_or_else(|| "comandos do modelo".to_string())
            )),
        );
        (profile, servidor, project, root, extra, heranca, novo)
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
        Percurso::Reversao,
        secret_values,
        true,
        heranca,
    )
}

/// O que uma corrida herda do histórico que a pediu. Só a reversão herda alguma
/// coisa: é o registro da execução desfeita que diz para onde voltar — o symlink da
/// release anterior e, num destino com Docker, a imagem que estava no ar.
#[derive(Debug, Default)]
struct Heranca {
    previous: Option<String>,
    imagem: Option<String>,
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
    percurso: Percurso,
    secret_values: HashMap<String, String>,
    allow_secrets: bool,
    heranca: Heranca,
) -> Result<i64, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let operacao = match &percurso {
        Percurso::Reversao => "Rollback",
        Percurso::Grafo { alvo: Some(_), .. } => "Diagnóstico",
        _ => "Deploy",
    };
    let destino = servidor
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| profile.remote_path.clone());
    let label = format!("{operacao} {} → {destino} (#{run_id})", profile.name);
    state.register(run_id, label.clone(), Arc::clone(&cancel))?;

    // Os valores fornecidos já entram na máscara desde aqui. Na fila de quinze etapas
    // a etapa 1 refaz `Secrets` com os mesmos dados, mas um nó rodado sozinho nunca
    // passa por aquela etapa: sem isto o valor da variável chegaria cru ao log
    // gravado, e é justamente o log gravado que sobrevive à janela.
    let secrets = Secrets::new(&profile.secret_keys, &secret_values);
    let ctx = Ctx {
        app: app.clone(),
        db: db.clone(),
        run_id,
        cancel: Arc::clone(&cancel),
        touched: AtomicU16::new(0),
        provided: secret_values,
        secrets,
        askpass: None,
        shell: "sh".to_string(),
        files: Vec::new(),
        plano: None,
        release: None,
        previous: heranca.previous,
        imagem_anterior: heranca.imagem,
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
        approvals: state.approvals.clone(),
        node_row: None,
        node_timeout: None,
        reversao: matches!(&percurso, Percurso::Reversao),
        attempt: 1,
    };
    let linhas = match &percurso {
        // todas as etapas aparecem como "aguardando" assim que o clique acontece
        Percurso::Linear | Percurso::Reversao => ctx.mark_waiting_do_motor(),
        Percurso::Grafo { nos, alvo, com_dependencias } => {
            ctx.mark_waiting_dos_nos(nos, alvo.as_deref(), *com_dependencias)
        }
    };
    if linhas == 0 {
        state.forget(run_id);
        if let Ok(conn) = db.lock() {
            let _ = db::finish_run(
                &conn,
                run_id,
                "failed",
                None,
                None,
                None,
                Some("o grafo da definição não tem nenhum nó para esta execução"),
            );
        }
        return Err("o grafo da definição não tem nenhum nó para esta execução".to_string());
    }
    let thread_state = state.clone();
    let spawned = thread::Builder::new()
        .name(format!("deploy-{run_id}"))
        .spawn(move || execute(ctx, thread_state, percurso));
    if let Err(e) = spawned {
        state.forget(run_id);
        if let Ok(conn) = db.lock() {
            let _ = db::finish_run(
                &conn,
                run_id,
                "failed",
                None,
                None,
                None,
                Some(&format!("falha ao iniciar a thread do pipeline: {e}")),
            );
        }
        return Err(format!("falha ao iniciar o pipeline: {e}"));
    }
    Ok(run_id)
}

/// O que uma corrida percorre. `Linear` é o pipeline de quinze etapas do capítulo
/// 10, e é por onde toda definição sem grafo salvo continua rodando — a etapa 3
/// não reescreve o caminho existente, ela acrescenta nós que chamam as mesmas
/// etapas. `Reversao` é o retorno assistido, que permanece linear por definição:
/// reverter é sempre a mesma sequência curta conferida no destino.
#[derive(Debug, Clone)]
pub enum Percurso {
    Linear,
    Reversao,
    Grafo {
        nos: Vec<PipelineNode>,
        /// `Some` é o modo de diagnóstico: um nó, e as dependências dele quando
        /// pedidas.
        alvo: Option<String>,
        com_dependencias: bool,
    },
}

fn execute(ctx: Ctx, state: DeployState, percurso: Percurso) {
    let mut ctx = ctx;
    let grafico = matches!(&percurso, Percurso::Grafo { .. });
    let reversao = matches!(&percurso, Percurso::Reversao);
    let resultado = match &percurso {
        Percurso::Reversao => rollback_pipeline(&mut ctx),
        Percurso::Linear => pipeline(&mut ctx),
        Percurso::Grafo { nos, alvo, com_dependencias } => {
            grafo(&mut ctx, nos, alvo.as_deref(), *com_dependencias)
        }
    };
    if !grafico {
        let flags = ctx.touched.load(Ordering::Relaxed);
        for position in 1..=15 {
            if flags & (1u16 << position) == 0 {
                ctx.mark(position, "skipped", Some("etapa não alcançada"), None, None);
            }
        }
    }
    let (status, detail) = match &resultado {
        Ok(resumo) => ("success", resumo.clone()),
        Err(fail) => (
            if fail.cancelled { "cancelled" } else { "failed" },
            fail.message.clone(),
        ),
    };
    finalize(&ctx, status, &detail, reversao);
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
            ctx.imagem_anterior.as_deref(),
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

    /// Um nó do grafo para as checagens do escalonador, que só olham chave, tipo,
    /// etapa e dependências.
    fn no_de_grafo(chave: &str, tipo: &str, etapa: Option<i64>, dep: &[&str]) -> PipelineNode {
        PipelineNode {
            id: 0,
            profile_id: 1,
            node_key: chave.to_string(),
            title: chave.to_string(),
            kind: tipo.to_string(),
            engine_step: etapa,
            executor: "local".to_string(),
            environment: None,
            working_dir: None,
            commands: Vec::new(),
            git_op: None,
            git_ref: None,
            analyzer: String::new(),
            severity_gate: String::new(),
            depends_on: dep.iter().map(|d| d.to_string()).collect(),
            parallel: false,
            timeout_secs: 0,
            retry_limit: 0,
            condition: None,
            requires_approval: false,
            touches_destination: false,
            x: 0,
            y: 0,
        }
    }

    /// O mesmo desenho que a tela produz: dois ramais depois da coleta, um portão
    /// de aprovação antes da cópia, e o registro no fim.
    fn grafo_de_teste() -> Vec<PipelineNode> {
        vec![
            no_de_grafo("validar", "motor", Some(1), &[]),
            no_de_grafo("git", "motor", Some(3), &["validar"]),
            no_de_grafo("coleta", "motor", Some(4), &["git"]),
            no_de_grafo("preparo", "comando", None, &["coleta"]),
            no_de_grafo("publicar", "motor", Some(8), &["coleta"]),
            no_de_grafo("gate", "aprovacao", None, &["preparo", "publicar"]),
            no_de_grafo("copia", "motor", Some(10), &["gate"]),
            no_de_grafo("historico", "motor", Some(15), &["copia"]),
        ]
    }

    #[test]
    fn o_escopo_do_diagnostico_e_o_no_sozinho_ou_a_corrente_dele() {
        let nos = grafo_de_teste();
        assert_eq!(escopo_do_grafo(&nos, None, false), (0..8).collect::<Vec<usize>>());
        assert_eq!(escopo_do_grafo(&nos, Some("copia"), false), vec![6]);
        assert_eq!(escopo_do_grafo(&nos, Some("copia"), true), vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(escopo_do_grafo(&nos, Some("que_nao_existe"), true).is_empty());
    }

    #[test]
    fn dependencia_fora_do_escopo_nao_trava_e_dentro_trava_so_com_falha() {
        let nos = grafo_de_teste();
        let indice: HashMap<&str, usize> = nos
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node_key.as_str(), i))
            .collect();
        let copia = &nos[6];
        let mut situacao = HashMap::new();
        situacao.insert(5usize, Sit::Falho);

        let so_a_copia: HashSet<usize> = [6usize].into_iter().collect();
        assert!(pronto(copia, &situacao, &indice, &so_a_copia));
        assert!(!bloqueado(copia, &situacao, &indice, &so_a_copia));

        let todo: HashSet<usize> = (0..8).collect();
        assert!(!pronto(copia, &situacao, &indice, &todo));
        assert!(bloqueado(copia, &situacao, &indice, &todo));

        // Pulado por condição não é falha: quem depende dele roda mesmo assim.
        situacao.insert(5usize, Sit::Pulado);
        assert!(pronto(copia, &situacao, &indice, &todo));
        assert!(!bloqueado(copia, &situacao, &indice, &todo));
    }

    #[test]
    fn falha_trava_a_corrente_inteira_e_a_condicao_nao_propaga_nada() {
        let nos = grafo_de_teste();
        let indice: HashMap<&str, usize> = nos
            .iter()
            .enumerate()
            .map(|(i, n)| (n.node_key.as_str(), i))
            .collect();
        let todo: HashSet<usize> = (0..8).collect();
        let gate = &nos[5];
        let mut situacao = HashMap::new();

        // `coleta` (2) falhou: `preparo` (3) está travado, e o portão acima dele
        // também — falha não para no filho direto.
        situacao.insert(2usize, Sit::Falho);
        situacao.insert(3usize, Sit::Bloqueado);
        assert!(bloqueado(gate, &situacao, &indice, &todo));
        assert!(!pronto(gate, &situacao, &indice, &todo));

        // O mesmo nó de cima, pulado porque a condição não se cumpriu: nada abaixo
        // dele está travado, e o portão roda depois que o outro ramal terminar.
        situacao.insert(3usize, Sit::Pulado);
        situacao.insert(4usize, Sit::Feito);
        assert!(!bloqueado(gate, &situacao, &indice, &todo));
        assert!(pronto(gate, &situacao, &indice, &todo));
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

    #[test]
    fn o_executor_peg_a_pasta_que_a_corrida_acabou_de_publicar() {
        // Com releases, o `current` é o código desta corrida. A raiz seria a
        // publicação anterior: construir a imagem a partir dela publicaria código
        // velho sem dizer nada.
        assert_eq!(pasta_do_executor("/srv/api", true, false), "/srv/api/current");
        // Sem releases não há symlink nenhum, e nesta máquina o destino é o destino.
        assert_eq!(pasta_do_executor("/srv/api", false, false), "/srv/api");
        assert_eq!(pasta_do_executor("C:/publicados/site", true, true), "C:/publicados/site");
    }

    #[test]
    fn a_falta_no_servidor_e_a_ferramenta_que_a_sonda_pediu() {
        let exigidas = [("git", "git"), ("pm2", "pm2"), ("node", "node (o app do pm2)")];
        let tem = |t: &str| t == "git" || t == "node";
        assert_eq!(faltas_do_servidor(&exigidas, tem), vec!["pm2"]);
        assert!(faltas_do_servidor(&exigidas, |_| true).is_empty());
    }

    #[test]
    fn a_recusa_nomeia_o_executor_e_sem_executor_diz_a_estrategia() {
        let com_executor = recusa_de_ferramentas(&["pm2", "node (o app do pm2)"], "pm2");
        assert!(com_executor.contains("o servidor não tem: pm2, node"), "{com_executor}");
        assert!(com_executor.contains("executor PM2"), "{com_executor}");
        // nada declarado é exatamente a mensagem de antes: a falta é da estratégia
        let sem_executor = recusa_de_ferramentas(&["git"], "");
        assert!(sem_executor.contains("escolha outra estratégia"), "{sem_executor}");
        assert!(!sem_executor.contains("executor"), "{sem_executor}");
    }

    /// Um repositório de verdade, criado pelo git desta máquina: o nó `git` não é
    /// conferido contra um git simulado, porque o que a linha conta é a resposta dele.
    fn repo_de_teste(nome: &str) -> PathBuf {
        let dir = pasta_de_teste(nome);
        git_em(&dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]);
        git_em(&dir, &["config", "user.name", "Teste Local"]);
        git_em(&dir, &["config", "user.email", "teste@local"]);
        git_em(&dir, &["config", "commit.gpgsign", "false"]);
        git_em(&dir, &["config", "core.autocrlf", "false"]);
        git_em(&dir, &["commit", "-q", "-m", "primeiro", "--allow-empty"]);
        dir
    }

    fn git_em(dir: &Path, args: &[&str]) -> String {
        let feito = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("o git desta máquina");
        assert!(
            feito.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&feito.stderr)
        );
        String::from_utf8_lossy(&feito.stdout).to_string()
    }

    /// Um "servidor" para os testes de publicação: repositório bare, como um remoto
    /// real (um repositório comum recusaria o push no ramo checkoutado).
    fn remoto_bare(de: &Path, nome: &str) -> PathBuf {
        let destino = de.parent().unwrap().join(format!("{nome}.git"));
        let _ = fs::remove_dir_all(&destino);
        git_em(de, &["clone", "--bare", "-q", ".", &destino.to_string_lossy()]);
        destino
    }

    /// O caminho no formato que o git aceita como URL local.
    fn url_local(dir: &Path) -> String {
        dir.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn o_no_git_marca_a_tag_e_diz_onde_ela_ficou() {
        let dir = repo_de_teste("no-git-tag");
        let feito = git_do_no(&dir, "tag", "v1.0.0").unwrap();
        assert!(feito.contains("tag v1.0.0 criada"), "{feito}");
        assert!(feito.contains("sem remoto"), "{feito}");
        // lida de volta no próprio repositório: a linha não promete o que o git não fez
        let nomes: Vec<String> = git::tags(&dir).unwrap().into_iter().map(|t| t.name).collect();
        assert!(nomes.contains(&"v1.0.0".to_string()), "{nomes:?}");
    }

    #[test]
    fn o_no_git_publica_a_tag_onde_ha_um_remoto() {
        let origem = repo_de_teste("no-git-tag-push");
        let bare = remoto_bare(&origem, "no-git-tag-push");
        let endereco = url_local(&bare);
        git_em(&origem, &["remote", "add", "origin", &endereco]);
        git_em(&origem, &["push", "-q", "origin", "main"]);

        let feito = git_do_no(&origem, "tag", "v2.0.0").unwrap();
        assert!(feito.contains("publicada em origin"), "{feito}");
        // o remoto é lido de volta: a tag não está só na fala do nó
        let lidas = git_em(&bare, &["tag", "-l"]);
        assert!(lidas.contains("v2.0.0"), "{lidas}");
    }

    #[test]
    fn o_no_git_altern_e_a_linha_vem_da_cabeca_lida_depois() {
        let dir = repo_de_teste("no-git-checkout");
        git_em(&dir, &["branch", "alt"]);
        assert_eq!(git_do_no(&dir, "checkout", "alt").unwrap(), "em alt");
        // um ramo que não existe falha com a palavra do git, não com a nossa
        let erro = git_do_no(&dir, "checkout", "nao-existe").unwrap_err();
        assert!(erro.contains("nao-existe"), "{erro}");
    }

    #[test]
    fn o_no_git_puxa_sem_inventar_merge() {
        let origem = repo_de_teste("no-git-pull");
        let bare = remoto_bare(&origem, "no-git-pull");
        let endereco = url_local(&bare);
        git_em(&origem, &["remote", "add", "origin", &endereco]);
        git_em(&origem, &["push", "-q", "-u", "origin", "main"]);
        let clone = origem.parent().unwrap().join("no-git-pull-clone");
        let _ = fs::remove_dir_all(&clone);
        git_em(origem.parent().unwrap(), &["clone", "-q", &endereco, &clone.to_string_lossy()]);

        git_em(&origem, &["commit", "-q", "-m", "segundo", "--allow-empty"]);
        git_em(&origem, &["push", "-q", "origin", "main"]);

        let feito = git_do_no(&clone, "pull", "").unwrap();
        assert!(feito.starts_with("Updating "), "{feito}");
        assert_eq!(
            git_em(&clone, &["rev-parse", "HEAD"]).trim(),
            git_em(&origem, &["rev-parse", "HEAD"]).trim(),
            "o nó falou em avançar e a cabeça do clone não mudou"
        );
        // e o segundo pull não cria commit nenhum: é a linha do git que fica no histórico
        let repetido = git_do_no(&clone, "pull", "").unwrap();
        assert!(repetido.contains("Already up to date"), "{repetido}");
        let _ = fs::remove_dir_all(&clone);
    }

    #[test]
    fn uma_operacao_que_o_motor_nao_conhece_para_o_no() {
        let dir = repo_de_teste("no-git-desconhecida");
        let erro = git_do_no(&dir, "reset", "HEAD~1").unwrap_err();
        assert!(erro.contains("pull, checkout, tag"), "{erro}");
        assert!(erro.contains("reset"), "{erro}");
    }

    /// Uma corrida de análise como ela sairia do banco, para conferir só o que o
    /// nó decide com ela na mão.
    fn corrida(blocked: bool, status: &str, corte: &str, nota: Option<&str>) -> AnalysisRun {
        AnalysisRun {
            id: 7,
            project_id: 1,
            project: "casa".to_string(),
            root: "C:/projetos/casa".to_string(),
            analyzer: "embutido".to_string(),
            tool: "embutido".to_string(),
            tool_version: None,
            rules: "dd-101-shell-true".to_string(),
            severity_gate: corte.to_string(),
            status: status.to_string(),
            started_at: "agora".to_string(),
            finished_at: Some("agora".to_string()),
            duration_ms: 12,
            files_scanned: 40,
            files_skipped: 0,
            exit_code: None,
            tool_errors: 0,
            blocked,
            counts: crate::models::AnalysisCounts {
                error: if blocked { 2 } else { 0 },
                warning: 1,
                info: 0,
            },
            note: nota.map(str::to_string),
        }
    }

    #[test]
    fn o_no_de_analise_barra_com_a_contagem_e_nao_com_o_codigo_de_saida() {
        let atingido = veredito_do_no(&corrida(true, "concluida", "ERROR", None)).unwrap_err();
        assert_eq!(atingido.command.as_deref(), Some("análise de código (corte ERROR)"));
        assert!(atingido.message.contains("erro 2, aviso 1"), "{}", atingido.message);
        assert!(atingido.message.contains("barra a publicação"), "{}", atingido.message);
        assert_eq!(atingido.exit_code, None, "o nó não tem código de saída de ferramenta");

        let dentro = veredito_do_no(&corrida(false, "concluida", "ERROR", None)).unwrap();
        assert!(dentro.contains("dentro do corte ERROR"), "{dentro}");
        assert!(dentro.contains("1 achado(s) com 40 arquivo(s) lidos"), "{dentro}");
    }

    #[test]
    fn o_no_de_analise_nao_passa_por_limpo_sem_leitura() {
        // Sem corte o nó informa, mas uma varredura que falhou não informa nada:
        // é falha dita com a nota de quem varreu, nunca um sucesso presumido.
        let sem_corte = veredito_do_no(&corrida(false, "falhou", "", Some("a varredura não leu arquivo nenhum")))
            .unwrap_err();
        assert!(
            sem_corte.message.contains("a varredura não leu arquivo nenhum"),
            "{}",
            sem_corte.message
        );
        // e um nó que só informa, com a leitura feita, conclui dizendo que nada barra
        let informativo = veredito_do_no(&corrida(false, "concluida", "", None)).unwrap();
        assert!(informativo.contains("nada barra"), "{informativo}");
    }

    #[test]
    fn uma_varredura_interrompida_e_cancelamento_do_no() {
        // Interrompida com corte atingido: os números estão lá, mas o estado diz que
        // a varredura não terminou. É cancelamento, não falha de contagem — quem vê
        // o histórico tem de ler "interrompida", não "o código tem 2 erros".
        let cortada = veredito_do_no(&corrida(true, "interrompida", "ERROR", None)).unwrap_err();
        assert!(cortada.cancelled, "{:?}", cortada.message);
        assert_eq!(cortada.message, "interrompido pelo usuário");
        assert_eq!(cortada.command, None, "não houve comando de ferramenta cortado");

        // sem corte também: metade da árvore lida não conclui o nó
        let muda = veredito_do_no(&corrida(false, "interrompida", "", None)).unwrap_err();
        assert!(muda.cancelled);

        // um estado que não é nenhum dos terminais — a corrida que ficou "rodando"
        // porque o aplicativo morreu no meio — é falha dita, nunca sucesso presumido
        let parada = veredito_do_no(&corrida(false, "rodando", "ERROR", None)).unwrap_err();
        assert!(!parada.cancelled, "estado desconhecido não é vontade do usuário");
        assert!(
            parada.message.contains("que não é uma conclusão"),
            "{}",
            parada.message
        );
    }

    #[test]
    fn o_teto_de_linhas_declara_o_corte_uma_só_vez() {
        // Saída maior que o teto: o bloco fecha no teto e a última linha declara o
        // corte — nunca um log que mente sobre o tamanho do que houve.
        let saida: String = (0..2_500)
            .map(|i| format!("linha {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let bloco = bloco_de_log("out", &saida, 0);
        assert_eq!(
            bloco.len(),
            TETO_LINHAS_POR_TENTATIVA as usize + 1,
            "as 2.000 linhas graváveis mais o aviso de corte"
        );
        assert_eq!(bloco[0], ("stdout".to_string(), "linha 0".to_string()));
        assert_eq!(
            bloco[TETO_LINHAS_POR_TENTATIVA as usize - 1].1,
            format!("linha {}", TETO_LINHAS_POR_TENTATIVA - 1)
        );
        let aviso = &bloco.last().unwrap();
        assert_eq!(aviso.0, "system");
        assert!(aviso.1.contains("truncado em 2000"), "{}", aviso.1);
        assert!(aviso.1.contains("500 linha(s) além do teto"), "{}", aviso.1);

        // Bloco que já estourou o teto não escreve mais nada, nem repete o aviso.
        assert!(bloco_de_log("out", "uma linha qualquer", TETO_LINHAS_POR_TENTATIVA).is_empty());

        // O corte é do que faltou, não do que já estava lá: 1999 gravadas e 3 novas
        // deixam 1 linha e avisam das 2 que não couberam.
        let apertado = bloco_de_log("err", "a\nb\nc", TETO_LINHAS_POR_TENTATIVA - 1);
        assert_eq!(apertado.len(), 2);
        assert_eq!(apertado[0], ("stderr".to_string(), "a".to_string()));
        assert!(apertado[1].1.contains("2 linha(s) além do teto"), "{}", apertado[1].1);

        // E o nome do fluxo do esquema vem do do motor, com as em branco fora:
        // linha vazia não é saída de ninguém.
        let nomeado = bloco_de_log("err", "caiu\n\n  \naqui dentro", 0);
        assert_eq!(
            nomeado,
            vec![
                ("stderr".to_string(), "caiu".to_string()),
                ("stderr".to_string(), "aqui dentro".to_string()),
            ]
        );
        assert_eq!(
            bloco_de_log("sistema", "etapa pulada", 0),
            vec![("system".to_string(), "etapa pulada".to_string())]
        );
    }

    #[test]
    fn a_pasta_de_trabalho_de_um_no_e_lida_com_prazo_e_a_falta_e_dita() {
        let base = pasta_de_teste("dir-local");
        let raiz = base.join("projeto");
        fs::create_dir_all(raiz.join("sub")).expect("criar a pasta de trabalho");
        fs::create_dir_all(base.join("fora")).expect("criar a pasta de fora");

        let dentro = dir_local(&raiz, "sub", "coleta", git_probe::PRAZO_SONDA).expect("a pasta respondeu");
        assert!(!dentro.to_string_lossy().starts_with(r"\\?\"), "{dentro:?}");

        // Caminho que existe mas dobra para fora: é a contenção que recusa.
        let dobrando = dir_local(&raiz, "../fora", "coleta", git_probe::PRAZO_SONDA)
            .expect_err("sai da pasta cadastrada");
        assert!(dobrando.message.contains("sai do projeto"), "{}", dobrando.message);

        // Pasta que não está lá: ausência dita, com as palavras do nó.
        let falta = dir_local(&raiz, "nao-existe", "coleta", git_probe::PRAZO_SONDA)
            .expect_err("a pasta não existe");
        assert!(falta.message.contains("não existe dentro do projeto"), "{}", falta.message);
        assert!(!falta.message.contains("não respondeu"), "{}", falta.message);

        // Prazo menor que qualquer resposta de disco: a corrida devolve o motivo da
        // espera, e não um "não existe" que culparia o desenho do grafo.
        let sem_resposta = dir_local(&raiz, "sub", "coleta", Duration::ZERO).expect_err("0 ms é prazo");
        assert!(sem_resposta.message.contains("não respondeu em 0 ms"), "{}", sem_resposta.message);
        assert!(sem_resposta.message.contains("projeto"), "{}", sem_resposta.message);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_regua_do_destino_compara_os_dois_lados_na_mesma_forma() {
        let base = pasta_de_teste("regua-destino");
        let raiz = base.join("projeto");
        fs::create_dir_all(&raiz).expect("criar o projeto");

        // O caso que perde trabalho: um destino *dentro* do projeto, ainda não
        // criado — o da primeira publicação. Os dois lados saem da mesma forma,
        // então a régua de `checar_definicao` dispara.
        let dentro = raiz.join("publicado");
        let (origem, alvo) = origem_e_destino(&raiz, &dentro, git_probe::PRAZO_SONDA).expect("os dois lados");
        assert!(alvo.starts_with(&origem), "{alvo:?} deveria estar dentro de {origem:?}");
        assert!(!origem.starts_with(&alvo), "e não o contrário");

        // Destino de fora, também inexistente: a régua diz fora, e a publicação
        // segue.
        let fora = base.join("publicado-fora");
        let (origem2, alvo2) = origem_e_destino(&raiz, &fora, git_probe::PRAZO_SONDA).expect("os dois lados");
        assert!(!alvo2.starts_with(&origem2), "{alvo2:?} não está dentro de {origem2:?}");
        assert!(!alvo2.to_string_lossy().starts_with(r"\\?\"), "{alvo2:?}");

        // Volume que não acorda: a validação para com o motivo da espera.
        let sem_resposta = origem_e_destino(&raiz, &fora, Duration::ZERO).expect_err("0 ms é prazo");
        assert!(sem_resposta.contains("não respondeu em 0 ms"), "{sem_resposta}");
        let _ = fs::remove_dir_all(&base);
    }
}
