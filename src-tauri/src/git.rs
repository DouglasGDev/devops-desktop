//! Operações Git sobre o binário `git` nativo do sistema.
//!
//! Toda chamada usa argv separado (nada é concatenado em comando shell) e
//! `git -C <pasta>` para nunca depender do diretório atual do processo.
//! Nenhum comando reescreve histórico remoto por conta própria: operações de
//! push, merge, rebase e remoção de branch são expostas como funções distintas
//! e a confirmação acontece na interface.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    time::Duration,
};

use serde::Serialize;

/// Campo separador usado nos formatos `--pretty=format:`. Nunca aparece em
/// hashes, e-mails ou nomes de branch.
/// Separador de campos dos formatos `--format` do git: nunca aparece em nome
/// de arquivo, branch ou mensagem.
const SEP: char = '\u{001f}';

/// Árvore vazia canônica — base para mostrar o primeiro commit do histórico.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

// ---------- modelos ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusFile {
    pub path: String,
    /// caminho de origem, quando a entrada é renomeio/cópia
    pub old_path: Option<String>,
    pub index_code: char,
    pub worktree_code: char,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflict: bool,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub branch: Option<String>,
    pub detached_at: Option<String>,
    /// Operação que o Git tem parada no meio, lida dos arquivos de controle do
    /// `.git`. `None` é o estado comum. Não é deduzido da cabeça detached: uma
    /// cabeça avulsa pode ser checkout de um commit, não uma operação.
    pub operation: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub files: Vec<StatusFile>,
    pub conflict_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String,
    pub old_no: Option<i64>,
    pub new_no: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub old_start: i64,
    pub old_lines: i64,
    pub new_start: i64,
    pub new_lines: i64,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status_char: char,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub unix_date: i64,
    pub subject: String,
    pub refs: Vec<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    pub graph: String,
    pub commit: CommitInfo,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_head: bool,
    pub remote: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub last_subject: String,
    pub last_unix_date: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    pub name: String,
    pub target: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub name: String,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
}

/// Veredito do teste de acesso a um remoto. A URL não entra aqui de propósito:
/// ela pode carregar credencial, e o nome do remoto basta para a interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccess {
    pub ok: bool,
    pub remote: String,
    /// Uma frase em português dizendo o que foi confirmado ou o que falta.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub commit: CommitInfo,
    pub files: Vec<StatusFile>,
    pub total_additions: i64,
    pub total_deletions: i64,
}

/// O conteúdo que o Git guarda de um arquivo. `text` vem preenchido quando o
/// blob é texto; senão é a `nota` que diz por que não há texto — sem inventar
/// conteúdo nem cortar silencioso.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub text: Option<String>,
    pub note: Option<String>,
    pub truncated: bool,
}

/// Um trabalho guardado pelo Git. O endereço vem da lista do próprio git
/// (`stash@{0}` é o mais recente) e a interface usa o índice numérico.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub index: usize,
    pub selector: String,
    pub sha: String,
    /// data do commit do guardado, no formato ISO curto do git
    pub date: String,
    pub subject: String,
}

// ---------- execução ----------

fn base_cmd(dir: &Path) -> Command {
    let mut cmd = crate::proc::command("git");
    cmd.arg("-C")
        .arg(dir)
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("--no-pager")
        // Nenhum comando pode abrir prompt de credencial: isso travaria a
        // aplicação. A autenticação remota fica no helper/chave do usuário.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        // As mensagens do git são analisadas para gerar os erros traduzidos da
        // interface: sem locale fixo a tradução dependeria do idioma do sistema.
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    cmd
}

/// Prazo de um `git` local. Uma leitura travada não pode segurar a interface
/// para sempre: sem prazo, o botão fica girando e a pessoa não sabe se o git
/// pensa ou se morreu.
const PRAZO_LOCAL: Duration = Duration::from_secs(30);
/// Prazo de um `git` que fala com um remoto. Clonar um repositório grande leva
/// minutos, então aqui o prazo é longo o bastante para ser legítimo e curto o
/// bastante para nunca parecer travado.
const PRAZO_REDE: Duration = Duration::from_secs(600);

/// O comando vai trafegar pela rede?
fn pela_rede(args: &[&str]) -> bool {
    matches!(
        args.first().copied().unwrap_or(""),
        "clone" | "fetch" | "pull" | "push" | "ls-remote"
    )
}

fn prazo_dos(args: &[&str]) -> Duration {
    if pela_rede(args) { PRAZO_REDE } else { PRAZO_LOCAL }
}

/// Texto do prazo que venceu, na unidade que faz sentido para o valor.
fn prazo_expirado(args: &[&str], prazo: Duration) -> String {
    let minutos = prazo.as_secs() / 60;
    let duracao = if minutos >= 1 {
        format!("{minutos} min")
    } else {
        format!("{} s", prazo.as_secs())
    };
    let motivo = if pela_rede(args) {
        "pode ser o remoto lento ou uma credencial que o git não pode pedir (GIT_TERMINAL_PROMPT=0)"
    } else {
        "pode ser um índice travado por outro git, um hook parado ou o antivírus segurando a pasta"
    };
    format!("o git não respondeu em {duracao} e foi interrompido — {motivo}")
}

/// Roda o git com prazo. `entrada` alimenta a stdin (pathspecs e patches).
///
/// Toda execução de git do aplicativo passa por aqui: um processo que não
/// termina é interrompido e devolvido como erro legível, nunca como tela
/// girando para sempre.
fn executar(dir: &Path, shown: &[&str], prazo: Duration, entrada: Option<&[u8]>) -> Result<String, String> {
    let mut cmd = base_cmd(dir);
    if entrada.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd
        .args(shown)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "o git não foi encontrado no PATH — instale o Git para usar esta seção".to_string()
            }
            _ => format!("não foi possível executar o git: {e}"),
        })?;
    if let Some(dados) = entrada {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "falha ao abrir a entrada do git".to_string())?;
        stdin
            .write_all(dados)
            .map_err(|e| format!("falha ao enviar dados ao git: {e}"))?;
        stdin.flush().ok();
    }
    drop(child.stdin.take());
    let feito = crate::sshexec::collect(&mut child, prazo, &AtomicBool::new(false));
    if feito.timed_out {
        return Err(prazo_expirado(shown, prazo));
    }
    if feito.exit_code == Some(0) {
        return Ok(feito.stdout);
    }
    Err(describe_failure(shown, feito.stderr.as_bytes(), feito.stdout.as_bytes()))
}

/// Roda `git` na pasta e devolve stdout. Erro traz a mensagem do git traduzida.
pub fn run<S: AsRef<str>>(dir: &Path, args: &[S]) -> Result<String, String> {
    let shown: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
    let prazo = prazo_dos(&shown);
    executar(dir, &shown, prazo, None)
}

/// Roda `git` alimentando a stdin (pathspecs e patches).
fn run_with_stdin<S: AsRef<str>>(dir: &Path, args: &[S], input: &[u8]) -> Result<String, String> {
    let shown: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
    let prazo = prazo_dos(&shown);
    executar(dir, &shown, prazo, Some(input))
}

/// Mensagens de erro compreensíveis (requisito da etapa de Git).
///
/// O git não é consistente: `commit` sem nada no índice escreve o motivo no
/// stdout e sai com código 1, enquanto outros erros usam stderr. Os dois fluxos
/// são analisados e, sem tradução conhecida, o texto original volta inteiro.
fn describe_failure(args: &[&str], stderr: &[u8], stdout: &[u8]) -> String {
    let raw_err = String::from_utf8_lossy(stderr).trim().to_string();
    let raw_out = String::from_utf8_lossy(stdout).trim().to_string();
    let lower = format!("{raw_err}\n{raw_out}").to_lowercase();
    let raw = if raw_err.is_empty() { raw_out.clone() } else { raw_err };
    let command = args.first().copied().unwrap_or("");
    if lower.contains("nothing to commit")
        || lower.contains("no changes added to commit")
        || lower.contains("nothing added to commit")
    {
        return "nada para fazer: não há alterações na área de staging".into();
    }
    if lower.contains("please tell me who you are") || lower.contains("author identity unknown") {
        return "o git não sabe quem você é — configure user.name e user.email (Configurações → Git)".into();
    }
    if lower.contains("authentication required")
        || lower.contains("terminal prompts disabled")
        || lower.contains("could not read username")
        || lower.contains("could not read password")
    {
        return format!(
            "autenticação remota necessária — use um remote com chave/SSH Agent ou um gerenciador de credenciais do git. Detalhe: {raw}"
        );
    }
    if lower.contains("rejected") && (lower.contains("non-fast-forward") || lower.contains("fetch first")) {
        return "o remoto tem commits novos — faça o fetch/pull antes do push (o histórico não é reescrito automaticamente)".into();
    }
    if lower.contains("would be overwritten by merge") || lower.contains("local changes would be overwritten") {
        return "alterações locais seriam perdidas — confirme e trate-as antes (a aplicação não descarta mudanças por conta própria)".into();
    }
    if lower.contains("is now empty") {
        return format!(
            "não sobrou mudança nenhuma para aplicar: este commit já está aplicado aqui. Cancele a operação ou pule este passo. Detalhe: {raw}"
        );
    }
    if lower.contains("conflict") || lower.contains("merge conflict") {
        return format!("há conflitos para resolver antes de continuar. Detalhe: {raw}");
    }
    if lower.contains("not a git repository") {
        return "esta pasta não é um repositório Git".into();
    }
    if lower.contains("no such ref was fetched") || lower.contains("unknown revision") || lower.contains("bad revision") {
        return format!("referência inválida para {command}. Detalhe: {raw}");
    }
    if raw.is_empty() {
        format!("o git falhou ao executar {command}")
    } else {
        raw
    }
}

// ---------- status ----------

