use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{auth::AskPass, models::Server};

const CONNECT_TIMEOUT_SECS: u64 = 20;
const POLL_MS: u64 = 200;
/// Depois que o filho termina, damos este prazo para o último byte dos canos
/// aparecer. Sem isso, um netcat ou daemon que herda o stdout seguraria a leitura.
const DRAIN_GRACE: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub struct ExecResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

impl ExecResult {
    pub fn ok(&self) -> bool {
        !self.timed_out && !self.cancelled && self.exit_code == Some(0)
    }

    /// Uma linha única resumindo o que aconteceu, já sem segredos.
    pub fn summary(&self) -> String {
        if self.cancelled {
            return "interrompido pelo usuário".to_string();
        }
        if self.timed_out {
            return "tempo limite excedido — o processo foi encerrado".to_string();
        }
        match self.exit_code {
            Some(0) => "concluído com sucesso".to_string(),
            Some(code) => format!("código de saída {code}"),
            None => "o processo não informou o código de saída".to_string(),
        }
    }
}

fn drain(mut pipe: impl Read, tx: mpsc::Sender<Vec<u8>>) {
    let mut out = Vec::new();
    let _ = pipe.read_to_end(&mut out);
    let _ = tx.send(out);
}

fn take_string(rx: mpsc::Receiver<Vec<u8>>) -> String {
    match rx.recv_timeout(DRAIN_GRACE) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => String::new(),
    }
}

fn ssh_base(server: &Server, askpass: Option<&AskPass>) -> Result<Command, String> {
    let alias = server.ssh_alias.as_deref().unwrap_or("").trim();
    if alias.is_empty() && server.host.trim().is_empty() {
        return Err(format!(
            "servidor “{}” sem host nem alias — edite o cadastro antes de publicar",
            server.name
        ));
    }
    let mut cmd = crate::proc::command("ssh");
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
    cmd.arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("LogLevel=ERROR");
    if let Some(idf) = &server.identity_file {
        cmd.arg("-i").arg(crate::ssh_config::expand_tilde(idf));
    }
    if !alias.is_empty() {
        cmd.arg(alias);
    } else {
        cmd.arg("-p").arg(server.port.to_string());
        cmd.arg(format!("{}@{}", server.username, server.host));
    }
    Ok(cmd)
}

/// O shell remoto é escolhido depois do teste de requisitos, e só aceita os
/// nomes conhecidos — nada de string arbitrária virando argv.
pub fn shell_arg(shell: &str) -> Result<&'static str, String> {
    match shell {
        "bash" => Ok("bash"),
        "sh" => Ok("sh"),
        other => Err(format!("shell remoto não suportado: {other}")),
    }
}

/// Os argumentos que acompanham o script vão ao argv remoto; nada de espaços,
/// aspas ou metacaracteres — o corpo do script viaja pela stdin.
pub fn validate_remote_args(args: &[&str]) -> Result<(), String> {
    for arg in args {
        if arg.is_empty() {
            return Err("argumento remoto vazio".to_string());
        }
        if arg
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '_' | '/' | '-' | ':'))
        {
            return Err(format!("argumento remoto rejeitado por segurança: {arg}"));
        }
    }
    Ok(())
}

/// Executa `script` no servidor.
///
/// O corpo do script vai **pela stdin** (`ssh <dest> bash -s`), então nenhuma
/// parte dele é concatenada em argv nem aparece na linha de comando local — os
/// valores das variáveis do deploy ficam apenas no script, que nunca é gravado
/// nem publicado nos logs.
pub fn run_script(
    server: &Server,
    askpass: Option<&AskPass>,
    shell: &str,
    script: &str,
    args: &[&str],
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<ExecResult, String> {
    let shell = shell_arg(shell)?;
    validate_remote_args(args)?;
    let mut cmd = ssh_base(server, askpass)?;
    cmd.arg(shell).arg("-s");
    if !args.is_empty() {
        cmd.arg("--");
        for arg in args {
            cmd.arg(arg);
        }
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("falha ao iniciar ssh no servidor “{}”: {e}", server.name))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "não foi possível abrir a entrada do ssh".to_string())?;
    let script_bytes = script.as_bytes().to_vec();
    thread::spawn(move || {
        let _ = stdin.write_all(&script_bytes);
        let _ = stdin.flush();
    });

    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        thread::spawn(move || drain(pipe, tx_out));
    }
    if let Some(pipe) = child.stderr.take() {
        thread::spawn(move || drain(pipe, tx_err));
    }

    let outcome = harvest(&mut child, timeout, cancel);
    let (exit_code, timed_out, cancelled) = outcome;
    let stdout = take_string(rx_out);
    let stderr = take_string(rx_err);
    if cancelled {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(ExecResult {
        exit_code,
        stdout,
        stderr,
        timed_out,
        cancelled,
    })
}

