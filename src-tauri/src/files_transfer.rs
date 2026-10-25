//! Transferências sftp com progresso em tempo real, cancelamento e
//! retomada quando o cliente a suporta.
//!
//! O medidor de progresso é lido da própria saída do cliente `sftp` (modo
//! interativo alimentado por script): linhas como
//! `arquivo.txt 45% 1.2MB 345KB/s 00:12`. Se o cliente não imprimir o
//! medidor, a transferência ainda conclui e emite o evento final.

use std::{
    collections::HashMap,
    io::{Read, Write},
    process::{Child, ExitStatus},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::auth::AskPass;
use crate::files::{sftp_cmd, sftp_quote};
use crate::models::Server;

pub(crate) type DbHandle = crate::db::DbHandle;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub transfer_id: i64,
    pub file_name: String,
    pub pct: f64,
    pub transferred: u64,
    pub total: Option<u64>,
    pub rate: Option<u64>,
    pub eta: Option<u64>,
    pub done: bool,
    pub ok: bool,
    pub error: Option<String>,
}

pub(crate) struct Handle {
    pub kind: &'static str,
    pub remote_path: String,
    pub local_path: String,
    pub server_name: String,
    pub cancel: Arc<AtomicBool>,
    pub child: Arc<Mutex<Option<Child>>>,
    pub started_at: Instant,
}

pub struct TransferState {
    pub(crate) handles: Arc<Mutex<HashMap<i64, Handle>>>,
    next_id: AtomicI64,
}

impl TransferState {
    pub fn new() -> Self {
        Self {
            handles: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicI64::new(1),
        }
    }
}

pub fn parse_human_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let t = t.trim_end_matches("/s").trim_end_matches("/S");
    let upper = t.to_uppercase();
    let (num_str, mul): (&str, f64) = if let Some(s) = upper.strip_suffix("KB") {
        (&t[..s.len()], 1_024.0)
    } else if let Some(s) = upper.strip_suffix("MB") {
        (&t[..s.len()], 1_048_576.0)
    } else if let Some(s) = upper.strip_suffix("GB") {
        (&t[..s.len()], 1_073_741_824.0)
    } else if let Some(s) = upper.strip_suffix("TB") {
        (&t[..s.len()], 1_099_511_627_776.0)
    } else if let Some(s) = upper.strip_suffix('K') {
        (&t[..s.len()], 1_024.0)
    } else if let Some(s) = upper.strip_suffix('M') {
        (&t[..s.len()], 1_048_576.0)
    } else if let Some(s) = upper.strip_suffix('G') {
        (&t[..s.len()], 1_073_741_824.0)
    } else if let Some(s) = upper.strip_suffix('T') {
        (&t[..s.len()], 1_099_511_627_776.0)
    } else if let Some(s) = upper.strip_suffix('B') {
        (&t[..s.len()], 1.0)
    } else {
        (t, 1.0)
    };
    let value: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("tamanho inválido: {text}"))?;
    Ok((value * mul).round() as u64)
}

fn parse_mm_ss(text: &str) -> Result<u64, String> {
    let cleaned = text.trim_end_matches(']').trim_start_matches('[');
    let (m, s) = cleaned
        .split_once(':')
        .ok_or_else(|| format!("tempo inválido: {text}"))?;
    let mm: u64 = m
        .parse()
        .map_err(|_| format!("minutos inválidos: {text}"))?;
    let ss: u64 = s
        .parse()
        .map_err(|_| format!("segundos inválidos: {text}"))?;
    Ok(mm * 60 + ss)
}

/// Interpreta uma linha de progresso do medidor do sftp.
pub fn parse_progress_line(line: &str) -> Option<TransferProgress> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 5 {
        return None;
    }
    // Âncora pela direita: `... nome [com espaços] PCT TAM VEL TEMPO`.
    let n = cols.len();
    let pct: f64 = cols[n - 4].strip_suffix('%')?.parse().ok()?;
    let transferred = parse_human_size(cols[n - 3]).ok()?;
    let rate = parse_human_size(cols[n - 2]).ok()?;
    let eta = parse_mm_ss(cols[n - 1]).ok()?;
    let name: String = cols[..n - 4].join(" ");
    Some(TransferProgress {
        transfer_id: 0,
        file_name: name,
        pct,
        transferred,
        total: if pct > 1.0 {
            Some((transferred as f64 / (pct / 100.0)).round() as u64)
        } else {
            None
        },
        rate: Some(rate),
        eta: Some(eta),
        done: false,
        ok: false,
        error: None,
    })
}

