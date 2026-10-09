use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::auth::AskPass;
use crate::models::ConnectionTestResult;

const CONNECT_TIMEOUT_SECS: u64 = 10;
const HARD_DEADLINE_SECS: u64 = 25;

/// Executa um cliente ssh em modo não interativo com verificação estrita de
/// chave de host. Nunca aceita automaticamente um host desconhecido ou com
/// identidade alterada: nesses casos a conexão falha e a causa é reportada.
///
/// `askpass` presente significa autenticação por senha: o prompt é respondido
/// pelo auxiliar temporário do `AskPass`, nunca por um terminal escondido.
///
/// Todos os argumentos são passados como argv separado — nenhuma string é
/// concatenada em um comando shell.
pub fn test(
    ssh_alias: Option<&str>,
    host: &str,
    port: i64,
    username: &str,
    identity_file: Option<&str>,
    askpass: Option<&AskPass>,
) -> ConnectionTestResult {
    let start = Instant::now();
    let mut cmd = Command::new("ssh");
    match askpass {
        Some(pass) => {
            pass.apply(&mut cmd);
            for option in pass.ssh_options() {
                cmd.arg("-o").arg(option);
            }
        }
        None => {
            cmd.arg("-o").arg("BatchMode=yes");
        }
    }
    cmd.arg("-o").arg("StrictHostKeyChecking=yes")
        .arg("-o").arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        .arg("-o").arg("LogLevel=ERROR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(idf) = identity_file {
        let expanded = crate::ssh_config::expand_tilde(idf);
        cmd.arg("-i").arg(expanded);
    }

    let target = match ssh_alias {
        Some(alias) => alias.to_string(),
        None => {
            cmd.arg("-p").arg(port.to_string());
            format!("{username}@{host}")
        }
    };
    cmd.arg(&target).arg("true");

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ConnectionTestResult {
                ok: false,
                exit_code: None,
                duration_ms: start.elapsed().as_millis() as u64,
                error_kind: Some("other".into()),
                message: if e.kind() == std::io::ErrorKind::NotFound {
                    "Cliente ssh não encontrado no PATH. Instale o OpenSSH ou ajuste o PATH.".into()
                } else {
                    format!("não foi possível iniciar o cliente ssh: {e}")
                },
            };
        }
    };

    let (status, _stdout, stderr) = wait_with_deadline(child, Duration::from_secs(HARD_DEADLINE_SECS));
    let duration_ms = start.elapsed().as_millis() as u64;
    let stderr_text = String::from_utf8_lossy(&stderr).trim().to_string();

    let timed_out = status.is_none();
    let exit_code = status.as_ref().and_then(|s| s.code()).map(|c| c as i64);

    if timed_out {
        return ConnectionTestResult {
            ok: false,
            exit_code: None,
            duration_ms,
            error_kind: Some("timeout".into()),
            message: format!(
                "tempo limite excedido ({HARD_DEADLINE_SECS}s) sem resposta do servidor{}",
                stderr_suffix(&stderr_text)
            ),
        };
    }

    let ok = status.map(|s| s.success()).unwrap_or(false);
    if ok {
        return ConnectionTestResult {
            ok: true,
            exit_code,
            duration_ms,
            error_kind: None,
            message: "Conexão estabelecida e autenticação aceita.".into(),
        };
    }

    let (kind, message) = classify(&stderr_text, exit_code);
    ConnectionTestResult {
        ok: false,
        exit_code,
        duration_ms,
        error_kind: Some(kind),
        message,
    }
}

fn stderr_suffix(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" — {stderr}")
    }
}