/// Executa um comando local exatamente como cadastrado no modelo de deploy
/// (testes e build). `cmd /C` no Windows, `sh -c` nos demais: o comando é um
/// único argv, nunca concatenação com o caminho do projeto.
pub fn run_local(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<ExecResult, String> {
    run_local_with_env(command, cwd, &[], timeout, cancel)
}

/// O mesmo comando local, com variáveis extras no ambiente do processo. É por
/// aqui que um deploy sem servidor recebe as variáveis do modelo: ambiente, e não
/// linha de comando — o texto do comando é o que aparece na interface e no log.
pub fn run_local_with_env(
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<ExecResult, String> {
    if command.contains('\0') {
        return Err("o comando contém caractere nulo".to_string());
    }
    if command.trim().is_empty() {
        return Err("comando vazio".to_string());
    }
    let (program, flag) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
    let mut cmd = crate::proc::command(program);
    cmd.arg(flag)
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (chave, valor) in env {
        cmd.env(chave, valor);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("interpretador de comandos ({program}) não encontrado no PATH")
        } else {
            format!("falha ao iniciar o comando local: {e}")
        }
    })?;
    Ok(collect(&mut child, timeout, cancel))
}

/// Colhe um processo já iniciado dentro do prazo, drenando stdout e stderr em
/// threads: um cano cheio nunca trava a coleta. No estouro de prazo ou
/// cancelamento o filho é encerrado antes da função voltar.
pub fn collect(child: &mut Child, timeout: Duration, cancel: &AtomicBool) -> ExecResult {
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        thread::spawn(move || drain(pipe, tx_out));
    }
    if let Some(pipe) = child.stderr.take() {
        thread::spawn(move || drain(pipe, tx_err));
    }
    let (exit_code, timed_out, cancelled) = harvest(child, timeout, cancel);
    if timed_out || cancelled {
        let _ = child.kill();
        let _ = child.wait();
    }
    ExecResult {
        exit_code,
        stdout: take_string(rx_out),
        stderr: take_string(rx_err),
        timed_out,
        cancelled,
    }
}

/// Colhe o processo até o prazo ou até o cancelamento, devolvendo o código de
/// saída e as duas flags. O cancelamento é cooperativo: a interface levanta a
/// flag e esta espera termina em no máximo `POLL_MS`.
pub fn harvest(child: &mut Child, timeout: Duration, cancel: &AtomicBool) -> (Option<i32>, bool, bool) {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status.code(), false, false),
            Ok(None) => {}
            Err(_) => return (None, false, false),
        }
        if cancel.load(Ordering::SeqCst) {
            return (None, false, true);
        }
        if started.elapsed() >= timeout {
            return (None, true, false);
        }
        thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// Script de checagem de requisitos. Retorna uma linha por ferramenta
/// encontrada (`tool <caminho>`); o motor transforma isso no mapa de recursos.
pub fn probe_script(
    need_git: bool,
    need_docker: bool,
    need_compose: bool,
    need_rsync: bool,
    need_cksum: bool,
) -> String {
    let mut tools = vec!["bash", "tar", "find"];
    if need_git {
        tools.push("git");
    }
    if need_docker {
        tools.push("docker");
    }
    if need_compose {
        tools.push("docker-compose");
    }
    if need_rsync {
        tools.push("rsync");
    }
    if need_cksum {
        tools.push("cksum");
    }
    let mut script = String::from("set -u\n");
    for tool in tools {
        script.push_str(&format!(
            "p=$(command -v {tool} 2>/dev/null) && echo \"{tool} $p\" || echo \"{tool} -\"\n"
        ));
    }
    script.push_str("echo \"shell $BASH_VERSION\"\n");
    script
}

/// Interpreta a saída de `probe_script`.
pub fn parse_probe(raw: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, path)) = line.split_once(' ') else { continue };
        out.push((name.to_string(), if path == "-" { None } else { Some(path.to_string()) }));
    }
    out
}

