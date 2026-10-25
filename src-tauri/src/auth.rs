//! Autenticação por senha sob demanda (opção do usuário, sem persistência).
//!
//! A senha nunca é gravada em disco nem no SQLite: ela vive apenas na memória
//! do processo e é entregue ao cliente OpenSSH pela variável de ambiente
//! `DEVOPS_SSH_PASSWORD`. O auxiliar de prompt (askpass) é um script temporário
//! que apenas imprime essa variável — não contém segredo algum — e é removido no
//! `Drop`.
//!
//! O OpenSSH só aceita `SSH_ASKPASS` quando os prompts estão habilitados, então
//! o modo senha troca `BatchMode=yes` por `BatchMode=no` e limita a uma tentativa
//! (`NumberOfPasswordPrompts=1`). A verificação de chave de host continua
//! estrita: `StrictHostKeyChecking=yes` é preservado por quem monta o comando.

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicI64, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::models::Server;

/// Máximo de espera pela resposta do usuário na janela do aplicativo.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);
/// Limita o tamanho aceito de uma senha digitada.
const MAX_PASSWORD_LEN: usize = 512;
const PASSWORD_ENV: &str = "DEVOPS_SSH_PASSWORD";
static SCRIPT_SEQ: AtomicI64 = AtomicI64::new(1);

/// Payload do evento `auth-request` consumido pela interface.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthRequest {
    pub id: i64,
    pub server_name: String,
    pub purpose: String,
    pub timeout_secs: u64,
}

#[derive(Default)]
struct Inner {
    pending: Mutex<HashMap<i64, Sender<Option<String>>>>,
    next_id: AtomicI64,
}

/// Atende solicitações de senha: cada operação registra uma pendência, emite o
/// evento para a UI e aguarda a resposta (ou o cancelamento/expiração).
#[derive(Clone, Default)]
pub struct AuthBroker {
    inner: Arc<Inner>,
}

impl AuthBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pede uma senha ao usuário usando a janela real do aplicativo.
    pub fn ask(&self, app: &AppHandle, server_name: &str, purpose: &str) -> Option<String> {
        self.request(server_name, purpose, ASK_TIMEOUT, |req| {
            let _ = app.emit("auth-request", req.clone());
        })
    }

    /// Núcleo desacoplado da UI: emite via closure e aguarda a resposta.
    pub fn request(
        &self,
        server_name: &str,
        purpose: &str,
        timeout: Duration,
        emit: impl FnOnce(&AuthRequest),
    ) -> Option<String> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let Ok(mut pending) = self.inner.pending.lock() else {
                return None;
            };
            pending.insert(id, tx);
        }
        emit(&AuthRequest {
            id,
            server_name: server_name.to_string(),
            purpose: purpose.to_string(),
            timeout_secs: timeout.as_secs(),
        });

        let answer = rx.recv_timeout(timeout).ok().flatten();

        // Se ninguém respondeu (expiração), descarta a pendência.
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.remove(&id);
        }
        answer
    }

    /// Entrega a senha digitada à operação em espera.
    pub fn respond(&self, id: i64, password: &str) -> Result<(), String> {
        if password.is_empty() {
            return Err("a senha não pode ser vazia".into());
        }
        if password.len() > MAX_PASSWORD_LEN {
            return Err(format!(
                "senha longa demais (máximo {MAX_PASSWORD_LEN} caracteres)"
            ));
        }
        let tx = self
            .inner
            .pending
            .lock()
            .map_err(|_| "o gerenciador de senhas está indisponível")?
            .remove(&id)
            .ok_or_else(|| "a solicitação de senha não está mais ativa".to_string())?;
        tx.send(Some(password.to_string()))
            .map_err(|_| "a operação que esperava a senha já encerrou".to_string())
    }

    /// O usuário cancelou o prompt: a operação deve falhar sem tentar conexão.
    pub fn cancel(&self, id: i64) -> Result<(), String> {
        let tx = self
            .inner
            .pending
            .lock()
            .map_err(|_| "o gerenciador de senhas está indisponível")?
            .remove(&id)
            .ok_or_else(|| "a solicitação de senha não está mais ativa".to_string())?;
        tx.send(None)
            .map_err(|_| "a operação que esperava a senha já encerrou".to_string())
    }

    /// Só faz sentido dentro dos testes unitários.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.inner.pending.lock().map(|p| p.len()).unwrap_or(0)
    }
}