fn emit(app: &AppHandle, p: TransferProgress) {
    let _ = app.emit("transfer-progress", p);
}

fn final_event(id: i64, ok: bool, error: Option<String>) -> TransferProgress {
    TransferProgress {
        transfer_id: id,
        file_name: String::new(),
        pct: if ok { 100.0 } else { 0.0 },
        transferred: 0,
        total: None,
        rate: None,
        eta: None,
        done: true,
        ok,
        error,
    }
}

/// Uma transferência pronta para rodar: o que copiar, em que servidor, com qual
/// credencial. Agrupar esses campos mantém a assinatura de `start` e do
/// `run_worker` pequena quando surgirem novas opções.
pub struct Job {
    pub db: DbHandle,
    pub server: Server,
    pub kind: &'static str,
    pub remote: String,
    pub local: String,
    pub recursive: bool,
    pub askpass: Option<AskPass>,
}

/// Inicia upload (`remote` é o destino no servidor) ou download em thread
/// própria. `recursive` usa `put -r`/`get -r`. Retomada: tenta `reput`/`reget`
/// para arquivos; se o cliente não suportar, refaz com `put`/`get` completos.
/// A senha de `askpass` vive apenas na memória desta thread e desaparece com a
/// transferência.
pub fn start(app: AppHandle, state: &TransferState, job: Job) -> Result<i64, String> {
    if job.kind != "upload" && job.kind != "download" {
        return Err("tipo de transferência inválido".into());
    }
    if !valid_transfer_path(&job.remote) {
        return Err("caminho remoto inválido (use um caminho absoluto ou host:/caminho)".into());
    }
    if !valid_local_path(&job.local) {
        return Err("caminho local inválido".into());
    }
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let handles = Arc::clone(&state.handles);
    handles
        .lock()
        .map_err(|_| "gerenciador de transferências indisponível".to_string())?
        .insert(
            id,
            Handle {
                kind: job.kind,
                remote_path: job.remote.clone(),
                local_path: job.local.clone(),
                server_name: job.server.name.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                child: Arc::new(Mutex::new(None)),
                started_at: Instant::now(),
            },
        );

    let app_for_thread = app.clone();
    let handles_for_thread = Arc::clone(&handles);
    thread::Builder::new()
        .name(format!("transfer-{id}"))
        .spawn(move || run_worker(app_for_thread, handles_for_thread, id, job))
        .map_err(|e| format!("falha ao iniciar a transferência: {e}"))?;
    Ok(id)
}

fn valid_transfer_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.chars().any(|c| "\n\r".contains(c))
        && (path.starts_with('/') || path.contains(':'))
}

fn valid_local_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.chars().any(|c| "\n\r".contains(c))
        && path.len() <= 4096
}

fn script_for(kind: &str, resume: bool, recursive: bool, src: &str, dst: &str) -> String {
    let command = match (kind, resume, recursive) {
        ("upload", true, _) => "reput",
        ("upload", false, true) => "put -r",
        ("upload", false, false) => "put",
        (_, true, _) => "reget",
        (_, false, true) => "get -r",
        _ => "get",
    };
    format!("{command} {} {}\nbye\n", sftp_quote(src), sftp_quote(dst))
}

fn transfer_src(kind: &str, remote: &str, local: &str) -> String {
    if kind == "upload" {
        local.to_string()
    } else {
        remote.to_string()
    }
}

fn transfer_dst(kind: &str, remote: &str, local: &str) -> String {
    if kind == "upload" {
        remote.to_string()
    } else {
        local.to_string()
    }
}

struct Attempt {
    /// true quando o erro indica que o cliente não conhece reput/reget.
    resume_unsupported: bool,
    cancelled: bool,
    ok: bool,
    message: String,
}

