//! Sessões de terminal SSH reais: cada aba executa o cliente `ssh` nativo
//! dentro de um pseudo-terminal (ConPTY no Windows, openpty no Linux).
//! A verificação de chave de host permanece interativa no próprio terminal —
//! a aplicação nunca aceita um host desconhecido automaticamente.

use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use tauri::{AppHandle, Emitter};

use crate::{
    audit,
    db,
    models::{Server, TerminalSessionInfo},
    ssh_config,
};

type DbHandle = db::DbHandle;

const CONNECT_TIMEOUT_SECS: u64 = 20;

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputEvent {
    pub session_id: i64,
    pub data: String,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitEvent {
    pub session_id: i64,
    pub exit_code: Option<i64>,
}

struct Session {
    info: TerminalSessionInfo,
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
}

pub struct TerminalState {
    sessions: Arc<Mutex<HashMap<i64, Session>>>,
    next_id: AtomicI64,
}

impl TerminalState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicI64::new(1),
        }
    }
}

fn lock_sessions(term: &TerminalState) -> Result<MutexGuard<'_, HashMap<i64, Session>>, String> {
    term.sessions
        .lock()
        .map_err(|_| "o gerenciador de terminal está indisponível".to_string())
}

type SpawnedPty = (
    Box<dyn MasterPty + Send>,
    Box<dyn Write + Send>,
    Box<dyn Read + Send>,
    Arc<Mutex<Box<dyn Child + Send + Sync>>>,
);

pub fn open_session(
    app: &AppHandle,
    db: &DbHandle,
    term: &TerminalState,
    server_id: i64,
    cols: u16,
    rows: u16,
) -> Result<TerminalSessionInfo, String> {
    let server = {
        let conn = db
            .lock()
            .map_err(|_| "o banco de dados local está indisponível".to_string())?;
        db::get_server(&conn, server_id)?
    };

    let id = term.next_id.fetch_add(1, Ordering::SeqCst);
    let info = TerminalSessionInfo {
        id,
        server_id: server.id,
        server_name: server.name.clone(),
        environment: server.environment.clone(),
        username: server.username.clone(),
        host: server.host.clone(),
        port: server.port,
        alive: true,
        exit_code: None,
        started_at: db::now_rfc3339(),
    };

    let (master, writer, reader, child) = match spawn_pty(&server, cols, rows) {
        Ok(v) => v,
        Err(e) => {
            if let Ok(conn) = db.lock() {
                let _ = audit::log(
                    &conn,
                    "error",
                    "terminal",
                    "Falha ao abrir sessão de terminal",
                    Some(&server.name),
                    None,
                    None,
                    Some(&e),
                );
            }
            return Err(e);
        }
    };

    {
        let mut sessions = lock_sessions(term)?;
        sessions.insert(
            id,
            Session {
                info: info.clone(),
                master,
                writer: Mutex::new(writer),
                child: Arc::clone(&child),
            },
        );
    }

    let sessions_for_exit = Arc::clone(&term.sessions);
    let app_for_exit = app.clone();
    let app_for_reader = app.clone();
    let db_for_exit = Arc::clone(db);
    let name_for_exit = server.name.clone();
    let child_for_exit = Arc::clone(&child);
    std::thread::Builder::new()
        .name(format!("terminal-read-{id}"))
        .spawn(move || pump_output(app_for_reader, id, reader))
        .map_err(|e| format!("falha ao iniciar a leitura do terminal: {e}"))?;

    std::thread::Builder::new()
        .name(format!("terminal-exit-{id}"))
        .spawn(move || {
            watch_exit(
                app_for_exit,
                sessions_for_exit,
                db_for_exit,
                id,
                name_for_exit,
                child_for_exit,
            )
        })
        .map_err(|e| format!("falha ao vigiar o encerramento do terminal: {e}"))?;

    if let Ok(conn) = db.lock() {
        audit::log(
            &conn,
            "success",
            "terminal",
            "Sessão de terminal iniciada",
            Some(&server.name),
            None,
            None,
            Some(&format!("{}@{}:{}", server.username, server.host, server.port)),
        )?;
    }
    Ok(info)
}

fn spawn_pty(server: &Server, cols: u16, rows: u16) -> Result<SpawnedPty, String> {
    let cmd = build_ssh_command(server);
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("não foi possível abrir o pseudo-terminal: {e}"))?;
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("não foi possível iniciar o cliente ssh: {e}"))?;
    let child: Arc<Mutex<Box<dyn Child + Send + Sync>>> = Arc::new(Mutex::new(child));
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("falha ao preparar a leitura do terminal: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("falha ao preparar a escrita no terminal: {e}"))?;
    Ok((pair.master, writer, reader, child))
}