/// Script temporário de askpass + senha em memória para uma única operação.
pub struct AskPass {
    password: String,
    script: PathBuf,
}

impl AskPass {
    pub fn new(password: &str) -> Result<Self, String> {
        if password.is_empty() {
            return Err("a senha não pode ser vazia".into());
        }
        if password.len() > MAX_PASSWORD_LEN {
            return Err(format!(
                "senha longa demais (máximo {MAX_PASSWORD_LEN} caracteres)"
            ));
        }
        let seq = SCRIPT_SEQ.fetch_add(1, Ordering::Relaxed);
        let (prefix, content) = if cfg!(windows) {
            (
                "bat",
                "@echo off\r\npowershell -NoProfile -NonInteractive -Command \"[Console]::Out.Write($env:DEVOPS_SSH_PASSWORD)\"\r\n"
                    .to_string(),
            )
        } else {
            (
                "sh",
                "#!/bin/sh\nprintf %s \"$DEVOPS_SSH_PASSWORD\"\n".to_string(),
            )
        };
        let script = std::env::temp_dir().join(format!(
            "devops-askpass-{}-{seq}.{prefix}",
            std::process::id()
        ));
        let mut file = std::fs::File::create(&script)
            .map_err(|e| format!("não foi possível preparar o prompt de senha: {e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("falha ao escrever o prompt de senha: {e}"))?;
        file.flush().ok();
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700));
        }

        Ok(Self {
            password: password.to_string(),
            script,
        })
    }

    /// Opções `-o` que habilitam o prompt de senha mantendo a checagem de host.
    pub fn ssh_options(&self) -> [String; 3] {
        [
            "BatchMode=no".into(),
            "NumberOfPasswordPrompts=1".into(),
            "PreferredAuthentications=password,keyboard-interactive".into(),
        ]
    }

    /// Aponta o OpenSSH para o auxiliar temporário e injeta a senha no ambiente.
    pub fn apply(&self, cmd: &mut Command) {
        cmd.env("SSH_ASKPASS", &self.script)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(PASSWORD_ENV, &self.password);
    }

    /// Usado apenas pelos testes que inspecionam o auxiliar temporário.
    #[cfg(test)]
    pub fn script_path(&self) -> &PathBuf {
        &self.script
    }
}

impl Drop for AskPass {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script);
        // Reduz a janela de leitura acidental do segredo em memória.
        self.password.clear();
    }
}

/// Servidores marcados como `password` pedem a senha em cada operação.
pub fn needs_password(server: &Server) -> bool {
    server.auth_method == "password"
}