fn run_worker(app: AppHandle, handles: Arc<Mutex<HashMap<i64, Handle>>>, id: i64, job: Job) {
    let Job {
        db,
        server,
        kind,
        remote,
        local,
        recursive,
        askpass,
    } = job;
    let src = transfer_src(kind, &remote, &local);
    let dst = transfer_dst(kind, &remote, &local);
    // Retomada só faz sentido em arquivo único; recursivo já é put/get -r.
    let resume = !recursive;

    let mut attempt = run_once(
        &app,
        &handles,
        id,
        &server,
        script_for(kind, resume, recursive, &src, &dst),
        askpass.as_ref(),
    );
    if resume && attempt.resume_unsupported && !attempt.ok && !attempt.cancelled {
        attempt = run_once(
            &app,
            &handles,
            id,
            &server,
            script_for(kind, false, recursive, &src, &dst),
            askpass.as_ref(),
        );
    }

    if attempt.ok {
        emit(&app, final_event(id, true, None));
        finish(&app, &handles, &db, id, None);
        return;
    }
    if attempt.cancelled {
        // `run_once` já emitiu o evento de cancelamento; aqui só registramos.
        finish(
            &app,
            &handles,
            &db,
            id,
            Some("transferência cancelada pelo usuário"),
        );
        return;
    }
    emit(&app, final_event(id, false, Some(attempt.message.clone())));
    finish(&app, &handles, &db, id, Some(&attempt.message));
}

fn run_once(
    app: &AppHandle,
    handles: &Arc<Mutex<HashMap<i64, Handle>>>,
    id: i64,
    server: &Server,
    script: String,
    askpass: Option<&AskPass>,
) -> Attempt {
    let (cancel, child_slot) = {
        let guard = match handles.lock() {
            Ok(g) => g,
            Err(_) => {
                return Attempt {
                    resume_unsupported: false,
                    cancelled: false,
                    ok: false,
                    message: "gerenciador de transferências indisponível".into(),
                }
            }
        };
        let Some(h) = guard.get(&id) else {
            return Attempt {
                resume_unsupported: false,
                cancelled: false,
                ok: false,
                message: "transferência removida".into(),
            };
        };
        (Arc::clone(&h.cancel), Arc::clone(&h.child))
    };

    let mut cmd = match sftp_cmd(server, false, askpass) {
        Ok(c) => c,
        Err(e) => {
            return Attempt {
                resume_unsupported: false,
                cancelled: false,
                ok: false,
                message: e,
            }
        }
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Attempt {
                resume_unsupported: false,
                cancelled: false,
                ok: false,
                message: format!("não foi possível iniciar o sftp: {e}"),
            }
        }
    };
    let Some(mut stdin) = child.stdin.take() else {
        return Attempt {
            resume_unsupported: false,
            cancelled: false,
            ok: false,
            message: "falha ao abrir a entrada do sftp".into(),
        };
    };
    let Some(mut stdout) = child.stdout.take() else {
        return Attempt {
            resume_unsupported: false,
            cancelled: false,
            ok: false,
            message: "falha ao abrir a saída do sftp".into(),
        };
    };
    let Some(mut stderr) = child.stderr.take() else {
        return Attempt {
            resume_unsupported: false,
            cancelled: false,
            ok: false,
            message: "falha ao abrir o erro do sftp".into(),
        };
    };

    {
        let Ok(mut guard) = child_slot.lock() else {
            return Attempt {
                resume_unsupported: false,
                cancelled: false,
                ok: false,
                message: "gerenciador de transferências indisponível".into(),
            };
        };
        *guard = Some(child);
    }

    thread::spawn(move || {
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        drop(stdin);
    });

    // stderr é drenado em thread própria: um erro grande não pode nos travar.
    let (err_tx, err_rx) = std::sync::mpsc::channel::<String>();
    thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        let _ = err_tx.send(buf);
    });

    let mut line = String::new();
    let mut buf = [0u8; 8192];
    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for b in &buf[..n] {
                    match *b {
                        b'\r' | b'\n' => {
                            if !line.is_empty() {
                                if let Some(p) = parse_progress_line(&line) {
                                    emit(
                                        app,
                                        TransferProgress {
                                            transfer_id: id,
                                            ..p
                                        },
                                    );
                                }
                            }
                            line.clear();
                        }
                        c => line.push(c as char),
                    }
                }
            }
        }
    }

    let cancelled = cancel.load(Ordering::SeqCst);
    let err = err_rx
        .recv_timeout(Duration::from_secs(3))
        .unwrap_or_default();

    let status = {
        let Ok(mut guard) = child_slot.lock() else {
            return Attempt {
                resume_unsupported: false,
                cancelled,
                ok: false,
                message: "gerenciador de transferências indisponível".into(),
            };
        };
        let code = guard.as_mut().and_then(|c| {
            wait_deadline(c, Duration::from_millis(500))
                .as_ref()
                .and_then(|s| s.code())
        });
        if let Some(mut c) = guard.take() {
            if cancelled {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        code
    };

    if cancelled {
        emit(
            app,
            final_event(id, false, Some("transferência cancelada".into())),
        );
        return Attempt {
            resume_unsupported: false,
            cancelled: true,
            ok: false,
            message: "cancelada".into(),
        };
    }
    if status == Some(0) {
        return Attempt {
            resume_unsupported: false,
            cancelled: false,
            ok: true,
            message: String::new(),
        };
    }

    let lower = err.to_lowercase();
    let resume_unsupported = lower.contains("reput")
        || lower.contains("reget")
        || lower.contains("invalid command")
        || lower.contains("unrecognized")
        || lower.contains("unsupported");
    let message = if err.trim().is_empty() {
        format!(
            "a transferência falhou{}",
            status
                .map(|c| format!(" (sftp código {c})"))
                .unwrap_or_default()
        )
    } else {
        classify(&err)
    };
    Attempt {
        resume_unsupported,
        cancelled: false,
        ok: false,
        message,
    }
}

/// Espera o filho terminar por até `deadline`, sem `wait_timeout`
/// (que só existe em Unix). Devolve o status se houver, None caso contrário.
fn wait_deadline(child: &mut Child, deadline: Duration) -> Option<ExitStatus> {
    let limit = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if Instant::now() >= limit {
                    return None;
                }
                thread::sleep(Duration::from_millis(30));
            }
            Err(_) => return None,
        }
    }
}