/// Drena o PTY transmitindo a saída em tempo real. Termina quando o ssh morre
/// e o observador (`watch_exit`) fecha o master. No Windows/ConPTY o read só
/// devolve EOF quando as alças do pseudo-terminal são fechadas, por isso a
/// detecção de saída não pode depender deste laço.
fn pump_output(app: AppHandle, session_id: i64, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = String::from_utf8_lossy(&buf[..n]).to_string();
                if app
                    .emit("terminal-output", TerminalOutputEvent { session_id, data })
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Vigia o processo ssh por polling. Quando ele termina, marca a sessão como
/// morta, fecha o master (destravando o leitor) e emite o código de saída.
/// Polling — e não `wait()` bloqueante — para não segurar o lock do filho e
/// permitir que `close_session` o encerre pelo meio.
fn watch_exit(
    app: AppHandle,
    sessions: Arc<Mutex<HashMap<i64, Session>>>,
    db: DbHandle,
    session_id: i64,
    server_name: String,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
) {
    let exit_code = loop {
        let done = child
            .lock()
            .ok()
            .and_then(|mut c| c.try_wait().ok())
            .flatten();
        if let Some(status) = done {
            break Some(status.exit_code() as i64);
        }
        if !sessions.lock().map(|m| m.contains_key(&session_id)).unwrap_or(false) {
            // close_session já removeu e encerrou; nada a fazer.
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    };
    let Some(exit_code) = exit_code else { return };

    // Sessão ausente do mapa = o usuário fechou a aba (auditoria feita lá).
    let session = sessions.lock().ok().and_then(|mut m| m.remove(&session_id));
    let Some(session) = session else { return };
    // Ao sair do escopo, `session` fecha master/writer → o leitor recebe EOF.
    drop(session);

    let _ = app.emit(
        "terminal-exit",
        TerminalExitEvent {
            session_id,
            exit_code: Some(exit_code),
        },
    );
    if let Ok(conn) = db.lock() {
        let _ = audit::log(
            &conn,
            "info",
            "terminal",
            "Sessão de terminal encerrada",
            Some(&server_name),
            None,
            Some(exit_code),
            Some("o processo ssh finalizou"),
        );
    }
}

pub fn write_session(term: &TerminalState, session_id: i64, data: &str) -> Result<(), String> {
    let sessions = lock_sessions(term)?;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| format!("a sessão de terminal #{session_id} não existe"))?;
    if !session.info.alive {
        return Err("a sessão de terminal já foi encerrada".into());
    }
    let mut writer = session
        .writer
        .lock()
        .map_err(|_| "o terminal não aceita escrita neste momento".to_string())?;
    writer
        .write_all(data.as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|e| format!("falha ao enviar dados ao terminal: {e}"))
}

pub fn resize_session(term: &TerminalState, session_id: i64, cols: u16, rows: u16) -> Result<(), String> {
    if !(2..=500).contains(&cols) || !(2..=300).contains(&rows) {
        return Err(format!("dimensões de terminal inválidas: {cols}x{rows}"));
    }
    let sessions = lock_sessions(term)?;
    let session = sessions
        .get(&session_id)
        .ok_or_else(|| format!("a sessão de terminal #{session_id} não existe"))?;
    if !session.info.alive {
        return Err("a sessão de terminal já foi encerrada".into());
    }
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("falha ao redimensionar o terminal: {e}"))
}

pub fn close_session(db: &DbHandle, term: &TerminalState, session_id: i64) -> Result<(), String> {
    let session = {
        let mut sessions = lock_sessions(term)?;
        sessions
            .remove(&session_id)
            .ok_or_else(|| format!("a sessão de terminal #{session_id} não existe"))?
    };
    if let Ok(mut child) = session.child.lock() {
        let _ = child.kill();
    }
    drop(session.master);
    let conn = db
        .lock()
        .map_err(|_| "o banco de dados local está indisponível".to_string())?;
    audit::log(
        &conn,
        "info",
        "terminal",
        "Sessão de terminal encerrada",
        Some(&session.info.server_name),
        None,
        session.info.exit_code,
        Some("encerrada pelo usuário na interface"),
    )
}