fn split_z(raw: &str) -> Vec<String> {
    raw.split('\0')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Arquivos presentes no índice do Git, com caminho relativo ao repositório.
/// É o que o verificador de segredos usa para saber se uma credencial já está
/// sendo rastreada — estar no `.gitignore` não tira o arquivo do índice.
pub fn tracked_files(dir: &Path) -> Result<Vec<String>, String> {
    let raw = run(dir, &["ls-files", "-z"])?;
    Ok(split_z(&raw))
}

/// Máximo de texto que a interface recebe de um arquivo de uma vez.
pub const LIMITE_CONTEUDO: usize = 512 * 1024;

/// O conteúdo que o Git guarda do arquivo (`HEAD:<caminho>`). Não é leitura do
/// disco: é exatamente o que está commitado, com o próprio git resolvendo o
/// caminho — por isso um arquivo modificado mostra o texto do commit, e a
/// diferença aparece no diff.
pub fn file_content(dir: &Path, path: &str) -> Result<FileContent, String> {
    let bruto = blob_at(dir, "HEAD", path)
        .ok_or_else(|| format!("o Git não tem {} no HEAD atual", path))?;
    if bruto.iter().take(8_000).any(|b| *b == 0) {
        return Ok(FileContent {
            path: path.to_string(),
            text: None,
            note: Some("arquivo binário — não há texto para mostrar".to_string()),
            truncated: false,
        });
    }
    let texto = String::from_utf8(bruto).map_err(|_| format!("{} não está em UTF-8", path))?;
    if texto.len() <= LIMITE_CONTEUDO {
        return Ok(FileContent {
            path: path.to_string(),
            text: Some(texto),
            note: None,
            truncated: false,
        });
    }
    let mut fim = LIMITE_CONTEUDO;
    while !texto.is_char_boundary(fim) {
        fim -= 1;
    }
    let mut cortado = texto[..fim].to_string();
    cortado.push_str("\n… (conteúdo cortado; abra o arquivo no editor para ler inteiro)");
    Ok(FileContent {
        path: path.to_string(),
        text: Some(cortado),
        note: None,
        truncated: true,
    })
}

/// O que vai entrar no próximo commit. Sem HEAD ainda (primeiro commit) o
/// `diff --cached` compara o índice contra a árvore vazia e funciona igual.
pub fn staged_files(dir: &Path) -> Result<Vec<String>, String> {
    let raw = run(dir, &["diff", "--cached", "--name-only", "-z"])?;
    Ok(split_z(&raw))
}

/// Tira o arquivo do índice sem apagar a cópia local (`git rm --cached`).
pub fn untrack(dir: &Path, path: &str) -> Result<(), String> {
    validate_path(path)?;
    run(dir, &["rm", "--cached", "--", path]).map(|_| ())
}

/// Conteúdo de um caminho exatamente como uma revisão o guarda
/// (`git show <rev>:<caminho>`). `None` quando a revisão não tem o arquivo —
/// é isso que diz ao verificador de segredos que uma exclusão staged não
/// publica conteúdo novo.
pub fn blob_at(dir: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    validate_id(rev).ok()?;
    validate_path(path).ok()?;
    read_blob(dir, &format!("{rev}:{path}"))
}

/// Conteúdo exatamente como está no índice (`git show :<caminho>`), ou seja, o
/// que o próximo commit vai publicar.
pub fn index_blob(dir: &Path, path: &str) -> Option<Vec<u8>> {
    validate_path(path).ok()?;
    read_blob(dir, &format!(":{path}"))
}

fn read_blob(dir: &Path, spec: &str) -> Option<Vec<u8>> {
    let output = base_cmd(dir).args(["show", spec]).output().ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

/// Caminhos que um push publicaria e o remoto ainda pode não ter: o que foi
/// adicionado ou modificado entre a branch de rastreio e o HEAD. Sem branch
/// de rastreio reconhecível, todo o conteúdo do HEAD está em jogo.
pub fn push_risk_paths(dir: &Path) -> Result<Vec<String>, String> {
    let upstream = run(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "@{u}")
        .filter(|s| validate_id(s).is_ok());
    if let Some(name) = upstream {
        let verify = format!("{name}^{{commit}}");
        if run(dir, &["rev-parse", "--verify", "--quiet", &verify]).is_ok() {
            let raw = run(
                dir,
                &["diff", "--name-only", "--diff-filter=AM", "-z", &format!("{name}..HEAD")],
            )?;
            return Ok(split_z(&raw));
        }
    }
    let raw = run(dir, &["ls-tree", "-r", "--name-only", "-z", "HEAD"])?;
    Ok(split_z(&raw))
}

/// Lista o caminho no `.gitignore` da raiz se ele ainda não estiver lá.
/// `true` quando o arquivo foi alterado. Ser listado não tira nada do índice —
/// por isso esta função é usada junto com `untrack`, nunca sozinha.
pub fn ensure_ignored(dir: &Path, path: &str) -> Result<bool, String> {
    validate_path(path)?;
    let wanted = path
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    if wanted.is_empty() {
        return Err("caminho de arquivo inválido".into());
    }
    let file = dir.join(".gitignore");
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();
    let listado = text.lines().any(|l| {
        let l = l.trim();
        !l.is_empty()
            && !l.starts_with('#')
            && l.replace('\\', "/")
                .trim_start_matches("./")
                .trim_end_matches('/')
                == wanted
    });
    if listado {
        return Ok(false);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&wanted);
    text.push('\n');
    std::fs::write(&file, text).map_err(|e| format!("não foi possível escrever o .gitignore: {e}"))?;
    Ok(true)
}

/// Onde o Git guarda os arquivos de controle desta pasta. Num repositório comum
/// é o `.git`; numa pasta de worktree o `.git` é um arquivo apontando para a
/// pasta real — e é essa que tem as marcas de operação.
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let ponto = dir.join(".git");
    if ponto.is_dir() {
        return Some(ponto);
    }
    let texto = std::fs::read_to_string(&ponto).ok()?;
    let bruto = texto.trim().strip_prefix("gitdir:")?.trim();
    if bruto.is_empty() {
        return None;
    }
    let apontado = PathBuf::from(bruto);
    Some(if apontado.is_absolute() {
        apontado
    } else {
        dir.join(apontado)
    })
}

/// A operação que o Git deixou parada no meio, lida dos arquivos de controle do
/// `.git`. Cada uma dessas marcas existe só enquanto a operação espera resolução,
/// então isto não é suposição: é o que o git gravou.
pub fn operation_in_progress(dir: &Path) -> Option<String> {
    let git = git_dir(dir)?;
    let ha = |nome: &str| git.join(nome).exists();
    if git.join("rebase-merge").is_dir() || git.join("rebase-apply").is_dir() {
        return Some("rebase".into());
    }
    if ha("MERGE_HEAD") {
        return Some("merge".into());
    }
    if ha("CHERRY_PICK_HEAD") {
        return Some("cherry-pick".into());
    }
    if ha("REVERT_HEAD") {
        return Some("revert".into());
    }
    None
}

pub fn status(dir: &Path) -> Result<RepoStatus, String> {
    let raw = run(dir, &["status", "--porcelain=v2", "-z"])?;
    let files = parse_status(&raw);
    let branch = run(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let detached_at = if branch.is_some() {
        None
    } else {
        run(dir, &["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
    };
    let upstream = run(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "@{u}");
    let (ahead, behind) = match upstream.as_deref() {
        Some(_) => counts(dir).unwrap_or((0, 0)),
        None => (0, 0),
    };
    let conflict_count = files.iter().filter(|f| f.conflict).count();
    Ok(RepoStatus {
        branch,
        detached_at,
        operation: operation_in_progress(dir),
        upstream,
        ahead,
        behind,
        conflict_count,
        files,
    })
}

fn counts(dir: &Path) -> Option<(i64, i64)> {
    let text = run(dir, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]).ok()?;
    let mut cols = text.split_whitespace();
    let behind = cols.next()?.parse().ok()?;
    let ahead = cols.next()?.parse().ok()?;
    Some((ahead, behind))
}

/// Interpreta `status --porcelain=v2 -z`.
///
/// Os registros têm um número fixo de campos antes do caminho, e o caminho pode
/// conter espaços: `1` tem 8, `2` tem 9 (mais o caminho de origem no próximo
/// item separado por NUL) e `u` tem 10.
pub fn parse_status(raw: &str) -> Vec<StatusFile> {
    let mut out = Vec::new();
    let mut items = raw.split('\0');
    while let Some(token) = items.next() {
        if token.is_empty() {
            continue;
        }
        let tag = token.chars().next().unwrap_or(' ');
        match tag {
            '1' | '2' | 'u' => {
                let fixed = match tag {
                    '1' => 8,
                    '2' => 9,
                    _ => 10,
                };
                let fields: Vec<&str> = token.splitn(fixed + 1, ' ').collect();
                if fields.len() < fixed + 1 {
                    continue;
                }
                let xy = fields[1];
                let index_code = xy.chars().next().unwrap_or('.');
                let worktree_code = xy.chars().nth(1).unwrap_or('.');
                let path = fields[fixed].to_string();
                let old_path = if tag == '2' { items.next().map(str::to_string) } else { None };
                out.push(make_status_file(
                    path,
                    old_path,
                    index_code,
                    worktree_code,
                    tag == 'u',
                    fields[2].chars().next().unwrap_or('.'),
                ));
            }
            '?' => {
                out.push(make_status_file(
                    token[2..].to_string(),
                    None,
                    '?',
                    '?',
                    false,
                    '.',
                ));
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn make_status_file(
    path: String,
    old_path: Option<String>,
    index_code: char,
    worktree_code: char,
    conflict: bool,
    sub: char,
) -> StatusFile {
    let staged = index_code != '.' && index_code != '?';
    let unstaged = worktree_code != '.' && worktree_code != '?';
    let kind = if conflict {
        "conflito"
    } else if sub == 'T' {
        // o tipo do arquivo mudou (regular ↔ symlink ↔ submódulo)
        "tipo alterado"
    } else {
        match index_code.max(worktree_code) {
            'A' => "adicionado",
            'D' => "removido",
            'R' => "renomeado",
            'C' => "copiado",
            'T' => "tipo alterado",
            'U' => "conflito",
            '?' => "não rastreado",
            _ => "modificado",
        }
    };
    StatusFile {
        path,
        old_path,
        index_code,
        worktree_code,
        staged,
        unstaged,
        untracked: index_code == '?' && worktree_code == '?',
        conflict,
        kind: kind.into(),
    }
}

// ---------- diff ----------

/// Diffs de um caminho (ou de todo o repo com `path = None`).
///
/// `commit` mostra o que aquele commit mudou. O intervalo é montado a partir do
/// primeiro pai — `git diff <commit>` sozinho compararia com a árvore de
/// trabalho, e `{commit}^!` falha no primeiro commit do histórico.
pub fn diff(
    dir: &Path,
    path: Option<&str>,
    staged: bool,
    commit: Option<&str>,
) -> Result<Vec<FileDiff>, String> {
    let range = match commit {
        Some(c) => {
            validate_id(c)?;
            let parent = commit_parent(dir, c).unwrap_or_else(|| EMPTY_TREE.to_string());
            Some(format!("{parent}..{c}"))
        }
        None => None,
    };
    let mut args: Vec<String> = vec!["diff".into(), "--no-color".into(), "--unified=3".into()];
    match &range {
        Some(r) => args.push(r.clone()),
        None => {
            if staged {
                args.push("--cached".into());
            }
        }
    }
    if let Some(p) = path {
        args.push("--".into());
        args.push(p.to_string());
    }
    let text = run(dir, &args)?;
    Ok(parse_diff(&text))
}

/// Divide a saída de `git diff` em arquivos e linhas numeradas.
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_no = 0i64;
    let mut new_no = 0i64;

    // `lines()` (e não split('\n')) evita criar uma última linha fantasma e já
    // remove o '\r' quando o git emite CRLF.
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest
                .split(" b/")
                .nth(1)
                .unwrap_or(rest.trim())
                .to_string();
            files.push(FileDiff {
                path,
                old_path: None,
                status_char: 'M',
                hunks: Vec::new(),
                binary: false,
            });
            old_no = 0;
            new_no = 0;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some(rest) = line.strip_prefix("--- a/") {
            file.old_path = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ b/") {
            file.path = rest.to_string();
            continue;
        }
        if line.starts_with("Binary files ") {
            file.binary = true;
            continue;
        }
        if line.starts_with("new file mode ") {
            file.status_char = 'A';
            continue;
        }
        if line.starts_with("deleted file mode ") {
            file.status_char = 'D';
            continue;
        }
        if let Some(rest) = line.strip_prefix("rename from ") {
            file.status_char = 'R';
            file.old_path = Some(rest.to_string());
            continue;
        }
        if line.starts_with("@@ ") || line == "@@" {
            if let Some((o_start, o_lines, n_start, n_lines)) = parse_hunk_header(line) {
                old_no = o_start;
                new_no = n_start;
                file.hunks.push(DiffHunk {
                    header: line.to_string(),
                    old_start: o_start,
                    old_lines: o_lines,
                    new_start: n_start,
                    new_lines: n_lines,
                    lines: Vec::new(),
                });
            }
            continue;
        }
        if line.starts_with("index ") || line.starts_with("similarity ") || line.starts_with("Mode") {
            continue;
        }
        let Some(h) = file.hunks.last_mut() else { continue };
        let (kind, text_without_mark) = match line.chars().next() {
            Some('+') => ("add", &line[1..]),
            Some('-') => ("del", &line[1..]),
            Some(' ') => ("ctx", &line[1..]),
            _ => ("ctx", line),
        };
        let entry = DiffLine {
            kind: kind.to_string(),
            old_no: if kind != "add" { Some(old_no) } else { None },
            new_no: if kind != "del" { Some(new_no) } else { None },
            text: text_without_mark.to_string(),
        };
        if kind != "add" {
            old_no += 1;
        }
        if kind != "del" {
            new_no += 1;
        }
        h.lines.push(entry);
    }
    files
}

/// `@@ -12,7 +12,9 @@` → `(12, 7, 12, 9)`.
fn parse_hunk_header(header: &str) -> Option<(i64, i64, i64, i64)> {
    let inside = header.split("@@ ").nth(1)?;
    let range_part = inside.split(" @@").next()?;
    let (old, new) = range_part.split_once(" +")?;
    let old = old.strip_prefix('-')?;
    let (o_start, o_lines) = parse_range(old)?;
    let (n_start, n_lines) = parse_range(new)?;
    Some((o_start, o_lines, n_start, n_lines))
}

fn parse_range(text: &str) -> Option<(i64, i64)> {
    let mut parts = text.split(',');
    let start: i64 = parts.next()?.parse().ok()?;
    let lines = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
    Some((start, lines))
}

// ---------- staging / commit ----------

pub fn stage(dir: &Path, paths: &[String]) -> Result<(), String> {
    for path in paths {
        run(dir, &["add", "--", path])?;
    }
    Ok(())
}

pub fn unstage(dir: &Path, paths: &[String]) -> Result<(), String> {
    for path in paths {
        // `restore --staged` volta o índice ao HEAD sem tocar no arquivo.
        run(dir, &["restore", "--staged", "--", path])?;
    }
    Ok(())
}

/// Aplica/desaplica hunks escolhidos no índice, reconstruindo o patch.
/// `selected` são os índices (0-based) dos hunks daquele arquivo.
pub fn stage_hunks(
    dir: &Path,
    path: &str,
    selected: &[usize],
    staged: bool,
    reverse: bool,
) -> Result<usize, String> {
    let files = diff(dir, Some(path), staged, None)?;
    let Some(file) = files.first() else {
        return Err("nenhuma alteração encontrada para este arquivo".into());
    };
    if file.binary {
        return Err("arquivos binários não podem ser aplicados por hunk".into());
    }
    let mut patch = String::new();
    patch.push_str(&format!("diff --git a/{p} b/{p}\n", p = file.path));
    if let Some(old) = &file.old_path {
        patch.push_str(&format!("--- a/{old}\n"));
    } else {
        patch.push_str(&format!("--- a/{p}\n", p = file.path));
    }
    patch.push_str(&format!("+++ b/{p}\n", p = file.path));
    let mut applied = 0usize;
    for (i, hunk) in file.hunks.iter().enumerate() {
        if !selected.contains(&i) {
            continue;
        }
        applied += 1;
        patch.push_str(&hunk.header);
        patch.push('\n');
        for line in &hunk.lines {
            let mark = match line.kind.as_str() {
                "add" => '+',
                "del" => '-',
                _ => ' ',
            };
            patch.push(mark);
            patch.push_str(&line.text);
            patch.push('\n');
        }
    }
    if applied == 0 {
        return Err("nenhum hunk selecionado".into());
    }
    let mut args = vec!["apply", "--cached", "--recount"];
    if reverse {
        args.push("--reverse");
    }
    // `-` lê o patch da stdin.
    args.push("-");
    run_with_stdin(dir, &args, patch.as_bytes()).map(|_| applied)
}

/// Commit das alterações do índice. Nunca pula hooks.
pub fn commit(dir: &Path, message: &str, amend: bool) -> Result<String, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("escreva a mensagem do commit".into());
    }
    let mut args: Vec<&str> = vec!["commit"];
    if amend {
        args.push("--amend");
    }
    args.extend(["-m", message]);
    run(dir, &args)?;
    run(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

pub fn discard_file(dir: &Path, path: &str, staged: bool) -> Result<(), String> {
    if staged {
        return Err("para desfazer um arquivo já enviado ao índice, remova do staging antes".into());
    }
    run(dir, &["restore", "--worktree", "--", path])?;
    Ok(())
}

// ---------- histórico ----------

const LOG_FORMAT: &str = "%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%P";

/// Linhas do `git log --graph`: o prefixo de decoração vira as trilhas do
/// gráfico desenhado na interface.
pub fn graph(dir: &Path, limit: i64, all_refs: bool) -> Result<Vec<GraphRow>, String> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--graph".into(),
        "--decorate=short".into(),
        "--color=never".into(),
        format!("--pretty=format:{LOG_FORMAT}"),
        format!("--max-count={}", limit.clamp(1, 2000)),
    ];
    if all_refs {
        args.extend(["--branches".into(), "--tags".into(), "--remotes".into()]);
    }
    let text = run(dir, &args)?;
    Ok(parse_graph(&text))
}

pub fn parse_graph(text: &str) -> Vec<GraphRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let split = line.find(|c: char| !matches!(c, '*' | '|' | '/' | '\\' | '<' | '>' | ' '));
        let Some(at) = split else { continue };
        let (graph_prefix, record) = line.split_at(at);
        if record.is_empty() {
            continue;
        }
        if let Some(commit) = parse_commit_line(record) {
            rows.push(GraphRow {
                graph: graph_prefix.to_string(),
                commit,
            });
        }
    }
    rows
}

fn parse_commit_line(record: &str) -> Option<CommitInfo> {
    let fields: Vec<&str> = record.split(SEP).collect();
    if fields.len() < 8 {
        return None;
    }
    Some(CommitInfo {
        id: fields[0].to_string(),
        short_id: fields[1].to_string(),
        author_name: fields[2].to_string(),
        author_email: fields[3].to_string(),
        unix_date: fields[4].parse().unwrap_or(0),
        refs: parse_refs(fields[5]),
        subject: fields[6].to_string(),
        parents: fields[7].split(' ').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect(),
        body: String::new(),
    })
}

/// `HEAD -> main, tag: v1.2, origin/main` → nomes e tipo de cada referência.
fn parse_refs(raw: &str) -> Vec<String> {
    raw.split(", ")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.strip_prefix("tag: ").unwrap_or(s).to_string())
        .collect()
}