fn classify(err: &str) -> String {
    let lower = err.to_lowercase();
    if lower.contains("host key verification failed") {
        "chave de host desconhecida — conecte pelo terminal e confirme a identidade do servidor antes".into()
    } else if lower.contains("permission denied") && lower.contains("publickey") {
        "autenticação recusada — confira a chave e o SSH Agent".into()
    } else if lower.contains("no such file") || lower.contains("not exist") {
        "arquivo ou diretório inexistente na origem".into()
    } else if lower.contains("permission denied") {
        "permissão negada pelo servidor".into()
    } else if lower.contains("connection refused") {
        "conexão recusada — verifique host e porta".into()
    } else if lower.contains("could not resolve") || lower.contains("name or service") {
        "host não resolvido (DNS)".into()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "tempo limite de conexão excedido".into()
    } else {
        err.trim().to_string()
    }
}

/// Auditoria do desfecho + remoção do handle. O handle precisa existir para
/// lermos `server_name/kind/remote/local` — por isso ele só sai do mapa aqui.
fn finish(
    _app: &AppHandle,
    handles: &Arc<Mutex<HashMap<i64, Handle>>>,
    db: &DbHandle,
    id: i64,
    error: Option<&str>,
) {
    let meta = {
        let Ok(map) = handles.lock() else { return };
        let Some(h) = map.get(&id) else { return };
        (
            h.server_name.clone(),
            h.kind,
            h.remote_path.clone(),
            h.local_path.clone(),
            h.started_at.elapsed().as_secs(),
        )
    };
    let Ok(conn) = db.lock() else { return };
    let (server_name, kind, remote, local, secs) = meta;
    let operation = if kind == "upload" {
        "Upload"
    } else {
        "Download"
    };
    let _ = match error {
        None => crate::audit::log(
            &conn,
            "success",
            "files",
            &format!("{operation} concluído"),
            Some(&server_name),
            None,
            None,
            Some(&format!("{local} → {remote} · {secs}s")),
        ),
        Some(e) => crate::audit::log(
            &conn,
            "error",
            "files",
            &format!("{operation} falhou"),
            Some(&server_name),
            None,
            None,
            Some(&format!("{local} → {remote} · {e}")),
        ),
    };
    drop(conn);
    if let Ok(mut map) = handles.lock() {
        map.remove(&id);
    }
}