/// Aspas simples para dentro de um script POSIX.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server {
            id: 1,
            name: "web".into(),
            host: "example.invalid".into(),
            port: 22,
            username: "deploy".into(),
            auth_method: "key".into(),
            identity_file: None,
            ssh_alias: None,
            environment: "staging".into(),
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

    #[test]
    fn servidor_sem_destino_e_recusado_antes_de_spawn() {
        let mut s = server();
        s.host = " ".into();
        let erro = ssh_base(&s, None).unwrap_err();
        assert!(erro.contains("sem host nem alias"), "{erro}");
        assert!(ssh_base(&server(), None).is_ok());
    }

    #[test]
    fn alias_substitui_host_e_porta_no_destino() {
        let mut s = server();
        s.ssh_alias = Some("  web-prod  ".into());
        let cmd = ssh_base(&s, None).unwrap();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.contains(&"web-prod".to_string()), "{args:?}");
        assert!(!args.iter().any(|a| a == "-p"), "{args:?}");
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"StrictHostKeyChecking=yes".to_string()));
    }

    #[test]
    fn quote_de_aspas_simples_protege_o_script() {
        assert_eq!(shell_quote("/var/www/site"), "'/var/www/site'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        // uma injeção clássica vira texto inofensivo dentro das aspas
        assert_eq!(shell_quote("x'; rm -rf /; echo 'y"), "'x'\\''; rm -rf /; echo '\\''y'");
    }

    #[test]
    fn shell_remoto_só_accepta_bash_ou_sh() {
        assert_eq!(shell_arg("bash").unwrap(), "bash");
        assert_eq!(shell_arg("sh").unwrap(), "sh");
        assert!(shell_arg("bash; id").is_err());
    }

    #[test]
    fn argumentos_remotos_sao_validados() {
        assert!(validate_remote_args(&["releases/2026-10-09T0010", "site-current"]).is_ok());
        let erro = validate_remote_args(&["rm -rf /"]).unwrap_err();
        assert!(erro.contains("rejeitado"), "{erro}");
        assert!(validate_remote_args(&["x;id"]).unwrap_err().contains("rejeitado"));
        assert!(validate_remote_args(&[""]).is_err());
        // aspas e cifrões também não passam
        assert!(validate_remote_args(&["'$HOME'"]).unwrap_err().contains("rejeitado"));
    }

    #[test]
    fn sonda_pedem_as_ferramentas_da_estrategia() {
        let script = probe_script(true, false, false, true, true);
        assert!(script.contains("command -v git"), "{script}");
        assert!(script.contains("command -v rsync"));
        assert!(script.contains("command -v bash"));
        assert!(script.contains("command -v cksum"));
        assert!(!script.contains("command -v docker"));
        let minima = probe_script(false, false, false, false, false);
        assert!(!minima.contains("command -v git"), "{minima}");
        assert!(!minima.contains("command -v cksum"), "{minima}");
    }

    #[test]
    fn leitura_da_sonda_distingue_presente_e_ausente() {
        let rows = parse_probe("bash /usr/bin/bash\ngit -\nshell 5.2.15\n");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ("bash".to_string(), Some("/usr/bin/bash".to_string())));
        assert_eq!(rows[1], ("git".to_string(), None));
    }

    #[test]
    fn comando_local_recebe_ambiente_e_roda_na_pasta_pedida() {
        let cancel = AtomicBool::new(false);
        let comando = if cfg!(windows) {
            "echo %DEVOPS_VAR%&& cd"
        } else {
            "printf '%s\\n' \"$DEVOPS_VAR\"; pwd"
        };
        let executado = run_local_with_env(
            comando,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            &[("DEVOPS_VAR".to_string(), "valor-da-variavel".to_string())],
            Duration::from_secs(30),
            &cancel,
        )
        .expect("rodar comando local");
        assert!(executado.ok(), "stderr: {}", executado.stderr);
        let saida = executado.stdout.replace('\r', "");
        let mut linhas = saida.lines().map(|l| l.trim()).filter(|l| !l.is_empty());
        // A variável chegou pelo ambiente do processo, não pela linha de comando.
        assert_eq!(linhas.next(), Some("valor-da-variavel"));
        let pasta = linhas.next().unwrap_or_default();
        assert!(
            pasta.replace('\\', "/").ends_with("src-tauri"),
            "cwd inesperado: {pasta}"
        );
    }

    #[test]
    fn comando_local_devolve_codigo_de_saida_e_falha() {
        let cancel = AtomicBool::new(false);
        let falho = run_local(
            "exit 7",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            Duration::from_secs(30),
            &cancel,
        )
        .expect("rodar comando que falha");
        assert!(!falho.ok());
        assert_eq!(falho.exit_code, Some(7));

        assert!(run_local("", std::path::Path::new("."), Duration::from_secs(5), &cancel).is_err());
        assert!(
            run_local("echo a\0b", std::path::Path::new("."), Duration::from_secs(5), &cancel).is_err()
        );
    }

    #[test]
    fn comando_local_interrompido_marca_cancelamento() {
        let cancel = AtomicBool::new(true);
        let r = run_local(
            if cfg!(windows) { "ping -n 30 127.0.0.1 >nul" } else { "sleep 30" },
            std::path::Path::new("."),
            Duration::from_secs(120),
            &cancel,
        )
        .expect("executor devolve o resultado");
        assert!(r.cancelled, "o cancelamento não foi registrado: {r:?}");
        assert_eq!(r.exit_code, None);
        assert!(r.cancelled, "o cancelamento não foi registrado: {r:?}");
    }
}