/// Arquivos alterados por um commit (para o painel de detalhe).
pub fn commit_detail(dir: &Path, id: &str) -> Result<CommitDetail, String> {
    validate_id(id)?;
    let single = run(
        dir,
        &["show", "--no-patch", &format!("--pretty=format:{LOG_FORMAT}"), id],
    )?;
    let mut commit = parse_commit_line(single.trim()).ok_or_else(|| "commit não reconhecido".to_string())?;
    commit.body = run(dir, &["log", "-1", "--pretty=%B", id]).unwrap_or_default().trim().to_string();
    let name_status = run(dir, &["show", "--pretty=format:", "--name-status", "-M", id])?;
    let files = parse_name_status(&name_status);
    let nums = run(dir, &["show", "--pretty=format:", "--numstat", "-M", id])?;
    let (additions, deletions) = sum_numstat(&nums);
    Ok(CommitDetail {
        commit,
        files,
        total_additions: additions,
        total_deletions: deletions,
    })
}

fn parse_name_status(text: &str) -> Vec<StatusFile> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 2 {
            continue;
        }
        let code = cols[0].chars().next().unwrap_or('M').to_ascii_uppercase();
        // renames e cópias vêm como `R100 <origem> <destino>`
        let (path, old_path) = if matches!(code, 'R' | 'C') && cols.len() >= 3 {
            (cols[2].to_string(), Some(cols[1].to_string()))
        } else {
            (cols[1].to_string(), None)
        };
        out.push(make_status_file(path, old_path, code, '.', false, '.'));
    }
    out
}

fn sum_numstat(text: &str) -> (i64, i64) {
    let mut add = 0i64;
    let mut del = 0i64;
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 2 {
            continue;
        }
        // Arquivos binários vêm como "-" e não têm contagem.
        if let Ok(n) = cols[0].parse::<i64>() {
            add += n;
        }
        if let Ok(n) = cols[1].parse::<i64>() {
            del += n;
        }
    }
    (add, del)
}

/// Diferença entre duas revisões (commits ou branches).
pub fn compare(dir: &Path, from: &str, to: &str) -> Result<Vec<FileDiff>, String> {
    validate_id(from)?;
    validate_id(to)?;
    let text = run(dir, &["diff", "--no-color", &format!("{from}..{to}")])?;
    Ok(parse_diff(&text))
}

/// Caminho relativo de arquivo dentro do repositório. Nada de `-` inicial
/// (opções do git), `..` (escapada) nem caminho absoluto.
fn validate_path(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return Err("caminho de arquivo inválido".into());
    }
    if trimmed.starts_with('-') || trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(format!("caminho fora do repositório ou opção indevida: {trimmed}"));
    }
    if trimmed.contains('\0') || trimmed.contains('\n') {
        return Err("caminho com caractere de controle".into());
    }
    if trimmed.split(['/', '\\']).any(|part| part == "..") {
        return Err(format!("caminho com .. não é permitido: {trimmed}"));
    }
    Ok(())
}

fn validate_id(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 200 {
        return Err("referência inválida".into());
    }
    if trimmed.starts_with('-') {
        return Err("referência não pode começar com '-'".into());
    }
    if trimmed.contains("..") || trimmed.contains(' ') || trimmed.contains('\0') || trimmed.contains('\n') {
        return Err("referência inválida".into());
    }
    Ok(())
}

// ---------- branches / refs ----------