pub fn cancel(state: &TransferState, id: i64) -> Result<(), String> {
    let (flag, slot) = {
        let map = state
            .handles
            .lock()
            .map_err(|_| "gerenciador de transferências indisponível".to_string())?;
        let h = map
            .get(&id)
            .ok_or_else(|| format!("transferência #{id} não encontrada ou já concluída"))?;
        (Arc::clone(&h.cancel), Arc::clone(&h.child))
    };
    flag.store(true, Ordering::SeqCst);
    if let Ok(mut guard) = slot.lock() {
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
        }
    }
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransferInfo {
    pub id: i64,
    pub kind: String,
    pub remote_path: String,
    pub local_path: String,
    pub server_name: String,
    pub elapsed_seconds: u64,
}

pub fn list_active(state: &TransferState) -> Vec<TransferInfo> {
    state
        .handles
        .lock()
        .map(|map| {
            map.iter()
                .map(|(id, h)| TransferInfo {
                    id: *id,
                    kind: h.kind.to_string(),
                    remote_path: h.remote_path.clone(),
                    local_path: h.local_path.clone(),
                    server_name: h.server_name.clone(),
                    elapsed_seconds: h.started_at.elapsed().as_secs(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn shutdown(state: &TransferState) {
    if let Ok(map) = state.handles.lock() {
        for h in map.values() {
            h.cancel.store(true, Ordering::SeqCst);
            if let Ok(mut guard) = h.child.lock() {
                if let Some(mut child) = guard.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseia_linha_de_progresso_padrao() {
        let p = parse_progress_line("arquivo.txt 45% 1.2MB 345KB/s 00:12").expect("linha válida");
        assert_eq!(p.file_name, "arquivo.txt");
        assert!((p.pct - 45.0).abs() < f64::EPSILON);
        assert_eq!(p.transferred, 1_258_291);
        assert_eq!(p.rate, Some(353_280));
        assert_eq!(p.eta, Some(12));
        assert!(p.total.unwrap() > p.transferred);
    }

    #[test]
    fn parseia_nome_com_espacos() {
        let p = parse_progress_line("pasta meu arquivo.bin 100% 2.0KB 500B/s 00:00")
            .expect("linha válida");
        assert_eq!(p.file_name, "pasta meu arquivo.bin");
        assert_eq!(p.transferred, 2048);
    }

    #[test]
    fn recusa_linhas_sem_progresso() {
        assert!(parse_progress_line("sftp> put arquivo").is_none());
        assert!(parse_progress_line("receiving arquivo").is_none());
        assert!(parse_progress_line("").is_none());
    }

    #[test]
    fn tamanhos_humanos() {
        assert_eq!(parse_human_size("512B").unwrap(), 512);
        assert_eq!(parse_human_size("1K").unwrap(), 1024);
        assert_eq!(parse_human_size("1.5M").unwrap(), 1_572_864);
        assert_eq!(parse_human_size("2G").unwrap(), 2 * 1_073_741_824);
        assert!(parse_human_size("abc").is_err());
    }

    #[test]
    fn script_de_upload_e_download() {
        assert!(
            script_for("upload", false, false, "/local/a.txt", "/remote/a.txt").starts_with("put ")
        );
        assert!(
            script_for("upload", true, false, "/local/a.txt", "/remote/a.txt")
                .starts_with("reput ")
        );
        assert!(
            script_for("upload", false, true, "/local/dir", "/remote/dir").starts_with("put -r ")
        );
        assert!(
            script_for("download", false, false, "/remote/a.txt", "/local/a.txt")
                .starts_with("get ")
        );
        assert!(
            script_for("download", true, false, "/remote/a.txt", "/local/a.txt")
                .starts_with("reget ")
        );
        assert!(
            script_for("download", false, true, "/remote/dir", "/local/dir").starts_with("get -r ")
        );
    }

    #[test]
    fn caminhos_validados_antes_de_iniciar() {
        assert!(valid_transfer_path("/srv/app/file.txt"));
        assert!(valid_transfer_path("host:/srv/app"));
        assert!(!valid_transfer_path("relativo.txt"));
        assert!(!valid_transfer_path("com\nnewline"));
        assert!(valid_local_path("C:\\tmp\\a.txt"));
        assert!(!valid_local_path(""));
        assert!(!valid_local_path("com\0null"));
    }
}