pub fn list_sessions(term: &TerminalState) -> Result<Vec<TerminalSessionInfo>, String> {
    let sessions = lock_sessions(term)?;
    let mut out: Vec<TerminalSessionInfo> = sessions.values().map(|s| s.info.clone()).collect();
    out.sort_by_key(|s| s.id);
    Ok(out)
}

/// Encerra todas as sessões — chamado no desligamento da aplicação.
pub fn shutdown(term: &TerminalState) {
    if let Ok(mut sessions) = term.sessions.lock() {
        for (_, s) in sessions.drain() {
            if let Ok(mut child) = s.child.lock() {
                let _ = child.kill();
            }
            drop(s.master);
        }
    }
}

/// Monta o argv do cliente ssh com argumentos separados — nenhuma string é
/// concatenada em um comando shell.
fn build_ssh_command(server: &Server) -> CommandBuilder {
    let mut cmd = CommandBuilder::new("ssh");
    cmd.arg("-tt");
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=ask");
    cmd.arg("-o");
    cmd.arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"));
    cmd.arg("-o");
    cmd.arg("ServerAliveInterval=30");
    cmd.arg("-o");
    cmd.arg("ServerAliveCountMax=4");
    if let Some(identity) = &server.identity_file {
        cmd.arg("-i");
        cmd.arg(ssh_config::expand_tilde(identity));
    }
    match &server.ssh_alias {
        // Alias do ~/.ssh/config respeita proxy jump e demais opções do usuário.
        Some(alias) => {
            cmd.arg(alias);
        }
        None => {
            cmd.arg("-p");
            cmd.arg(server.port.to_string());
            cmd.arg(format!("{}@{}", server.username, server.host));
        }
    }
    cmd.env("TERM", "xterm-256color");
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn server_inalcavel(porta: i64) -> Server {
        Server {
            id: 1,
            name: "porta-fechada".into(),
            host: "127.0.0.1".into(),
            port: porta,
            username: "tester".into(),
            auth_method: "agent".into(),
            identity_file: None,
            ssh_alias: None,
            environment: "other".into(),
            group_name: None,
            notes: None,
            source: "manual".into(),
            last_status: None,
            last_status_detail: None,
            last_checked_at: None,
            last_ok_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    /// Exercita o pipeline real: ConPTY + argv montado + processo ssh + leitura
    /// da saída + código de saída. A porta TCP foi aberta e descartada, então a
    /// conexão é recusada, o ssh falha e o que ele imprime deve chegar pelo PTY.
    #[test]
    fn spawn_pty_executa_ssh_e_captura_saida_e_codigo() {
        let porta = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").expect("porta livre local");
            let p = l.local_addr().expect("addr").port() as i64;
            drop(l);
            p
        };
        let (master, writer, mut reader, child) =
            spawn_pty(&server_inalcavel(porta), 80, 24).expect("não foi possível abrir o pseudo-terminal");

        // Drena o PTY em paralelo; quando o ssh morre, a leitura atinge EOF.
        let (tx, rx) = std::sync::mpsc::channel::<(Vec<u8>, bool)>();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => out.extend_from_slice(&buf[..n]),
                }
            }
            let _ = tx.send((out, true));
        });

        // Polling: wait() bloqueante poderia travar se o leitor não estivesse ativo.
        let started = Instant::now();
        let status = loop {
            match child
                .lock()
                .expect("lock do filho")
                .try_wait()
                .expect("try_wait do ssh")
            {
                Some(s) => break s,
                None => {
                    if started.elapsed() > Duration::from_secs(30) {
                        let _ = child.lock().expect("lock").kill();
                        panic!("o ssh não finalizou em 30s");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };

        // No Windows/ConPTY o leitor só atinge EOF quando o master é fechado;
        // derrubamos as alças para destravar o read bloqueado do leitor.
        drop(master);
        drop(writer);

        let (out, eof) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("o leitor do PTY não chegou ao EOF em 5s após fechar o master");
        let texto = String::from_utf8_lossy(&out).to_lowercase();

        assert_ne!(status.exit_code(), 0, "ssh deveria sair com erro na porta recusada");
        assert!(eof, "a leitura do PTY deveria terminar após fechar o master");
        assert!(
            texto.contains("127.0.0.1") || texto.contains("refused") || texto.contains("connection"),
            "a saída do ssh deveria chegar pelo PTY; recebemos: {texto:?}"
        );
    }
}