pub fn branches(dir: &Path) -> Result<Vec<BranchInfo>, String> {
    const FMT: &str = "%(if)%(HEAD)%(then)H%(else)-%(end)%1f%(refname:short)%1f%(upstream:short)%1f%(upstream:track)%1f%(objectname:short)%1f%(committerdate:unix)%1f%(subject)";
    let mut out = Vec::new();
    for scope in ["refs/heads", "refs/remotes"] {
        let remoto = scope == "refs/remotes";
        let text = run(dir, &["for-each-ref", &format!("--format={FMT}"), scope])?;
        for line in text.lines() {
            if let Some(info) = parse_branch_line(line, remoto) {
                out.push(info);
            }
        }
    }
    out.sort_by(|a, b| {
        b.is_head
            .cmp(&a.is_head)
            .then_with(|| a.remote.cmp(&b.remote))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn parse_branch_line(line: &str, remoto: bool) -> Option<BranchInfo> {
    let cols: Vec<&str> = line.split(SEP).collect();
    if cols.len() < 7 {
        return None;
    }
    let is_head = cols[0] == "H";
    let name = cols[1].to_string();
    // apenas os refs remotos têm `origin/branch`; um branch local com barra
    // (feature/x) não pode ser confundido com um remoto.
    let remote = if remoto { name.split('/').next().map(|s| s.to_string()) } else { None };
    let (ahead, behind) = parse_track(cols[2]);
    Some(BranchInfo {
        name,
        is_head,
        remote,
        ahead,
        behind,
        last_subject: cols[6].to_string(),
        last_unix_date: cols[5].parse().unwrap_or(0),
    })
}

/// `[ahead 2, behind 1]` → (2, 1).
fn parse_track(raw: &str) -> (i64, i64) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in raw.trim_matches(['[', ']']).split(", ") {
        let mut cols = part.split(' ');
        match (cols.next(), cols.next()) {
            (Some("ahead"), Some(n)) => ahead = n.parse().unwrap_or(0),
            (Some("behind"), Some(n)) => behind = n.parse().unwrap_or(0),
            _ => {}
        }
    }
    (ahead, behind)
}

pub fn checkout(dir: &Path, name: &str, create: bool, start_point: Option<&str>) -> Result<(), String> {
    validate_id(name)?;
    if create {
        let mut args = vec!["checkout", "-b", name];
        if let Some(start) = start_point {
            validate_id(start)?;
            args.push(start);
        }
        run(dir, &args)?;
        return Ok(());
    }
    run(dir, &["checkout", name])?;
    Ok(())
}

pub fn rename_branch(dir: &Path, from: &str, to: &str) -> Result<(), String> {
    validate_id(from)?;
    validate_id(to)?;
    run(dir, &["branch", "-m", from, to])?;
    Ok(())
}

pub fn delete_branch(dir: &Path, name: &str, force: bool) -> Result<(), String> {
    validate_id(name)?;
    let flag = if force { "-D" } else { "-d" };
    run(dir, &["branch", flag, name])?;
    Ok(())
}

pub fn merge(dir: &Path, branch: &str) -> Result<String, String> {
    validate_id(branch)?;
    run(dir, &["merge", "--no-edit", branch])
}

pub fn rebase(dir: &Path, upstream: &str) -> Result<String, String> {
    validate_id(upstream)?;
    run(dir, &["rebase", upstream])
}

/// As operações que podem ficar paradas esperando resolução de conflito. O nome
/// é o mesmo que `operation_in_progress` devolve, então a interface cancela e
/// continua exatamente o que o Git tem em andamento — nunca um palpite.
const OPERACOES: &[&str] = &["merge", "rebase", "cherry-pick", "revert"];

fn operacao(kind: &str) -> Result<&'static str, String> {
    OPERACOES
        .iter()
        .find(|o| *o == &kind)
        .copied()
        .ok_or_else(|| format!("operação desconhecida: {kind}"))
}

/// Cancela a operação e devolve a pasta ao estado de antes dela.
pub fn abort_operation(dir: &Path, kind: &str) -> Result<(), String> {
    let sub = operacao(kind)?;
    run(dir, &[sub, "--abort"]).map(|_| ())
}

/// Retoma a operação depois de resolver os conflitos. O git quer escrever a
/// mensagem do commit e chamaria um editor que não existe aqui; `core.editor=true`
/// aceita a mensagem que o próprio git já preparou no `.git`.
pub fn continue_operation(dir: &Path, kind: &str) -> Result<String, String> {
    let sub = operacao(kind)?;
    if sub == "merge" {
        // merge não tem "--continue": o que fecha a operação é o commit normal.
        return Err("para concluir o merge, marque os conflitos como resolvidos e faça o commit".into());
    }
    run(dir, &["-c", "core.editor=true", sub, "--continue"])
}

/// Abandona o passo parado (commit já aplicado em cherry-pick/rebase, por
/// exemplo) sem gravar nada dele. Usado quando o git avisa que não sobrou
/// mudança nenhuma para aplicar.
pub fn skip_operation(dir: &Path, kind: &str) -> Result<String, String> {
    let sub = operacao(kind)?;
    if sub == "merge" {
        return Err("o merge não tem etapa para pular — cancele ou faça o commit".into());
    }
    run(dir, &[sub, "--skip"])
}

/// Resolve um conflito com o lado escolhido e marca como resolvido.
pub fn resolve_conflict(dir: &Path, path: &str, side: &str) -> Result<(), String> {
    if side != "ours" && side != "theirs" {
        return Err("lado de resolução inválido".into());
    }
    let flag = if side == "ours" { "--ours" } else { "--theirs" };
    run(dir, &["checkout", flag, "--", path])?;
    run(dir, &["add", "--", path])?;
    Ok(())
}

// ---------- reversão ----------

/// Quantos pais um commit tem: 1 no caso comum, 2+ quando é um merge.
fn parent_count(dir: &Path, id: &str) -> usize {
    run(dir, &["rev-list", "--parents", "-n", "1", id])
        .map(|line| line.split_whitespace().count().saturating_sub(1))
        .unwrap_or(1)
}

/// Commit de merge tem mais de um pai e o git exige escolher qual deles continua.
/// A checagem acontece antes de chamar o git para a resposta vir em português.
fn pai_para_continuar(dir: &Path, id: &str, mainline: Option<i64>) -> Result<Option<i64>, String> {
    let parents = parent_count(dir, id);
    if let Some(main) = mainline {
        if main < 1 || main as usize > parents {
            return Err(format!("pai inválido: este commit tem {parents} pais"));
        }
        return Ok(Some(main));
    }
    if parents > 1 {
        return Err(format!(
            "este commit é um merge com {parents} pais; escolha qual deles continua (o pai 1 é o ramo que recebeu o merge)"
        ));
    }
    Ok(None)
}

/// Desfaz um commit gravando um commit novo com as mudanças invertidas. O
/// histórico existente não é reescrito, então isto funciona mesmo em ramo que
/// já foi publicado — o `push` posterior propaga a reversão.
pub fn revert(dir: &Path, id: &str, mainline: Option<i64>) -> Result<String, String> {
    validate_id(id)?;
    let mut args: Vec<String> = vec!["revert".into(), "--no-edit".into()];
    if let Some(main) = pai_para_continuar(dir, id, mainline)? {
        args.push("-m".into());
        args.push(main.to_string());
    }
    args.push(id.to_string());
    run(dir, &args)
}

/// Reproduz aqui um commit que existe em outro ramo, gravando um commit novo. O
/// ramo de origem não é tocado e nada vai ao remoto por aqui. Como no revert, um
/// commit de merge só passa com o pai escolhido.
pub fn cherry_pick(dir: &Path, id: &str, mainline: Option<i64>) -> Result<String, String> {
    validate_id(id)?;
    let mut args: Vec<String> = vec!["cherry-pick".into()];
    if let Some(main) = pai_para_continuar(dir, id, mainline)? {
        args.push("-m".into());
        args.push(main.to_string());
    }
    args.push(id.to_string());
    run(dir, &args)
}

/// Move a cabeça do ramo para outro commit. `soft` e `mixed` só mexem no
/// histórico e no índice; `hard` também apaga o conteúdo em trabalho, por isso
/// a interface só chama depois de confirmação explícita.
pub fn reset(dir: &Path, mode: &str, target: &str) -> Result<String, String> {
    let flag = match mode {
        "soft" => "--soft",
        "mixed" => "--mixed",
        "hard" => "--hard",
        other => return Err(format!("modo de reversão desconhecido: {other}")),
    };
    validate_id(target)?;
    let antes = short_head(dir);
    let saida = run(dir, &["reset", flag, target])?;
    Ok(format!(
        "{}\ncabeça agora em {} (antes: {})",
        saida.trim(),
        short_head(dir),
        antes
    ))
}

fn short_head(dir: &Path) -> String {
    run(dir, &["rev-parse", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "desconhecida".to_string())
}

// ---------- trabalho guardado ----------

const STASH_FORMAT: &str = "%gd\u{1f}%h\u{1f}%ci\u{1f}%s";

/// O que está guardado, na ordem do git (`stash@{0}` é o mais recente). Sem
/// guardados a lista volta vazia.
pub fn stash_list(dir: &Path) -> Result<Vec<StashEntry>, String> {
    let raw = run(dir, &["stash", "list", &format!("--format={STASH_FORMAT}")])?;
    Ok(raw
        .lines()
        .filter_map(|linha| {
            let campo: Vec<&str> = linha.split(SEP).collect();
            if campo.len() < 4 {
                return None;
            }
            let indice = campo[0].strip_prefix("stash@{")?.strip_suffix('}')?.parse().ok()?;
            Some(StashEntry {
                index: indice,
                selector: campo[0].to_string(),
                sha: campo[1].to_string(),
                date: campo[2].to_string(),
                subject: campo[3].to_string(),
            })
        })
        .collect())
}

/// O endereço do guardado escolhido, conferido na lista atual. O git responderia
/// com um "fatal" em inglês sobre um índice que a tela mostrava existir.
fn stash_endereco(dir: &Path, index: usize) -> Result<String, String> {
    let lista = stash_list(dir)?;
    if lista.is_empty() {
        return Err("não há trabalho guardado neste repositório".into());
    }
    lista
        .iter()
        .find(|e| e.index == index)
        .map(|e| e.selector.clone())
        .ok_or_else(|| {
            format!(
                "o guardado {index} não existe mais: a lista atual tem {} entrada(s). Atualize a lista.",
                lista.len()
            )
        })
}

/// Guarda o trabalho em andamento e deixa a pasta limpa, para se poder trocar de
/// ramo sem perder nada. `include_untracked` leva também os arquivos que o git
/// ainda não conhece — sem essa opção eles ficam na pasta.
pub fn stash_push(dir: &Path, message: Option<&str>, include_untracked: bool) -> Result<String, String> {
    let atual = status(dir)?;
    if atual.conflict_count > 0 {
        return Err(format!(
            "há {} arquivo(s) em conflito — resolva ou cancele a operação antes de guardar",
            atual.conflict_count
        ));
    }
    let mut args: Vec<String> = vec!["stash".into(), "push".into()];
    if include_untracked {
        args.push("--include-untracked".into());
    }
    let recado = message.map(str::trim).filter(|m| !m.is_empty());
    if let Some(texto) = recado {
        args.push("-m".into());
        args.push(texto.to_string());
    }
    let saida = run(dir, &args)?;
    // Sem alteração o git sai com código 0 e avisa no stdout: nada foi guardado,
    // e é isso que a pessoa precisa ler.
    if saida.to_lowercase().contains("no local changes") {
        return Ok("nada para guardar: não há alteração em trabalho nem no índice".into());
    }
    Ok(saida.trim().to_string())
}

/// Traz o guardado de volta para a pasta. `pop` também o retira da lista, mas
/// quando há conflito o git **mantém** a entrada — dito aqui em vez de escondido.
pub fn stash_apply(dir: &Path, index: usize, pop: bool) -> Result<String, String> {
    let endereco = stash_endereco(dir, index)?;
    let acao = if pop { "pop" } else { "apply" };
    let feito = run(dir, &["stash", acao, &endereco]);
    let conflitos = status(dir).map(|s| s.conflict_count).unwrap_or(0);
    if let Err(e) = feito {
        // Conflito na aplicação é um resultado, não uma pane: o git sai com
        // código diferente de zero para avisar, e a pasta fica em conflito.
        if conflitos == 0 {
            return Err(e);
        }
    }
    if conflitos > 0 {
        return Ok(
            "aplicado com conflito na pasta: o guardado ficou na lista. Resolva os conflitos para ficar com as duas mudanças.".into(),
        );
    }
    Ok(if pop {
        "trabalho aplicado e o guardado saiu da lista".into()
    } else {
        "trabalho aplicado — o guardado continua na lista".into()
    })
}

/// Exclui um guardado. O conteúdo dele não entra na pasta antes, então isto perde
/// o trabalho: a interface pede confirmação e a auditoria registra.
pub fn stash_drop(dir: &Path, index: usize) -> Result<String, String> {
    let endereco = stash_endereco(dir, index)?;
    let saida = run(dir, &["stash", "drop", &endereco])?;
    Ok(saida.trim().to_string())
}

// ---------- sincronização ----------

pub fn fetch(dir: &Path, remote: Option<&str>) -> Result<String, String> {
    match remote {
        Some(r) => {
            validate_id(r)?;
            run(dir, &["fetch", "--prune", r])
        }
        None => run(dir, &["fetch", "--all", "--prune"]),
    }
}

/// Traz a cabeça de um pull request para o repositório local e muda para uma
/// branch de leitura (`github/pr-<n>`). O refspec `refs/pull/<n>/head` é público
/// e só de leitura: nada é escrito no remoto. A branch usa `-B` porque pode
/// existir de uma leitura anterior e deve apontar para o que acabou de chegar.
pub fn checkout_pull(dir: &Path, remote: &str, number: i64) -> Result<String, String> {
    validate_id(remote)?;
    if number <= 0 {
        return Err("número de pull request inválido".into());
    }
    let branch = format!("github/pr-{number}");
    run(dir, &["fetch", remote, &format!("refs/pull/{number}/head")])
        .map_err(|e| format!("não foi possível buscar o pull request #{number}: {e}"))?;
    run(dir, &["checkout", "-B", &branch, "FETCH_HEAD"])
        .map_err(|e| format!("o pull request #{number} foi buscado, mas a mudança para {branch} falhou: {e}"))?;
    Ok(branch)
}

/// `--ff-only` por padrão: nunca cria merge automático sem o usuário pedir.
pub fn pull(dir: &Path, strategy: &str) -> Result<String, String> {
    let flag = match strategy {
        "ff-only" => "--ff-only",
        "merge" => "--no-rebase",
        "rebase" => "--rebase",
        other => return Err(format!("estratégia de pull desconhecida: {other}")),
    };
    run(dir, &["pull", flag])
}

pub fn push(dir: &Path, remote: Option<&str>, branch: Option<&str>, set_upstream: bool) -> Result<String, String> {
    let mut args: Vec<String> = vec!["push".into()];
    if set_upstream {
        args.push("--set-upstream".into());
    }
    if let Some(r) = remote {
        validate_id(r)?;
        args.push(r.to_string());
    }
    // `--set-upstream` sem branch no comando: o git exige o refspec, então
    // usamos a branch atual — que é exatamente o que o usuário quis dizer.
    let escolhida = match branch {
        Some(b) => Some(b.to_string()),
        None if set_upstream => Some(
            run(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
                .map_err(|_| "a cabeça está destacada; escolha a branch antes de definir o rastreio")?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    if let Some(b) = escolhida {
        validate_id(&b)?;
        args.push(b);
    }
    run(dir, &args)
}

// ---------- tags / remotes ----------

/// Envia uma tag que já existe localmente. `push` com refspec explícito: sem
/// isto o git empurraria apenas o ramo, e uma publicação ficaria sem a marca.
pub fn push_tag(dir: &Path, remote: &str, name: &str) -> Result<String, String> {
    validate_id(remote)?;
    validate_id(name)?;
    run(dir, &["push", remote, &format!("refs/tags/{name}")])
}

/// O remoto de uma publicação: `origin` quando existe, senão o primeiro
/// cadastrado. Sem remoto não há para onde publicar — o erro diz isso.
pub fn default_remote(dir: &Path) -> Result<String, String> {
    let texto = run(dir, &["remote"])?;
    let nomes: Vec<&str> = texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if nomes.is_empty() {
        return Err("este repositório não tem remoto cadastrado — adicione um antes de publicar".into());
    }
    Ok(nomes
        .iter()
        .find(|n| **n == "origin")
        .map(|n| n.to_string())
        .unwrap_or_else(|| nomes[0].to_string()))
}

const ACCESS_TIMEOUT: Duration = Duration::from_secs(25);

/// Prova real de que esta máquina consegue falar com o remoto: `git ls-remote`
/// com prazo, stdin fechado e prompt de credencial desativado pelo `base_cmd`.
/// Só lê refs: nada é escrito no remoto e a URL nunca aparece no resultado.
pub fn remote_access(dir: &Path, remote: &str) -> Result<RemoteAccess, String> {
    validate_id(remote)?;
    let mut child = base_cmd(dir)
        .arg("ls-remote")
        .arg(remote)
        .arg("HEAD")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "o git não foi encontrado no PATH — instale o Git para usar esta seção".to_string()
            }
            _ => format!("não foi possível executar o git: {e}"),
        })?;
    let feito = crate::sshexec::collect(&mut child, ACCESS_TIMEOUT, &AtomicBool::new(false));
    Ok(describe_access(remote, &feito))
}

/// Traduz o desfecho do `ls-remote` em uma frase de liberação. A distinção que
/// importa para o usuário: remoto que responde mas recusa a credencial é "sem
/// liberação", e um remoto recém-criado sem commits é sucesso, não falha.
fn describe_access(remote: &str, feito: &crate::sshexec::ExecResult) -> RemoteAccess {
    let negado = |motivo: String| RemoteAccess {
        ok: false,
        remote: remote.to_string(),
        detail: motivo,
    };
    if feito.timed_out {
        return negado(format!(
            "{remote} não respondeu em {}s — confira a conexão e o endereço do remoto.",
            ACCESS_TIMEOUT.as_secs()
        ));
    }
    if feito.exit_code == Some(0) {
        let head = feito
            .stdout
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        if head.is_empty() {
            return RemoteAccess {
                ok: true,
                remote: remote.to_string(),
                detail: "acesso liberado: o remoto está vazio — a primeira publicação cria a base.".into(),
            };
        }
        return RemoteAccess {
            ok: true,
            remote: remote.to_string(),
            detail: format!("acesso liberado: HEAD do {remote} em {}.", short_ref(&head)),
        };
    }
    let texto = format!("{}\n{}", feito.stderr, feito.stdout).to_lowercase();
    const SEM_CREDENCIAL: &str = "sem liberação: esta máquina não tem credencial válida para ";
    for marca in [
        "permission denied",
        "authentication failed",
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "support for password authentication was removed",
        "invalid username or password",
        "bad credentials",
    ] {
        if texto.contains(marca) {
            return negado(format!(
                "{SEM_CREDENCIAL}{remote}. Coloque a chave no ssh-agent, faça o login do GitHub \
                 ou rode `gh auth login` (Configurações → Acesso da máquina)."
            ));
        }
    }
    if texto.contains("repository not found")
        || texto.contains("does not appear to be a git repository")
    {
        return negado(format!(
            "{remote} não aponta para um repositório acessível — confira o endereço do remoto e, \
             se ele for privado, a liberação desta máquina."
        ));
    }
    for marca in [
        "could not resolve",
        "name or service not known",
        "connection refused",
        "network is unreachable",
        "connection reset",
        "timed out",
    ] {
        if texto.contains(marca) {
            return negado(format!(
                "a máquina não chegou ao servidor do remoto {remote} ({marca}) — verifique a \
                 conexão de rede."
            ));
        }
    }
    let detalhe = reduzido(&format!("{}\n{}", feito.stderr, feito.stdout));
    negado(format!(
        "o teste de acesso a {remote} falhou ({}). Detalhe do git: {detalhe}",
        feito.summary()
    ))
}

/// As três últimas linhas úteis do que o git escreveu, sem estourar a tela.
fn reduzido(texto: &str) -> String {
    let linhas: Vec<&str> = texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tomados = if linhas.len() <= 3 {
        linhas.join(" ")
    } else {
        format!("… {}", linhas[linhas.len() - 3..].join(" "))
    };
    if tomados.is_empty() {
        return "(sem detalhe)".into();
    }
    let corte: String = tomados.chars().take(400).collect();
    if corte.chars().count() == tomados.chars().count() {
        corte
    } else {
        format!("{corte}…")
    }
}

fn short_ref(id: &str) -> String {
    id.chars().take(7).collect()
}

pub fn tags(dir: &Path) -> Result<Vec<TagInfo>, String> {
    const FMT: &str = "%(refname:short)%1f%(objectname:short)%1f%(if)%(*objectname)%(then)%(contents:subject)%(else)%(contents:subject)%(end)";
    let text = run(dir, &["for-each-ref", &format!("--format={FMT}"), "refs/tags"])?;
    let mut out = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split(SEP).collect();
        if cols.is_empty() || cols[0].is_empty() {
            continue;
        }
        out.push(TagInfo {
            name: cols[0].to_string(),
            target: cols.get(1).map(|s| s.to_string()).unwrap_or_default(),
            message: cols.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        });
    }
    Ok(out)
}

pub fn create_tag(dir: &Path, name: &str, target: Option<&str>, message: Option<&str>) -> Result<(), String> {
    validate_id(name)?;
    match message.filter(|m| !m.trim().is_empty()) {
        Some(m) => {
            let mut args = vec![
                "tag".to_string(),
                "-a".to_string(),
                name.to_string(),
                "-m".to_string(),
                m.to_string(),
            ];
            if let Some(t) = target {
                validate_id(t)?;
                args.push(t.to_string());
            }
            run(dir, &args)?;
        }
        None => {
            let mut args = vec!["tag", name];
            if let Some(t) = target {
                validate_id(t)?;
                args.push(t);
            }
            run(dir, &args)?;
        }
    }
    Ok(())
}

pub fn delete_tag(dir: &Path, name: &str) -> Result<(), String> {
    validate_id(name)?;
    run(dir, &["tag", "-d", name])?;
    Ok(())
}

/// Retira a tag do remoto com o refspec de exclusão (`:refs/tags/<nome>`). Só a
/// referência deixa de existir lá: nenhum commit é reescrito e nada é apagado
/// do histórico local — para isso existe `delete_tag`.
pub fn delete_remote_tag(dir: &Path, remote: &str, name: &str) -> Result<String, String> {
    validate_id(remote)?;
    validate_id(name)?;
    run(dir, &["push", remote, &format!(":refs/tags/{name}")])
}

pub fn remotes(dir: &Path) -> Result<Vec<RemoteInfo>, String> {
    let names = run(dir, &["remote"])?;
    let mut out = Vec::new();
    for name in names.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let fetch_url = run(dir, &["remote", "get-url", name]).ok().map(|s| s.trim().to_string());
        let push_url = run(dir, &["remote", "get-url", "--push", name])
            .ok()
            .map(|s| s.trim().to_string());
        out.push(RemoteInfo {
            name: name.to_string(),
            fetch_url,
            push_url,
        });
    }
    Ok(out)
}

pub fn add_remote(dir: &Path, name: &str, url: &str) -> Result<(), String> {
    validate_id(name)?;
    validate_url(url)?;
    run(dir, &["remote", "add", name, url])?;
    Ok(())
}

pub fn set_remote_url(dir: &Path, name: &str, url: &str) -> Result<(), String> {
    validate_id(name)?;
    validate_url(url)?;
    run(dir, &["remote", "set-url", name, url])?;
    Ok(())
}

pub fn remove_remote(dir: &Path, name: &str) -> Result<(), String> {
    validate_id(name)?;
    run(dir, &["remote", "remove", name])?;
    Ok(())
}

fn validate_url(url: &str) -> Result<(), String> {
    let u = url.trim();
    if u.is_empty() || u.len() > 2000 {
        return Err("informe uma URL de repositório".into());
    }
    // Sem shell e sem concatenação: espaços são seguros como argv. O que não
    // pode acontecer é o valor ser lido como uma opção do git.
    if u.starts_with('-') || u.contains('\0') || u.contains('\n') {
        return Err("URL inválida".into());
    }
    let absolute = u.starts_with('/')
        || u.starts_with("~/")
        || (u.as_bytes().get(1) == Some(&b':') && u.as_bytes()[0].is_ascii_alphabetic());
    if !(u.contains("://") || u.contains('@') || absolute) {
        return Err("a URL deve começar com https://, ssh://, usuário@host ou ser um caminho absoluto".into());
    }
    Ok(())
}

// ---------- clone / identidade ----------