fn classify(stderr: &str, exit_code: Option<i64>) -> (String, String) {
    let lower = stderr.to_lowercase();
    if lower.contains("permission denied") || lower.contains("no more authentication methods available") {
        return (
            "auth".into(),
            format!(
                "falha de autenticação — o servidor recusou a credencial. Para chave/agente verifique ~/.ssh e `ssh-add -l`; para senha confirme o usuário e a senha informados no prompt{}",
                stderr_suffix(stderr),
            ),
        );
    }
    if lower.contains("could not resolve hostname") || lower.contains("name or service not known") || lower.contains("no such host") {
        return (
            "dns".into(),
            format!("host não resolvido (DNS){}", stderr_suffix(stderr)),
        );
    }
    if lower.contains("connection refused") {
        return (
            "refused".into(),
            format!("conexão recusada — a porta pode estar fechada ou o serviço sshd não está ativo{}", stderr_suffix(stderr)),
        );
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return (
            "timeout".into(),
            format!("tempo limite de conexão excedido{}", stderr_suffix(stderr)),
        );
    }
    if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
        || lower.contains("host key for")
        || lower.contains("known_hosts")
    {
        return (
            "hostkey".into(),
            format!(
                "chave de host desconhecida ou alterada — por segurança a conexão não prossegue. Rode manualmente `ssh <alvo>` para inspecionar e confirmar a identidade do servidor se for legítimo{}",
                stderr_suffix(stderr),
            ),
        );
    }
    if lower.contains("no route to host") || lower.contains("network is unreachable") || lower.contains("network is down") {
        return (
            "network".into(),
            format!("sem rota para o host{}", stderr_suffix(stderr)),
        );
    }
    if lower.contains("connection reset") || lower.contains("connection closed") {
        return (
            "refused".into(),
            format!("conexão encerrada pelo servidor durante o handshake{}", stderr_suffix(stderr)),
        );
    }
    match exit_code {
        Some(255) => (
            "other".into(),
            format!("falha na conexão (ssh retornou 255){}", stderr_suffix(stderr)),
        ),
        Some(c) => (
            "other".into(),
            format!("ssh encerrou com código {c}{}", stderr_suffix(stderr)),
        ),
        None => (
            "other".into(),
            format!("falha na conexão{}", stderr_suffix(stderr)),
        ),
    }
}

/// Espera o processo concluído, matando-o se ultrapassar o prazo. Lê os pipes
/// no mesmo encadeamento para não travar em buffers cheios.
fn wait_with_deadline(
    mut child: std::process::Child,
    deadline: Duration,
) -> (Option<std::process::ExitStatus>, Vec<u8>, Vec<u8>) {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let limit = Instant::now() + deadline;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= limit {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = tx.send((None, Vec::new(), Vec::new()));
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
        // Consome o restante dos pipes após a saída do processo.
        let mut stdout = Vec::new();
        if let Some(mut pipe) = child.stdout.take() {
            let _ = pipe.read_to_end(&mut stdout);
        }
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_end(&mut stderr);
        }
        let status = child.wait();
        let _ = tx.send((status.ok(), stdout, stderr));
    });
    rx.recv().unwrap_or((None, Vec::new(), Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifica_auth() {
        let (kind, _) = classify("Permission denied (publickey).", Some(255));
        assert_eq!(kind, "auth");
    }

    #[test]
    fn classifica_dns() {
        let (kind, _) = classify("ssh: Could not resolve hostname nada: Name or service not known", Some(255));
        assert_eq!(kind, "dns");
    }

    #[test]
    fn classifica_recusa() {
        let (kind, _) = classify("kex_exchange_identification: Connection refused", Some(255));
        assert_eq!(kind, "refused");
    }

    #[test]
    fn classifica_hostkey() {
        let (kind, _) = classify("Host key verification failed.", Some(255));
        assert_eq!(kind, "hostkey");
    }

    // Exercício ponta a ponta do ciclo real: spawn do cliente ssh, watchdog
    // de prazo, leitura de pipes e classificação. Usa uma porta local fechada;
    // exige o ssh no PATH — requisito da própria aplicação.
    #[test]
    fn teste_real_conexao_a_porta_fechada() {
        let result = test(None, "127.0.0.1", 1, "nobody", None, None);
        assert!(!result.ok);
        assert!(result.duration_ms > 0);
        assert!(!result.message.is_empty());
        assert!(result.error_kind.is_some());
    }

    // O caminho de senha usa BatchMode=no + SSH_ASKPASS. Com a porta fechada o
    // cliente encerra antes de pedir credencial: a operação precisa falhar sem
    // travar esperando um prompt que nunca chega.
    #[test]
    fn teste_real_com_askpass_nao_pende() {
        let pass = crate::auth::AskPass::new("senha-de-teste").unwrap();
        let start = Instant::now();
        let result = test(None, "127.0.0.1", 1, "nobody", None, Some(&pass));
        assert!(!result.ok);
        assert!(result.error_kind.is_some());
        assert!(start.elapsed() < Duration::from_secs(HARD_DEADLINE_SECS + 10));
    }
}