/// Prepara o auxiliar de senha da operação de `server`, pedindo a senha na
/// interface. Retorna `Ok(None)` quando o acesso é por chave ou agente.
pub fn prepare(
    broker: &AuthBroker,
    app: &AppHandle,
    server: &Server,
    purpose: &str,
) -> Result<Option<AskPass>, String> {
    if !needs_password(server) {
        return Ok(None);
    }
    let password = broker
        .ask(app, &server.name, purpose)
        .ok_or_else(|| format!("nenhuma senha foi informada para \"{}\"", server.name))?;
    AskPass::new(&password).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_with(auth_method: &str) -> Server {
        Server {
            id: 1,
            name: "servidor".into(),
            host: "example.invalid".into(),
            port: 22,
            username: "ubuntu".into(),
            auth_method: auth_method.into(),
            identity_file: None,
            ssh_alias: None,
            environment: "dev".into(),
            group_name: None,
            notes: None,
            source: "manual".into(),
            last_status: None,
            last_status_detail: None,
            last_checked_at: None,
            last_ok_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn apenas_metodo_password_dispara_prompt() {
        assert!(needs_password(&server_with("password")));
        assert!(!needs_password(&server_with("key")));
        assert!(!needs_password(&server_with("agent")));
    }

    #[test]
    fn broker_entrega_senha_ao_solicitante() {
        let broker = AuthBroker::new();
        let responder = broker.clone();
        let announced = Arc::new(AtomicI64::new(-1));
        let observed = announced.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let id = observed.load(Ordering::Relaxed);
            assert_eq!(responder.respond(id, "senha-secret"), Ok(()));
        });
        let answer = broker.request(
            "servidor",
            "Teste de conexão",
            Duration::from_secs(5),
            |req| announced.store(req.id, Ordering::Relaxed),
        );
        assert_eq!(answer.as_deref(), Some("senha-secret"));
        assert_eq!(broker.pending_count(), 0);
    }

    #[test]
    fn cancelamento_devolve_nenhum_valor() {
        let broker = AuthBroker::new();
        let responder = broker.clone();
        let announced = Arc::new(AtomicI64::new(-1));
        let observed = announced.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let _ = responder.cancel(observed.load(Ordering::Relaxed));
        });
        let answer = broker.request(
            "servidor",
            "Teste de conexão",
            Duration::from_secs(5),
            |req| announced.store(req.id, Ordering::Relaxed),
        );
        assert_eq!(answer, None);
    }

    #[test]
    fn solicitacao_sem_resposta_expira_e_limpa() {
        let broker = AuthBroker::new();
        let answer = broker.request(
            "servidor",
            "Teste de conexão",
            Duration::from_millis(80),
            |req| assert_eq!(req.server_name, "servidor"),
        );
        assert_eq!(answer, None);
        assert_eq!(broker.pending_count(), 0);
    }

    #[test]
    fn responde_apenas_pendencia_ativa() {
        let broker = AuthBroker::new();
        assert!(broker.respond(99, "senha").is_err());
        assert!(broker.cancel(99).is_err());
    }

    #[test]
    fn rejeita_senha_vazia_ou_longa() {
        let broker = AuthBroker::new();
        assert!(broker.respond(1, "").is_err());
        assert!(AskPass::new("").is_err());
        assert!(AskPass::new(&"a".repeat(MAX_PASSWORD_LEN + 1)).is_err());
    }

    #[test]
    fn script_de_askpass_nao_contem_a_senha_e_e_removido_no_drop() {
        let script = {
            let ask = AskPass::new("senha-secret").unwrap();
            let path = ask.script_path().clone();
            assert!(path.exists());
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(
                !content.contains("senha-secret"),
                "o script não pode guardar a senha"
            );
            assert!(content.contains(PASSWORD_ENV));
            assert_eq!(ask.ssh_options()[0], "BatchMode=no");
            assert_eq!(
                ask.ssh_options()[2],
                "PreferredAuthentications=password,keyboard-interactive"
            );
            path
        };
        assert!(!script.exists(), "o auxiliar temporário é apagado no Drop");
    }

    #[test]
    fn apply_publica_apenas_variaveis_de_ambiente() {
        let ask = AskPass::new("senha-secret").unwrap();
        let mut cmd = Command::new("ssh");
        ask.apply(&mut cmd);
        let envs: HashMap<_, _> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().to_string(),
                    v.map(|s| s.to_string_lossy().to_string()),
                )
            })
            .collect();
        assert_eq!(
            envs.get("SSH_ASKPASS_REQUIRE").and_then(|v| v.as_deref()),
            Some("force")
        );
        assert_eq!(
            envs.get(PASSWORD_ENV).and_then(|v| v.as_deref()),
            Some("senha-secret")
        );
        assert!(envs.contains_key("SSH_ASKPASS"));
    }
}