pub fn clone(dir_parent: &Path, url: &str, target_name: Option<&str>, branch: Option<&str>) -> Result<String, String> {
    validate_url(url)?;
    // As opções vêm antes dos argumentos posicionais: `git clone <opções> <url> <pasta>`.
    let mut args: Vec<String> = vec!["clone".into()];
    if let Some(b) = branch {
        validate_id(b)?;
        args.extend(["--branch".into(), b.to_string()]);
    }
    args.push(url.to_string());
    if let Some(name) = target_name {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 200
            || name.contains("..")
            || name.contains('/')
            || name.contains('\\')
            || name.starts_with('.')
        {
            return Err("nome da pasta de destino inválido".into());
        }
        args.push(name.to_string());
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    // clone roda no diretório pai, não dentro de um repo existente
    let mut child = base_cmd(dir_parent)
        .args(&refs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("não foi possível executar o git: {e}"))?;
    let feito = crate::sshexec::collect(&mut child, prazo_dos(&refs), &AtomicBool::new(false));
    if feito.timed_out {
        return Err(prazo_expirado(&refs, prazo_dos(&refs)));
    }
    if feito.exit_code != Some(0) {
        return Err(describe_failure(&refs, feito.stderr.as_bytes(), feito.stdout.as_bytes()));
    }
    Ok(feito.stderr.trim().to_string())
}

pub fn identity(dir: &Path) -> Result<(Option<String>, Option<String>), String> {
    let name = run(dir, &["config", "--get", "user.name"]).ok().map(|s| s.trim().to_string());
    let email = run(dir, &["config", "--get", "user.email"]).ok().map(|s| s.trim().to_string());
    Ok((
        name.filter(|s| !s.is_empty()),
        email.filter(|s| !s.is_empty()),
    ))
}

/// Referência para `git diff` de um commit com seu pai (para comparar no UI).
pub fn commit_parent(dir: &Path, id: &str) -> Option<String> {
    validate_id(id).ok()?;
    let text = run(dir, &["rev-parse", &format!("{id}^")]).ok()?;
    let parent = text.trim().to_string();
    if parent.is_empty() { None } else { Some(parent) }
}

pub fn is_repo(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// O commit que a pasta aponta agora. É o que uma execução de pipeline registra
/// como procedência; `None` quando a pasta não é um repositório ou não tem commit.
/// Prazo curto de propósito: quem chama isto segura a alça do banco enquanto lê, e
/// uma pasta lenta (rede, disco dormindo) não pode travar o aplicativo inteiro.
pub fn head_sha(dir: &Path) -> Option<String> {
    let text = executar(dir, &["rev-parse", "HEAD"], Duration::from_secs(3), None).ok()?;
    let sha = text.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn so_fala_de_rede_quem_realmente_fala() {
        assert!(pela_rede(&["clone", "https://x/y"]));
        assert!(pela_rede(&["push", "origin", "main"]));
        assert!(pela_rede(&["fetch"]));
        assert!(pela_rede(&["pull"]));
        assert!(pela_rede(&["ls-remote", "origin"]));
        assert!(!pela_rede(&["status", "--porcelain"]));
        assert!(!pela_rede(&["log", "-n", "50"]));
        assert!(!pela_rede(&[]));
    }

    #[test]
    fn prazo_expirado_diz_quanto_esperou_e_o_que_pode_ser() {
        let local = prazo_expirado(&["status"], PRAZO_LOCAL);
        assert!(local.contains("30 s"), "{local}");
        assert!(local.contains("índice travado"), "{local}");
        let rede = prazo_expirado(&["push", "origin", "main"], PRAZO_REDE);
        assert!(rede.contains("10 min"), "{rede}");
        assert!(rede.contains("credencial"), "{rede}");
    }

    #[test]
    fn um_git_que_nao_termina_devolve_erro_em_vezes_de_travar() {
        // O hook `pre-commit` abaixo nunca devolve o controle. Sem prazo o
        // commit ficaria girando para sempre na tela; com prazo, a chamada
        // volta como erro e dentro de poucos segundos.
        let dir = temp_repo("prazo");
        std::fs::write(dir.join(".git").join("hooks").join("pre-commit"), "#!/bin/sh\nsleep 60\n")
            .expect("escrever o hook");
        write(&dir, "a.txt", &base_file());
        stage(&dir, &["a.txt".into()]).unwrap();
        let antes = std::time::Instant::now();
        let erro = executar(&dir, &["commit", "-m", "nunca termina"], Duration::from_secs(3), None)
            .expect_err("o hook travado deveria impedir o commit");
        let decorrido = antes.elapsed();
        assert!(erro.contains("não respondeu em 3 s"), "mensagem: {erro}");
        assert!(decorrido < Duration::from_secs(20), "o prazo não valiu: {decorrido:?}");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("devops-git-{tag}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar pasta temporária");
        dir
    }

    fn temp_repo(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        run(&dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]).expect("git init");
        run(&dir, &["config", "user.name", "Teste Local"]).unwrap();
        run(&dir, &["config", "user.email", "teste@local"]).unwrap();
        run(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        run(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        dir
    }

    /// Um "servidor" para os testes de push: repositório bare criado a partir do
    /// trabalho, como um remoto real (um repositório comum recusaria o push).
    fn bare_origin(work: &Path) -> PathBuf {
        let nome = work.file_name().unwrap().to_string_lossy().to_string();
        let pais = work.parent().unwrap();
        let bare = pais.join(format!("{nome}.git"));
        let _ = std::fs::remove_dir_all(&bare);
        run(
            work,
            &["clone", "--bare", "-q", ".", &bare.to_string_lossy()],
        )
        .expect("criar remoto bare");
        bare
    }

    /// Caminho no formato que o git aceita como URL local.
    fn url(dir: &Path) -> String {
        dir.to_string_lossy().replace('\\', "/")
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).expect("escrever arquivo");
    }

    // Atalhos: as variáveis locais dos testes chamam `status`/`remotes`, o que
    // ofuscaria as funções de mesmo nome se chamadas diretamente.
    fn read_status(dir: &Path) -> Result<RepoStatus, String> {
        status(dir)
    }

    fn read_remotes(dir: &Path) -> Result<Vec<RemoteInfo>, String> {
        remotes(dir)
    }

    fn read_tags(dir: &Path) -> Result<Vec<TagInfo>, String> {
        tags(dir)
    }

    /// 30 linhas com duas mudanças distantes → dois hunks no diff.
    fn base_file() -> String {
        (1..=30)
            .map(|i| format!("linha {i}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    fn edited_file() -> String {
        base_file()
            .replace("linha 2\n", "linha 2 alterada\n")
            .replace("linha 20\n", "linha 20 alterada\n")
    }

    #[test]
    fn ciclo_real_de_commit_status_e_log() {
        let dir = temp_repo("ciclo");
        write(&dir, "a.txt", &base_file());
        stage(&dir, &["a.txt".into()]).unwrap();
        let first = commit(&dir, "primeiro commit\n\ncorpo do commit", false).unwrap();
        assert_eq!(first.len(), 40);

        let status = read_status(&dir).unwrap();
        assert!(status.files.is_empty(), "repositório limpo após o commit");
        assert!(status.branch.is_some());

        write(&dir, "a.txt", &edited_file());
        write(&dir, "novo.txt", "novo\n");
        let status = read_status(&dir).unwrap();
        let a = status.files.iter().find(|f| f.path == "a.txt").expect("a.txt");
        assert!(a.unstaged && !a.staged);
        let novo = status.files.iter().find(|f| f.path == "novo.txt").expect("novo.txt");
        assert!(novo.untracked);

        stage(&dir, &["a.txt".into()]).unwrap();
        let status = read_status(&dir).unwrap();
        let a = status.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(a.staged && !a.unstaged);

        commit(&dir, "segundo commit", false).unwrap();
        write(&dir, "b.txt", "b\n");
        stage(&dir, &["b.txt".into()]).unwrap();
        commit(&dir, "terceiro commit", false).unwrap();

        let rows = graph(&dir, 10, false).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].commit.subject, "terceiro commit");
        assert_eq!(rows[0].graph, "* ");
        assert_eq!(rows[1].graph, "* ");
        assert!(rows[0].commit.parents.iter().any(|p| p == &rows[1].commit.id));
        assert_eq!(rows[0].commit.refs, vec!["HEAD -> main".to_string()]);

        let detail = commit_detail(&dir, &rows[1].commit.id).unwrap();
        assert_eq!(detail.commit.subject, "segundo commit");
        assert_eq!(detail.files.len(), 1);
        assert!(detail.total_additions >= 1);
        // o primeiro commit tem corpo; o log do gráfico só traz o assunto
        let inicial = commit_detail(&dir, &rows[2].commit.id).unwrap();
        assert_eq!(inicial.commit.subject, "primeiro commit");
        assert!(inicial.commit.body.contains("corpo do commit"), "corpo: {:?}", inicial.commit.body);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commit_vazio_e_rejeitado() {
        let dir = temp_repo("vazio");
        write(&dir, "a.txt", "a\n");
        assert!(commit(&dir, "   ", false).is_err());
        stage(&dir, &["a.txt".into()]).unwrap();
        assert!(commit(&dir, "ok", false).is_ok());
        let err = commit(&dir, "", false).unwrap_err();
        assert!(err.contains("mensagem"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nada_staged_da_erro_claro() {
        let dir = temp_repo("nada");
        write(&dir, "a.txt", "a\n");
        let err = commit(&dir, "sem staging", false).unwrap_err();
        assert!(err.contains("nada para fazer"), "mensagem: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_tem_hunks_com_numeros_de_linha() {
        let dir = temp_repo("diff");
        write(&dir, "a.txt", &base_file());
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        write(&dir, "a.txt", &edited_file());

        let files = diff(&dir, Some("a.txt"), false, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks.len(), 2, "mudanças distantes geram dois hunks");
        let first = &files[0].hunks[0];
        assert_eq!(first.old_start, 1);
        let del = first.lines.iter().find(|l| l.kind == "del").expect("linha removida");
        let add = first.lines.iter().find(|l| l.kind == "add").expect("linha adicionada");
        assert_eq!(del.old_no, Some(2));
        assert_eq!(del.new_no, None);
        assert_eq!(add.new_no, Some(2));
        assert_eq!(add.old_no, None);
        assert_eq!(del.text, "linha 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hunk_individual_entra_no_indice() {
        let dir = temp_repo("hunk");
        write(&dir, "a.txt", &base_file());
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        write(&dir, "a.txt", &edited_file());

        // Só o primeiro hunk vai para o índice.
        let applied = stage_hunks(&dir, "a.txt", &[0], false, false).unwrap();
        assert_eq!(applied, 1);

        let status = read_status(&dir).unwrap();
        let a = status.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(a.staged, "hunk escolhido fica staged");
        assert!(a.unstaged, "o segundo hunk continua fora do índice");

        let staged_files = diff(&dir, Some("a.txt"), true, None).unwrap();
        assert_eq!(staged_files[0].hunks.len(), 1);
        assert_eq!(staged_files[0].hunks[0].old_start, 1);

        // Desfazer o mesmo hunk do índice devolve o arquivo a só-unstaged.
        stage_hunks(&dir, "a.txt", &[0], true, true).unwrap();
        let status = read_status(&dir).unwrap();
        let a = status.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(!a.staged);
        assert!(a.unstaged);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unstage_e_discard_usam_o_indice_e_o_worktree() {
        let dir = temp_repo("unstage");
        write(&dir, "a.txt", "a\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        write(&dir, "a.txt", "b\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        unstage(&dir, &["a.txt".into()]).unwrap();
        let status = read_status(&dir).unwrap();
        let a = status.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert!(!a.staged && a.unstaged);

        discard_file(&dir, "a.txt", false).unwrap();
        let status = read_status(&dir).unwrap();
        assert!(status.files.is_empty(), "desfazer o trabalho limpa o arquivo");
        let err = discard_file(&dir, "a.txt", true).unwrap_err();
        assert!(err.contains("staging"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn primeiro_commit_nao_tem_head_e_o_staging_e_lido_mesmo_assim() {
        let dir = temp_repo("staging");
        write(&dir, "a.txt", "a\n");
        std::fs::create_dir_all(dir.join("srv")).expect("criar pasta srv");
        write(&dir, "srv/.env", "TOKEN=abc123\n");
        stage(&dir, &["a.txt".into(), "srv/.env".into()]).unwrap();
        let staged = staged_files(&dir).unwrap();
        assert!(staged.contains(&"srv/.env".to_string()), "{staged:?}");
        assert!(staged.contains(&"a.txt".to_string()), "{staged:?}");

        // o histórico rastreia; depois de reter no .gitignore o índice ainda
        // precisa ser limpo — é exatamente o que `untrack` faz sem apagar o
        // arquivo local.
        commit(&dir, "base", false).unwrap();
        assert!(tracked_files(&dir).unwrap().contains(&"srv/.env".to_string()));
        untrack(&dir, "srv/.env").unwrap();
        assert!(!tracked_files(&dir).unwrap().contains(&"srv/.env".to_string()));
        assert!(dir.join("srv").join(".env").exists(), "o arquivo local fica intacto");

        let err = untrack(&dir, "../outra-pasta/.env").unwrap_err();
        assert!(err.contains(".."), "{err}");
        let err = untrack(&dir, "--force").unwrap_err();
        assert!(err.contains("fora do repositório"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_ignorado_e_o_que_o_push_levarao() {
        let dir = temp_repo("push-risk");
        std::fs::create_dir_all(dir.join("srv")).expect("criar pasta srv");
        write(&dir, "srv/.env", "TOKEN=abc123\n");
        stage(&dir, &["srv/.env".into()]).unwrap();
        commit(&dir, "base", false).unwrap();

        let blob = blob_at(&dir, "HEAD", "srv/.env").expect("blob do HEAD");
        assert!(String::from_utf8_lossy(&blob).contains("TOKEN=abc123"));
        assert!(blob_at(&dir, "HEAD", "nao-existe.txt").is_none());

        assert!(ensure_ignored(&dir, "srv/.env").unwrap(), "primeira vez escreve");
        assert!(!ensure_ignored(&dir, "srv/.env").unwrap(), "segunda vez é no-op");
        let gitignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
        assert_eq!(gitignore.trim(), "srv/.env");

        // sem branch de rastreio, o conteúdo inteiro do HEAD está em jogo
        let tudo = push_risk_paths(&dir).unwrap();
        assert!(tudo.contains(&"srv/.env".to_string()), "{tudo:?}");

        let bare = bare_origin(&dir);
        add_remote(&dir, "origin", &url(&bare)).unwrap();
        push(&dir, Some("origin"), None, true).unwrap();
        assert!(push_risk_paths(&dir).unwrap().is_empty(), "nada novo para publicar");

        std::fs::create_dir_all(dir.join("src")).expect("criar pasta src");
        write(&dir, "src/app.rs", "fn main() {}\n");
        stage(&dir, &["src/app.rs".into()]).unwrap();
        commit(&dir, "novo arquivo", false).unwrap();
        let risco = push_risk_paths(&dir).unwrap();
        assert_eq!(risco, vec!["src/app.rs".to_string()], "{risco:?}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn a_arvore_e_o_texto_sao_o_que_o_git_guarda() {
        let dir = temp_repo("arvore");
        std::fs::create_dir_all(dir.join("src")).expect("criar pasta src");
        write(&dir, "a.txt", "linha 1\n");
        write(&dir, "src/mod.rs", "pub fn x() {}\n");
        stage(&dir, &["a.txt".into(), "src/mod.rs".into()]).unwrap();
        commit(&dir, "base", false).unwrap();

        // `ls-files` é o índice: os dois entram, com barra de UNIX, e o arquivo
        // que só existe no disco fica fora até ser adicionado.
        write(&dir, "so-no-disco.txt", "ainda não entrou no índice\n");
        let arvore = tracked_files(&dir).unwrap();
        assert!(arvore.contains(&"a.txt".to_string()), "{arvore:?}");
        assert!(arvore.contains(&"src/mod.rs".to_string()), "{arvore:?}");
        assert!(!arvore.contains(&"so-no-disco.txt".to_string()), "{arvore:?}");

        let texto = file_content(&dir, "a.txt").unwrap();
        assert_eq!(texto.text.as_deref(), Some("linha 1\n"));
        assert_eq!(texto.note, None);
        assert!(!texto.truncated);

        // o que muda no disco não troca o conteúdo lido do HEAD
        write(&dir, "a.txt", "linha 1\nlinha 2 na mão\n");
        assert_eq!(file_content(&dir, "a.txt").unwrap().text.as_deref(), Some("linha 1\n"));

        // binário é dito como binário, não como texto vazio
        std::fs::write(dir.join("b.png"), [72u8, 73, 0, 255, 1]).expect("escrever binário");
        stage(&dir, &["b.png".into()]).unwrap();
        commit(&dir, "binário", false).unwrap();
        let bin = file_content(&dir, "b.png").unwrap();
        assert_eq!(bin.text, None);
        assert_eq!(bin.note.as_deref(), Some("arquivo binário — não há texto para mostrar"));

        // caminho que o HEAD não tem devolve a resposta do git, não um vazio
        let erro = file_content(&dir, "nao-existe.txt").unwrap_err();
        assert!(erro.contains("não tem"), "{erro}");

        // sem NUL mas fora de UTF-8: a tela recebe o motivo, não bytes trocados
        std::fs::write(dir.join("latin.txt"), [b'o', b'l', 0xE1, b'\n']).expect("escrever não-UTF-8");
        stage(&dir, &["latin.txt".into()]).unwrap();
        commit(&dir, "texto em outra codificação", false).unwrap();
        let erro = file_content(&dir, "latin.txt").unwrap_err();
        assert!(erro.contains("UTF-8"), "{erro}");

        // arquivo grande chega cortado, e o corte é avisado
        write(&dir, "gigante.txt", &"a".repeat(LIMITE_CONTEUDO + 4_096));
        stage(&dir, &["gigante.txt".into()]).unwrap();
        commit(&dir, "grande", false).unwrap();
        let grande = file_content(&dir, "gigante.txt").unwrap();
        assert!(grande.truncated, "{:?}", grande.note);
        assert!(grande.text.unwrap().contains("conteúdo cortado"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn branches_remotas_e_tags_em_repo_real() {
        let dir = temp_repo("branches");
        write(&dir, "a.txt", "a\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();

        checkout(&dir, "feature", true, None).unwrap();
        write(&dir, "b.txt", "b\n");
        stage(&dir, &["b.txt".into()]).unwrap();
        commit(&dir, "no feature", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        merge(&dir, "feature").unwrap();

        let list = branches(&dir).unwrap();
        let names: Vec<&str> = list.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"main"));
        assert!(names.contains(&"feature"));
        let head = list.iter().find(|b| b.is_head).expect("branch atual");
        assert_eq!(head.name, "main");
        assert_eq!(head.remote, None);

        rename_branch(&dir, "feature", "feature-2").unwrap();
        let names: Vec<String> = branches(&dir).unwrap().into_iter().map(|b| b.name).collect();
        assert!(names.iter().any(|n| n == "feature-2"));
        delete_branch(&dir, "feature-2", true).unwrap();
        let names: Vec<String> = branches(&dir).unwrap().into_iter().map(|b| b.name).collect();
        assert!(!names.iter().any(|n| n == "feature-2"), "branch some da lista");

        create_tag(&dir, "v1.0.0", None, Some("primeira versão")).unwrap();
        create_tag(&dir, "v0.9.0", Some("HEAD"), None).unwrap();
        let lista = read_tags(&dir).unwrap();
        assert_eq!(lista.len(), 2);
        let annotated = lista.iter().find(|t| t.name == "v1.0.0").unwrap();
        assert_eq!(annotated.message.as_deref(), Some("primeira versão"));
        delete_tag(&dir, "v0.9.0").unwrap();
        assert_eq!(read_tags(&dir).unwrap().len(), 1);

        // histórico com todos os refs mostra as tags no gráfico
        let rows = graph(&dir, 10, true).unwrap();
        assert!(rows.iter().any(|r| r.commit.refs.iter().any(|x| x == "v1.0.0")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflitos_e_tentativa_de_push_sem_remoto() {
        let dir = temp_repo("conflito");
        write(&dir, "a.txt", "a\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        checkout(&dir, "outra", true, None).unwrap();
        write(&dir, "a.txt", "outra linha\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "muda na outra", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        write(&dir, "a.txt", "main linha\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "muda no main", false).unwrap();

        let merge_err = merge(&dir, "outra").unwrap_err();
        let status = read_status(&dir).unwrap();
        assert!(status.conflict_count >= 1, "conflito detectado: {merge_err}");
        let conflicted = status.files.iter().find(|f| f.conflict).unwrap();
        resolve_conflict(&dir, &conflicted.path, "ours").unwrap();
        let status = read_status(&dir).unwrap();
        assert_eq!(status.conflict_count, 0);
        // `merge --no-edit` com conflito fica em andamento até concluir.
        let _ = run(&dir, &["commit", "--no-edit", "-m", "merge resolvido"]);

        // push sem remoto configurado falha com mensagem útil
        let err = push(&dir, None, None, false).unwrap_err();
        assert!(!err.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reversão real, sem nenhum servidor envolvido: `revert` grava o commit
    /// inverso e um merge só é revertido com o pai escolhido.
    #[test]
    fn revert_cria_commit_inverso_e_exige_pai_no_merge() {
        let dir = temp_repo("reverter");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "primeiro", false).unwrap();

        write(&dir, "a.txt", "dois\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        let segundo = commit(&dir, "segundo", false).unwrap();

        revert(&dir, &segundo, None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "um\n");
        let linhas = graph(&dir, 10, false).unwrap();
        assert!(
            linhas[0].commit.subject.contains("segundo"),
            "o topo do histórico deveria ser a reversão de segundo: {:?}",
            linhas[0].commit.subject
        );
        assert_eq!(parent_count(&dir, &linhas[0].commit.id), 1);
        assert!(
            linhas.iter().any(|r| r.commit.subject == "segundo"),
            "revert não apaga o histórico"
        );

        // merge: dois pais, então a reversão precisa dizer qual deles continua.
        checkout(&dir, "paralelo", true, None).unwrap();
        write(&dir, "c.txt", "c\n");
        stage(&dir, &["c.txt".into()]).unwrap();
        commit(&dir, "no paralelo", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        write(&dir, "d.txt", "d\n");
        stage(&dir, &["d.txt".into()]).unwrap();
        commit(&dir, "no main", false).unwrap();
        merge(&dir, "paralelo").unwrap();

        let cabeca = run(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        assert_eq!(parent_count(&dir, &cabeca), 2, "o merge deve ter dois pais");
        let erro = revert(&dir, &cabeca, None).unwrap_err();
        assert!(erro.contains("merge com 2 pais"), "mensagem útil: {erro}");
        assert!(revert(&dir, &cabeca, Some(3)).is_err(), "pai inexistente é recusado");
        revert(&dir, &cabeca, Some(1)).unwrap();
        assert!(!dir.join("c.txt").exists(), "revert do merge tira o que o paralelo trouxe");
        assert!(dir.join("d.txt").exists(), "o trabalho do ramo principal permanece");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Guardar trabalho é a única forma de trocar de ramo sem perder o que está
    /// na mão. O teste confere no git real: o que sai da pasta, o que a lista
    /// mostra, o que volta, e que sem nada alterado nada é guardado.
    #[test]
    fn guardar_e_trazer_o_trabalho_em_andamento() {
        let dir = temp_repo("guardar");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();

        assert!(stash_list(&dir).unwrap().is_empty(), "repositório novo não tem guardado");

        write(&dir, "a.txt", "um\ndois\n");
        write(&dir, "novo.txt", "ainda não entra no git\n");
        stash_push(&dir, Some("tentativa do a"), false).unwrap();

        // o modificado voltou ao commit; o não rastreado ficou na pasta, porque
        // sem `include_untracked` é isso que o git faz.
        assert!(read_status(&dir).unwrap().files.iter().all(|f| f.path != "a.txt"));
        assert!(dir.join("novo.txt").exists());

        let lista = stash_list(&dir).unwrap();
        assert_eq!(lista.len(), 1, "{lista:?}");
        assert_eq!(lista[0].index, 0);
        assert_eq!(lista[0].selector, "stash@{0}");
        assert!(lista[0].subject.contains("tentativa do a"), "{:?}", lista[0].subject);
        assert!(!lista[0].sha.is_empty());

        // nada alterado: o git avisa no stdout com código 0, e nada entra na lista.
        let aviso = stash_push(&dir, None, false).unwrap();
        assert!(aviso.contains("nada para guardar"), "{aviso}");
        assert_eq!(stash_list(&dir).unwrap().len(), 1, "nada novo foi guardado");

        // apply copia para a pasta e mantém a entrada.
        let aplicado = stash_apply(&dir, 0, false).unwrap();
        assert!(aplicado.contains("continua na lista"), "{aplicado}");
        assert!(blob_at(&dir, "stash@{0}", "a.txt").is_some(), "o guardado tem o arquivo");

        // para o `pop` valer, a pasta precisa estar limpa de novo.
        reset(&dir, "hard", "HEAD").unwrap();
        let trazido = stash_apply(&dir, 0, true).unwrap();
        assert!(trazido.contains("saiu da lista"), "{trazido}");
        assert!(stash_list(&dir).unwrap().is_empty());

        stash_push(&dir, Some("segunda"), true).unwrap();
        assert!(!dir.join("novo.txt").exists(), "com -u o não rastreado também sai da pasta");
        // índice que a lista não tem é recusado antes de falar com o git.
        let erro = stash_apply(&dir, 4, false).unwrap_err();
        assert!(erro.contains("não existe mais"), "{erro}");
        stash_drop(&dir, 0).unwrap();
        assert!(stash_list(&dir).unwrap().is_empty());
        let erro = stash_drop(&dir, 0).unwrap_err();
        assert!(erro.contains("não há trabalho guardado"), "{erro}");

        // com conflito na pasta, guardar é recusado em vez de embolar o git.
        write(&dir, "a.txt", "um\nconflito aqui\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "mexeu no a", false).unwrap();
        checkout(&dir, "paralelo", true, None).unwrap();
        write(&dir, "a.txt", "do paralelo\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "muda no paralelo", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        write(&dir, "a.txt", "um\noutro meio\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "muda no main", false).unwrap();
        let _ = merge(&dir, "paralelo");
        let meio = read_status(&dir).unwrap();
        assert!(meio.conflict_count > 0, "o merge deveria estar em conflito");
        assert_eq!(meio.operation.as_deref(), Some("merge"), "{:?}", meio.operation);
        let erro = stash_push(&dir, Some("no meio do conflito"), false).unwrap_err();
        assert!(erro.contains("em conflito"), "{erro}");
        abort_operation(&dir, "merge").unwrap();
        assert_eq!(read_status(&dir).unwrap().operation, None, "cancelou, acabou");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reproduzir um commit de outro ramo grava um commit novo aqui e, quando o
    /// git trava no meio, a tela sabe qual operação está parada — merge, rebase,
    /// cherry-pick e revert deixam marcas diferentes no `.git`.
    #[test]
    fn reproduzir_commit_e_a_operacao_que_ficou_parada() {
        let dir = temp_repo("reproduzir");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        assert_eq!(read_status(&dir).unwrap().operation, None);

        checkout(&dir, "feature", true, None).unwrap();
        write(&dir, "b.txt", "do feature\n");
        stage(&dir, &["b.txt".into()]).unwrap();
        let limpo = commit(&dir, "traz o b", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();

        cherry_pick(&dir, &limpo, None).unwrap();
        assert!(tracked_files(&dir).unwrap().contains(&"b.txt".to_string()));
        assert_eq!(
            String::from_utf8_lossy(&blob_at(&dir, "HEAD", "b.txt").unwrap()),
            "do feature\n"
        );
        // o ramo de origem não foi mexido: a cabeça dele continua a dele.
        checkout(&dir, "feature", false, None).unwrap();
        assert_eq!(run(&dir, &["rev-parse", "HEAD"]).unwrap().trim(), limpo);

        // mesmo arquivo dos dois lados: o git para no meio e diz o que é.
        checkout(&dir, "feature", false, None).unwrap();
        write(&dir, "b.txt", "do feature, mudado\n");
        stage(&dir, &["b.txt".into()]).unwrap();
        let conflitante = commit(&dir, "mexe no b", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        write(&dir, "b.txt", "o que o main tem\n");
        stage(&dir, &["b.txt".into()]).unwrap();
        commit(&dir, "divergiu", false).unwrap();

        let erro = cherry_pick(&dir, &conflitante, None).unwrap_err();
        assert!(erro.contains("conflitos"), "{erro}");
        let meio = read_status(&dir).unwrap();
        assert!(meio.conflict_count > 0);
        assert_eq!(meio.operation.as_deref(), Some("cherry-pick"), "{meio:?}");

        // cancelar devolve a pasta ao antes da operação.
        abort_operation(&dir, "cherry-pick").unwrap();
        let depois = read_status(&dir).unwrap();
        assert_eq!(depois.operation, None);
        assert_eq!(depois.conflict_count, 0);
        assert_eq!(
            String::from_utf8_lossy(&blob_at(&dir, "HEAD", "b.txt").unwrap()),
            "o que o main tem\n"
        );

        // resolver com o lado do commit reproduzido e continuar fecha a operação.
        let erro = cherry_pick(&dir, &conflitante, None).unwrap_err();
        assert!(erro.contains("conflitos"), "{erro}");
        let em_conflito = read_status(&dir)
            .unwrap()
            .files
            .into_iter()
            .find(|f| f.conflict)
            .expect("o cherry-pick deixou conflito");
        resolve_conflict(&dir, &em_conflito.path, "theirs").unwrap();
        continue_operation(&dir, "cherry-pick").unwrap();
        let fechado = read_status(&dir).unwrap();
        assert_eq!(fechado.operation, None, "continuar conclui o passo");
        assert_eq!(
            String::from_utf8_lossy(&blob_at(&dir, "HEAD", "b.txt").unwrap()),
            "do feature, mudado\n"
        );

        // um commit já aplicado não sobra: o git avisa e o passo pode ser pulado.
        let repetido = cherry_pick(&dir, &conflitante, None);
        assert!(repetido.is_err(), "não há o que aplicar de novo");
        assert_eq!(read_status(&dir).unwrap().operation.as_deref(), Some("cherry-pick"));
        skip_operation(&dir, "cherry-pick").unwrap();
        assert_eq!(read_status(&dir).unwrap().operation, None);

        // commit de merge só passa com o pai escolhido, igual ao revert.
        checkout(&dir, "para-merge", true, None).unwrap();
        write(&dir, "c.txt", "c\n");
        stage(&dir, &["c.txt".into()]).unwrap();
        commit(&dir, "no para-merge", false).unwrap();
        checkout(&dir, "main", false, None).unwrap();
        write(&dir, "d.txt", "d\n");
        stage(&dir, &["d.txt".into()]).unwrap();
        commit(&dir, "no main", false).unwrap();
        merge(&dir, "para-merge").unwrap();
        let cabeca = run(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        assert!(cherry_pick(&dir, &cabeca, None).unwrap_err().contains("merge com 2 pais"));
        assert!(cherry_pick(&dir, &cabeca, Some(7)).unwrap_err().contains("pai inválido"));

        // operação que não existe não é mandada ao git.
        assert!(continue_operation(&dir, "inventada").is_err());
        assert!(abort_operation(&dir, "inventada").is_err());
        assert!(
            continue_operation(&dir, "merge").is_err(),
            "merge fecha com commit, não com --continue"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `parse_status` não conhece a operação em andamento — quem sabe são os
    /// arquivos do `.git`. Este teste fixa o nome que a interface usa para cada
    /// marca, sem depender de um git em conflito para isso.
    #[test]
    fn as_marcas_de_operacao_do_git_viram_nome() {
        let dir = temp_repo("marcas");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "base", false).unwrap();
        let git = dir.join(".git");
        assert_eq!(operation_in_progress(&dir), None);
        for (marca, nome) in [
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ] {
            std::fs::write(git.join(marca), "deadbeef\n").expect("escrever a marca");
            assert_eq!(operation_in_progress(&dir).as_deref(), Some(nome), "{marca}");
            std::fs::remove_file(git.join(marca)).unwrap();
        }
        std::fs::create_dir_all(git.join("rebase-merge")).unwrap();
        assert_eq!(operation_in_progress(&dir).as_deref(), Some("rebase"));
        std::fs::remove_dir_all(git.join("rebase-merge")).unwrap();
        assert_eq!(operation_in_progress(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Os três modos de `reset` têm efeitos distintos e isto é verificado no
    /// sistema de arquivos de verdade — inclusive que `hard` descarta trabalho.
    #[test]
    fn reset_soft_mixed_e_hard_mudem_cada_um_do_seu_modo() {
        let dir = temp_repo("resetar");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        let primeiro = commit(&dir, "primeiro", false).unwrap();
        write(&dir, "a.txt", "dois\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "segundo", false).unwrap();

        write(&dir, "b.txt", "novo\n");
        stage(&dir, &["b.txt".into()]).unwrap();

        let saida = reset(&dir, "soft", &primeiro).unwrap();
        assert!(saida.contains("cabeça agora em"), "a saída diz onde a cabeça ficou: {saida}");
        let status = read_status(&dir).unwrap();
        assert_eq!(status.files.iter().filter(|f| f.path == "b.txt" && f.staged).count(), 1,
            "soft mantém o índice");

        reset(&dir, "mixed", &primeiro).unwrap();
        let status = read_status(&dir).unwrap();
        assert!(
            status.files.iter().any(|f| f.path == "b.txt" && !f.staged),
            "mixed solta o índice mas mantém o arquivo em trabalho"
        );
        assert!(dir.join("b.txt").exists(), "mixed não apaga nada do disco");

        write(&dir, "a.txt", "sujo\n");
        reset(&dir, "hard", &primeiro).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "um\n",
            "hard descarta o conteúdo em trabalho");
        assert!(dir.join("b.txt").exists(), "hard não apaga arquivo que o git não rastreia");

        assert!(reset(&dir, "esquisito", &primeiro).is_err(), "modo fora da lista é recusado");
        assert!(reset(&dir, "soft", "-hard").is_err(), "referência começando com '-' é recusada");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Nome explícito do que a verificação acima também cobre: `reset --soft`
    /// move só a cabeça — o índice continua preparado, arquivo por arquivo.
    #[test]
    fn reset_soft_preserva_o_indice() {
        let dir = temp_repo("reset-soft");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        let primeiro = commit(&dir, "primeiro", false).unwrap();
        write(&dir, "a.txt", "dois\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        let segundo = commit(&dir, "segundo", false).unwrap();
        write(&dir, "b.txt", "novo\n");
        stage(&dir, &["b.txt".into()]).unwrap();

        reset(&dir, "soft", &primeiro).unwrap();

        // a cabeça voltou, mas o índice é exatamente o do commit "segundo" + b.txt
        assert_eq!(run(&dir, &["rev-parse", "HEAD"]).unwrap().trim(), primeiro);
        assert_eq!(
            staged_files(&dir).unwrap(),
            vec!["a.txt".to_string(), "b.txt".to_string()],
            "o índice sobrevive ao reset --soft"
        );
        assert_ne!(segundo, primeiro, "são commits diferentes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remotes_e_fetch_em_clone_local() {
        let origem = temp_repo("origem");
        write(&origem, "a.txt", "a\n");
        stage(&origem, &["a.txt".into()]).unwrap();
        commit(&origem, "primeiro", false).unwrap();

        let pais = temp_dir("clonede");
        let destino = pais.join("work");
        clone(&pais, &url(&origem), Some("work"), None).unwrap();
        assert!(is_repo(&destino), "o clone produziu um repositório");
        let rows = graph(&destino, 10, false).unwrap();
        assert_eq!(rows[0].commit.subject, "primeiro");

        let remotes = read_remotes(&destino).unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].name, "origin");
        assert!(remotes[0].fetch_url.is_some());

        add_remote(&destino, "espelho", &url(&origem)).unwrap();
        assert_eq!(read_remotes(&destino).unwrap().len(), 2);
        set_remote_url(&destino, "espelho", &url(&origem)).unwrap();
        remove_remote(&destino, "espelho").unwrap();
        assert_eq!(read_remotes(&destino).unwrap().len(), 1);

        fetch(&destino, None).unwrap();
        // clone já traz o branch com upstream: pull fast-forward funciona
        let pull_msg = pull(&destino, "ff-only").unwrap();
        assert!(pull_msg.contains("Already up to date") || pull_msg.contains("atualizado"));
        assert!(pull(&destino, "esquisita").is_err());

        let _ = std::fs::remove_dir_all(&pais);
        let _ = std::fs::remove_dir_all(&origem);
    }

    #[test]
    fn buscar_um_pull_request_inexistente_nao_cria_branch_fantasma() {
        let origem = temp_repo("pr-origem");
        write(&origem, "a.txt", "a\n");
        stage(&origem, &["a.txt".into()]).unwrap();
        commit(&origem, "primeiro", false).unwrap();
        let pais = temp_dir("prclonede");
        let destino = pais.join("work");
        clone(&pais, &url(&origem), Some("work"), None).unwrap();

        // Um repositório local não publica refs/pull/*: a falha é devolvida como
        // falha, e nenhuma branch de leitura é criada.
        let erro = checkout_pull(&destino, "origin", 7).unwrap_err();
        assert!(erro.contains("pull request #7"), "mensagem: {erro}");
        assert!(
            run(&destino, &["show-ref", "--verify", "refs/heads/github/pr-7"]).is_err(),
            "criou a branch mesmo com o fetch tendo falhado"
        );
        assert!(checkout_pull(&destino, "origin", 0).is_err());
        assert!(checkout_pull(&destino, "origin com espaço", 1).is_err());

        let _ = std::fs::remove_dir_all(&pais);
        let _ = std::fs::remove_dir_all(&origem);
    }

    #[test]
    fn publicar_empurra_ramo_e_tag_e_despublicar_retira_so_a_tag() {
        let origem = temp_repo("pub-origem");
        write(&origem, "a.txt", "a\n");
        stage(&origem, &["a.txt".into()]).unwrap();
        commit(&origem, "primeiro", false).unwrap();
        let remoto = bare_origin(&origem);

        let pais = temp_dir("pub-clone");
        let destino = pais.join("work");
        clone(&pais, &url(&remoto), Some("work"), None).unwrap();
        assert_eq!(default_remote(&destino).unwrap(), "origin", "o clone traz origin");

        write(&destino, "b.txt", "b\n");
        stage(&destino, &["b.txt".into()]).unwrap();
        commit(&destino, "segundo", false).unwrap();
        push(&destino, Some("origin"), None, false).unwrap();
        create_tag(&destino, "v1", None, None).unwrap();
        push_tag(&destino, "origin", "v1").unwrap();

        assert!(
            tags(&remoto).unwrap().iter().any(|t| t.name == "v1"),
            "push_tag leva a tag até o remoto"
        );
        assert_eq!(
            run(&remoto, &["rev-parse", "main"]).unwrap().trim(),
            run(&destino, &["rev-parse", "HEAD"]).unwrap().trim(),
            "o ramo publicado está no mesmo commit"
        );

        delete_remote_tag(&destino, "origin", "v1").unwrap();
        assert!(
            !tags(&remoto).unwrap().iter().any(|t| t.name == "v1"),
            "despublicar tira a tag do remoto"
        );
        assert!(
            tags(&destino).unwrap().iter().any(|t| t.name == "v1"),
            "a tag local continua — nada foi apagado daqui"
        );
        assert_eq!(
            run(&remoto, &["rev-parse", "main"]).unwrap().trim(),
            run(&destino, &["rev-parse", "HEAD"]).unwrap().trim(),
            "despublicar a tag não mexe no ramo do remoto"
        );

        assert!(push_tag(&destino, "-x", "v1").is_err(), "remoto começando com '-' é recusado");
        assert!(delete_remote_tag(&destino, "origin", "..").is_err(), "nome com .. é recusado");

        let _ = std::fs::remove_dir_all(&pais);
        let _ = std::fs::remove_dir_all(&origem);
        let _ = std::fs::remove_dir_all(&remoto);
    }

    #[test]
    fn upstream_ahead_behind_sao_lidos() {
        let origem = temp_repo("upstream-origem");
        write(&origem, "a.txt", "a\n");
        stage(&origem, &["a.txt".into()]).unwrap();
        commit(&origem, "c1", false).unwrap();
        let remoto = bare_origin(&origem);

        let pais = temp_dir("upstream-clone");
        let destino = pais.join("work");
        clone(&pais, &url(&remoto), Some("work"), None).unwrap();
        let status = read_status(&destino).unwrap();
        assert!(status.upstream.is_some(), "clone define upstream: {:?}", status.upstream);
        assert_eq!((status.ahead, status.behind), (0, 0));

        write(&destino, "b.txt", "b\n");
        stage(&destino, &["b.txt".into()]).unwrap();
        commit(&destino, "c2 local", false).unwrap();
        let status = read_status(&destino).unwrap();
        assert_eq!((status.ahead, status.behind), (1, 0), "commit local fica à frente");

        push(&destino, None, None, false).unwrap();
        let status = read_status(&destino).unwrap();
        assert_eq!((status.ahead, status.behind), (0, 0), "push alinha com o remoto");
        let id_local = run(&destino, &["rev-parse", "HEAD"]).unwrap();
        let id_remoto = run(&remoto, &["rev-parse", "main"]).unwrap();
        assert_eq!(id_local.trim(), id_remoto.trim(), "o remoto recebeu o commit");

        // um segundo clone avança o remoto e o primeiro fica atrás
        let pais2 = temp_dir("upstream-outro");
        let outro = pais2.join("work");
        clone(&pais2, &url(&remoto), Some("work"), None).unwrap();
        write(&outro, "c.txt", "c\n");
        stage(&outro, &["c.txt".into()]).unwrap();
        commit(&outro, "c3 de outro clone", false).unwrap();
        push(&outro, None, None, false).unwrap();

        fetch(&destino, None).unwrap();
        let status = read_status(&destino).unwrap();
        assert_eq!((status.ahead, status.behind), (0, 1), "fetch revela o commit novo");

        let msg = pull(&destino, "ff-only").unwrap();
        assert!(msg.contains("Fast-forward") || msg.contains("c.txt"), "pull: {msg}");
        let status = read_status(&destino).unwrap();
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert!(destino.join("c.txt").exists(), "o fast-forward trouxe o arquivo");

        let _ = std::fs::remove_dir_all(&pais);
        let _ = std::fs::remove_dir_all(&pais2);
        let _ = std::fs::remove_dir_all(&remoto);
        let _ = std::fs::remove_dir_all(&origem);
    }

    #[test]
    fn status_v2_parseia_tipos_e_espacos() {
        // registros reais de `git status --porcelain=v2 -z`
        let raw = concat!(
            "1 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 \
             8ba3a16384aacc37d01564b28401755ce8053f51 novo.txt\0",
            "? sp ace.txt\0",
            "1 .M N... 100644 100644 100644 de980441c3ab03a8c07dda1ad27b8a11f39deb1e \
             de980441c3ab03a8c07dda1ad27b8a11f39deb1e a.txt\0",
            "2 R. N... 100644 100644 100644 587be6b4c3f93f93c489c0111bba5596147a26cb \
             587be6b4c3f93f93c489c0111bba5596147a26cb R100 moved.txt\0",
            "kept.txt\0",
            "u UU N... 100644 100644 100644 100644 7be73ce3c1b1cdaea86e8168dfee8575175953bf \
             ba2906d0666cf726c7eaadd2cd3db615dedfdf3a 48156c44efe8acc0adf403a81acec89514d942c7 a.txt\0",
            "1 T. T... 100644 120000 100644 0000000000000000000000000000000000000000 \
             0000000000000000000000000000000000000000 link\0",
            "! ignorado.txt\0",
        );
        let files = parse_status(raw);
        assert_eq!(files.len(), 6, "entradas `!` são ignoradas");
        let espaco = files.iter().find(|f| f.path == "sp ace.txt").unwrap();
        assert!(espaco.untracked);
        assert!(espaco.kind == "não rastreado", "kind: {}", espaco.kind);
        let novo = files.iter().find(|f| f.path == "novo.txt").unwrap();
        assert!(novo.staged && novo.kind == "adicionado");
        let modificado = files.iter().find(|f| f.path == "a.txt" && !f.conflict).unwrap();
        assert!(!modificado.staged && modificado.unstaged);
        let ren = files.iter().find(|f| f.path == "moved.txt").unwrap();
        assert_eq!(ren.old_path.as_deref(), Some("kept.txt"));
        assert_eq!(ren.kind, "renomeado");
        let conf = files.iter().find(|f| f.conflict).unwrap();
        assert_eq!(conf.path, "a.txt");
        assert_eq!(conf.kind, "conflito");
        let tipo = files.iter().find(|f| f.path == "link").unwrap();
        assert_eq!(tipo.kind, "tipo alterado");
    }

    #[test]
    fn parse_de_hunks_rename_e_binario() {
        let text = concat!(
            "diff --git a/old.txt b/new.txt\n",
            "similarity index 90%\n",
            "rename from old.txt\n",
            "rename to new.txt\n",
            "index 1234567..89abcde 100644\n",
            "--- a/old.txt\n",
            "+++ b/new.txt\n",
            "@@ -1,3 +1,3 @@\n",
            " contexto\n",
            "-removida\n",
            "+adicionada\n",
            " fim\n",
            "diff --git a/img.png b/img.png\n",
            "index 1111111..2222222 100644\n",
            "Binary files a/img.png and b/img.png differ\n",
        );
        let files = parse_diff(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status_char, 'R');
        assert_eq!(files[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].hunks[0].lines.len(), 4);
        assert!(files[1].binary);
        assert!(files[1].hunks.is_empty());
    }

    #[test]
    fn grafo_de_merge_e_parseado() {
        let com_merge = concat!(
            "* commit-a\x1fa\x1fDouglas\x1fd@x\x1f1700000000\x1fHEAD -> main\x1fmensagem A\x1fparente-x parente-y\n",
            "*   commit-b\x1fb\x1fDouglas\x1fd@x\x1f1700000001\x1forg/x\x1fmerge\x1fp1 p2\n",
            "|\\  \n",
            "| * commit-c\x1fc\x1fOutro\x1fo@x\x1f1700000002\x1f\x1flinha\x1f\n",
            "|/  \n",
            "* commit-d\x1fd\x1fDouglas\x1fd@x\x1f1700000003\x1ftag: v1\x1foutro\x1f\n"
        );
        let rows = parse_graph(com_merge);
        assert_eq!(rows.len(), 4, "linhas só de trilha não viram commits");
        assert_eq!(rows[1].commit.parents.len(), 2);
        assert_eq!(rows[2].graph, "| * ");
        assert_eq!(rows[2].commit.refs, Vec::<String>::new());
        assert_eq!(rows[3].commit.refs, vec!["v1".to_string()]);
    }

    #[test]
    fn name_status_e_numstat_do_commit() {
        let files = parse_name_status("A\tnovo.txt\nM\ta.txt\nR100\tvelho.txt\tnovo.txt\nT\tlink\n");
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].kind, "adicionado");
        assert_eq!(files[0].path, "novo.txt");
        assert_eq!(files[1].kind, "modificado");
        // em rename o caminho exibido é o de destino; a origem fica em old_path
        assert_eq!(files[2].path, "novo.txt");
        assert_eq!(files[2].old_path.as_deref(), Some("velho.txt"));
        assert_eq!(files[2].kind, "renomeado");
        assert_eq!(files[3].kind, "tipo alterado");
        assert_eq!(files[3].path, "link");
        let (add, del) = sum_numstat("3\t1\ta.txt\n-\t-\timg.png\n");
        assert_eq!((add, del), (3, 1));
    }

    #[test]
    fn diff_de_commit_usa_o_pai_e_a_arvore_vazia() {
        let dir = temp_repo("diffcommit");
        write(&dir, "a.txt", "um\n");
        stage(&dir, &["a.txt".into()]).unwrap();
        commit(&dir, "primeiro", false).unwrap();

        // primeiro commit: não tem pai, então a base é a árvore vazia
        let root = run(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        assert!(commit_parent(&dir, &root).is_none());
        let raiz = diff(&dir, None, false, Some(&root)).unwrap();
        assert_eq!(raiz.len(), 1);
        assert_eq!(raiz[0].path, "a.txt");
        assert_eq!(raiz[0].status_char, 'A');

        write(&dir, "a.txt", "um\ndois\n");
        write(&dir, "b.txt", "novo\n");
        stage(&dir, &["a.txt".into(), "b.txt".into()]).unwrap();
        commit(&dir, "segundo", false).unwrap();
        let head = run(&dir, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        assert_eq!(commit_parent(&dir, &head).as_deref(), Some(root.as_str()));

        let depois = diff(&dir, None, false, Some(&head)).unwrap();
        let nomes: Vec<&str> = depois.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(nomes, ["a.txt", "b.txt"]);
        let modificado = depois.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(modificado.status_char, 'M');
        assert!(
            modificado
                .hunks
                .iter()
                .any(|h| h.lines.iter().any(|l| l.kind == "add" && l.text.contains("dois"))),
            "diff do commit: {:?}",
            modificado.hunks
        );
        // um caminho dentro de um commit continua filtrado
        let so_b = diff(&dir, Some("b.txt"), false, Some(&head)).unwrap();
        assert_eq!(so_b.len(), 1);
        assert_eq!(so_b[0].path, "b.txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validacoes_de_referencia_e_url() {
        assert!(validate_id("main").is_ok());
        assert!(validate_id("-b").is_err());
        assert!(validate_id("a b").is_err());
        assert!(validate_id("a..b").is_err());
        assert!(validate_url("https://github.com/a/b.git").is_ok());
        assert!(validate_url("git@github.com:a/b.git").is_ok());
        assert!(validate_url("/tmp/repo").is_ok());
        assert!(validate_url("C:/repos/app").is_ok());
        assert!(validate_url("--upload-pack=mal").is_err());
        assert!(validate_url("repositorio-relativo").is_err());
    }

    #[test]
    fn track_ahead_behind_parseado() {
        assert_eq!(parse_track("[ahead 2, behind 1]"), (2, 1));
        assert_eq!(parse_track("[behind 3]"), (0, 3));
        assert_eq!(parse_track(""), (0, 0));
    }

    #[test]
    fn erros_conhecidos_tem_mensagem_util() {
        assert!(describe_failure(&["commit"], b"nothing to commit, working tree clean", b"").contains("nada para fazer"));
        // `git commit` sem staging falha com o texto no STDOUT
        assert!(
            describe_failure(
                &["commit"],
                b"",
                b"nothing added to commit but untracked files present (use \"git add\" to track)"
            )
            .contains("nada para fazer")
        );
        assert!(describe_failure(&["push"], b"! [rejected] main -> main (non-fast-forward)", b"").contains("histórico"));
        assert!(
            describe_failure(
                &["pull"],
                b"fatal: could not read Username for 'https://github.com': terminal prompts disabled",
                b""
            )
            .contains("autenticação")
        );
        assert!(describe_failure(&["log"], b"fatal: not a git repository", b"").contains("não é um repositório"));
        assert!(describe_failure(&["config"], b"fatal: please tell me who you are", b"").contains("user.name"));
        // sem tradução conhecida, o texto do git volta sem ser cortado
        assert!(describe_failure(&["apply"], b"error: patch failed: a.txt:1", b"").contains("patch failed"));
    }

    #[test]
    fn repositorio_inexistente_da_erro() {
        let dir = temp_dir("nao-repo");
        let err = read_status(&dir).unwrap_err();
        assert!(err.contains("não é um repositório"), "mensagem: {err}");
        assert!(!is_repo(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ExecResult fabricado só para exercitar a tradução do veredito, sem
    /// depender de rede.
    fn feito(exit: Option<i32>, stdout: &str, stderr: &str, timed_out: bool) -> crate::sshexec::ExecResult {
        crate::sshexec::ExecResult {
            exit_code: exit,
            stdout: stdout.into(),
            stderr: stderr.into(),
            timed_out,
            cancelled: false,
        }
    }

    #[test]
    fn veredito_de_acesso_libera_o_que_responde_e_recusa_o_que_nao_pode() {
        // remoto alcançável com commits
        let vivo = describe_access("origin", &feito(Some(0), "abc1234567890\tHEAD\n", "", false));
        assert!(vivo.ok);
        assert!(vivo.detail.contains("abc1234"), "mostra o HEAD curto: {}", vivo.detail);

        // remoto recém-criado: vazio é sucesso, não falha
        let vazio = describe_access("origin", &feito(Some(0), "", "", false));
        assert!(vazio.ok, "remoto sem commits ainda é acesso liberado");
        assert!(vazio.detail.contains("vazio"));

        // chave não aceita
        let negado = describe_access(
            "origin",
            &feito(Some(128), "", "git@github.com: Permission denied (publickey).", false),
        );
        assert!(!negado.ok);
        assert!(negado.detail.contains("sem liberação"), "mensagem: {}", negado.detail);

        // credencial pedida na hora (jamais acontece: o prompt está desativado)
        let sem_credencial = describe_access(
            "origin",
            &feito(
                Some(128),
                "",
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
                false,
            ),
        );
        assert!(!sem_credencial.ok);
        assert!(sem_credencial.detail.contains("sem liberação"));

        // endereço errado
        let endereco = describe_access(
            "origin",
            &feito(Some(128), "", "fatal: 'x' does not appear to be a git repository", false),
        );
        assert!(!endereco.ok);
        assert!(endereco.detail.contains("endereço"), "mensagem: {}", endereco.detail);

        // rede
        let rede = describe_access(
            "origin",
            &feito(Some(128), "", "ssh: connect to host github.com port 22: Connection timed out", false),
        );
        assert!(!rede.ok);
        assert!(rede.detail.contains("rede"), "mensagem: {}", rede.detail);

        // prazo estourado: a tela nunca fica pendurada
        let lento = describe_access("origin", &feito(None, "", "", true));
        assert!(!lento.ok);
        assert!(lento.detail.contains("não respondeu"));

        // sem tradução conhecida, o texto do git volta como detalhe
        let outro = describe_access("origin", &feito(Some(128), "", "error: RPC failed; HTTP 500", false));
        assert!(!outro.ok);
        assert!(outro.detail.contains("RPC failed"), "mensagem: {}", outro.detail);
    }

    #[test]
    fn detalhe_grande_e_cortado_e_a_contagem_nao_estoura() {
        assert_eq!(reduzido(""), "(sem detalhe)");
        let uma = reduzido("linha única\n");
        assert_eq!(uma, "linha única");
        let tres = reduzido("a\nb\nc\nd\n");
        assert!(tres.starts_with("… b c d"), "as três últimas: {tres}");
        let longo = reduzido(&"x".repeat(900));
        assert_eq!(longo.chars().count(), 401, "corte em 400 + reticências");
    }

    #[test]
    fn acesso_real_do_clone_ate_o_remoto_bare() {
        let origem = temp_repo("acesso-origem");
        write(&origem, "a.txt", "a\n");
        stage(&origem, &["a.txt".into()]).unwrap();
        commit(&origem, "primeiro", false).unwrap();
        let remoto = bare_origin(&origem);

        let pais = temp_dir("acesso-clone");
        let destino = pais.join("work");
        clone(&pais, &url(&remoto), Some("work"), None).unwrap();

        let ok = remote_access(&destino, "origin").expect("ls-remote roda");
        assert!(ok.ok, "o clone consegue falar com o bare: {}", ok.detail);
        assert_eq!(ok.remote, "origin");

        // um caminho que não é repositório: veredito negativo, sem exceção
        let morto = temp_repo("acesso-morto");
        add_remote(&morto, "origin", &url(&morto.join("nao-existe.git"))).unwrap();
        let ruim = remote_access(&morto, "origin").expect("o teste é feito mesmo com remoto ruim");
        assert!(!ruim.ok, "remoto inexistente não é liberação");
        assert!(ruim.detail.contains("origin"));

        // nome inválido nem chega a executar o git
        assert!(remote_access(&destino, "-x").is_err(), "remoto começando com '-' é recusado");
        assert!(remote_access(&destino, "../outro").is_err(), "referência com .. é recusada");

        let _ = std::fs::remove_dir_all(&pais);
        let _ = std::fs::remove_dir_all(&origem);
        let _ = std::fs::remove_dir_all(&remoto);
    }

    #[test]
    fn head_sha_e_o_commit_atual_e_none_onde_nao_ha_repo() {
        let dir = temp_repo("head-sha");
        assert_eq!(head_sha(&dir), None, "repositório recém-iniciado não tem HEAD");
        write(&dir, "a.txt", &base_file());
        stage(&dir, &["a.txt".into()]).unwrap();
        run(&dir, &["commit", "-q", "-m", "primeiro"]).unwrap();
        let sha = head_sha(&dir).expect("depois do commit o HEAD existe");
        assert_eq!(sha.len(), 40, "sha curto demais para ser um commit: {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "{sha}");

        let solto = temp_dir("head-sha-sem-repo");
        assert_eq!(head_sha(&solto), None, "pasta que não é repositório não tem commit");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&solto);
    }
}

