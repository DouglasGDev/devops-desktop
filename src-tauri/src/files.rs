//! Explorador de arquivos remoto sobre o cliente `sftp` nativo do OpenSSH.
//!
//! Usar o binário do sistema preserva a compatibilidade com `~/.ssh/config`,
//! chaves, SSH Agent e `known_hosts` do usuário. A verificação de chave de
//! host é estrita (`StrictHostKeyChecking=yes`): host desconhecido ou com
//! identidade alterada faz a operação falhar — nunca aceitamos automaticamente.
//!
//! Caminhos passam por quoting próprio do cliente sftp e o argv usa argumentos
//! separados; nenhuma string é concatenada em comando shell.

use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    thread,
    time::Duration,
};

use crate::auth::AskPass;
use crate::models::{RemoteEntry, Server};
use crate::ssh_config;

const CONNECT_TIMEOUT_SECS: u64 = 20;
const BATCH_DEADLINE_SECS: u64 = 45;

/// Escapa um caminho para uso como argumento do cliente sftp.
pub fn sftp_quote(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for ch in path.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Monta o argv do `sftp`. `batch` usa `-b -` (script pela stdin, sem
/// medidor); modo interativo é usado nas transferências para que o cliente
/// emita as linhas de progresso que nós interpretamos.
///
/// Com `askpass`, a autenticação é por senha: o `BatchMode=yes` é substituído
/// pelas opções que permitem o prompt ao auxiliar temporário.
pub(crate) fn sftp_cmd(
    server: &Server,
    batch: bool,
    askpass: Option<&AskPass>,
) -> Result<Command, String> {
    if server.host.trim().is_empty() && server.ssh_alias.as_deref().unwrap_or("").trim().is_empty() {
        return Err("servidor sem host ou alias — cadastre um destino válido".into());
    }
    let mut cmd = crate::proc::command("sftp");
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
    if batch {
        cmd.arg("-b").arg("-");
    } else {
        cmd.arg("-o").arg("ProgressMeter=yes");
    }
    if let Some(idf) = &server.identity_file {
        cmd.arg("-i").arg(ssh_config::expand_tilde(idf));
    }
    match &server.ssh_alias {
        Some(alias) if !alias.trim().is_empty() => cmd.arg(alias.trim()),
        _ => {
            cmd.arg("-P").arg(server.port.to_string());
            cmd.arg(format!("{}@{}", server.username, server.host))
        }
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

fn read_pipe(mut pipe: impl Read, tx: std::sync::mpsc::Sender<Vec<u8>>) {
    let mut out = Vec::new();
    let _ = pipe.read_to_end(&mut out);
    let _ = tx.send(out);
}

/// Traduz a saída bruta do cliente sftp para uma mensagem de diagnóstico.
fn error_text(stderr: &str, stdout: &str, status: Option<i32>) -> String {
    let text = format!("{stderr} {stdout}").to_lowercase();
    if text.contains("not exist") || text.contains("no such file") {
        "o caminho remoto não existe".into()
    } else if text.contains("host key verification failed") || text.contains("host key for") {
        "chave de host desconhecida ou alterada — conecte primeiro pelo terminal para confirmar a identidade do servidor".into()
    } else if text.contains("authentication failed")
        || text.contains("no authentication methods")
        || text.contains("permission denied (")
    {
        "autenticação recusada — confira a chave, o SSH Agent ou a senha informada no prompt".into()
    } else if text.contains("permission denied") || text.contains("operation not permitted") {
        "permissão negada pelo servidor".into()
    } else if text.contains("connection refused") {
        "conexão recusada — verifique host e porta".into()
    } else if text.contains("could not resolve") || text.contains("name or service") {
        "host não resolvido (DNS)".into()
    } else if text.contains("timed out") || text.contains("timeout") {
        "tempo limite de conexão excedido".into()
    } else if text.contains("connection closed") || text.contains("broken pipe") {
        "conexão encerrada durante a operação".into()
    } else {
        let detail = stderr.trim();
        if detail.is_empty() {
            format!(
                "operação sftp falhou{}",
                status.map(|c| format!(" (código {c})")).unwrap_or_default()
            )
        } else {
            detail.to_string()
        }
    }
}

/// Executa um script batch no sftp e devolve o stdout.
fn run_batch(server: &Server, script: &str, askpass: Option<&AskPass>) -> Result<String, String> {
    let never = AtomicBool::new(false);
    run_batch_cancel(
        server,
        script,
        askpass,
        Duration::from_secs(BATCH_DEADLINE_SECS),
        &never,
    )
}

/// Igual ao `run_batch`, mas com prazo próprio e cancelamento cooperativo — é o
/// que o motor de deploy usa para enviar muitos arquivos em lotes.
fn run_batch_cancel(
    server: &Server,
    script: &str,
    askpass: Option<&AskPass>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut child = sftp_cmd(server, true, askpass)?
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "cliente sftp não encontrado no PATH (OpenSSH)".to_string()
            } else {
                format!("não foi possível iniciar o sftp: {e}")
            }
        })?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "falha ao abrir a entrada do sftp".to_string())?;
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("falha ao enviar comandos ao sftp: {e}"))?;
        stdin.flush().ok();
    }
    drop(child.stdin.take());
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        thread::spawn(move || read_pipe(pipe, tx_out));
    }
    if let Some(pipe) = child.stderr.take() {
        thread::spawn(move || read_pipe(pipe, tx_err));
    }
    let (status, timed_out, cancelled) = crate::sshexec::harvest(&mut child, timeout, cancel);
    if timed_out || cancelled {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = rx_out
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    let stderr = rx_err
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();
    if cancelled {
        return Err("envio interrompido pelo usuário".into());
    }
    if timed_out {
        return Err(format!(
            "tempo limite de {} s excedido no envio — o sftp foi encerrado",
            timeout.as_secs()
        ));
    }
    if status == Some(0) {
        Ok(stdout)
    } else {
        Err(error_text(&stderr, &stdout, status))
    }
}

/// Envia um lote de arquivos numa única sessão sftp, criando as pastas pai com
/// `-mkdir -p`. Cada destino é um caminho absoluto no servidor.
pub fn put_chunk(
    server: &Server,
    chunk: &[(PathBuf, String)],
    askpass: Option<&AskPass>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if chunk.is_empty() {
        return Ok(());
    }
    let mut script = String::new();
    for (local, remote) in chunk {
        if !local.is_file() {
            return Err(format!("o arquivo local não existe: {}", local.display()));
        }
        let parent = match remote.rsplit_once('/') {
            Some((p, _)) if !p.is_empty() => p.to_string(),
            _ => "/".to_string(),
        };
        script.push_str(&format!("-mkdir -p {}\n", sftp_quote(&parent)));
        script.push_str(&format!(
            "put {} {}\n",
            sftp_quote(&local.to_string_lossy().replace('\\', "/")),
            sftp_quote(remote)
        ));
    }
    script.push_str("bye\n");
    run_batch_cancel(server, &script, askpass, timeout, cancel).map(|_| ())
}

/// Lista um diretório remoto (nome, tipo, tamanho, permissões, dono, data).
pub fn list_dir(
    server: &Server,
    path: &str,
    askpass: Option<&AskPass>,
) -> Result<Vec<RemoteEntry>, String> {
    if path.is_empty() {
        return Err("caminho remoto inválido".into());
    }
    let script = format!("ls -la {}\n", sftp_quote(path));
    let stdout = run_batch(server, &script, askpass)?;
    Ok(parse_ls(&stdout, path))
}

pub fn make_dir(server: &Server, path: &str, askpass: Option<&AskPass>) -> Result<(), String> {
    run_batch(server, &format!("mkdir {}\n", sftp_quote(path)), askpass).map(|_| ())
}

pub fn rename(
    server: &Server,
    from: &str,
    to: &str,
    askpass: Option<&AskPass>,
) -> Result<(), String> {
    run_batch(
        server,
        &format!("rename {} {}\n", sftp_quote(from), sftp_quote(to)),
        askpass,
    )
    .map(|_| ())
}

pub fn remove_file(server: &Server, path: &str, askpass: Option<&AskPass>) -> Result<(), String> {
    run_batch(server, &format!("rm {}\n", sftp_quote(path)), askpass).map(|_| ())
}

pub fn remove_dir(server: &Server, path: &str, askpass: Option<&AskPass>) -> Result<(), String> {
    run_batch(server, &format!("rmdir {}\n", sftp_quote(path)), askpass).map(|_| ())
}

pub fn set_perms(
    server: &Server,
    path: &str,
    mode: &str,
    askpass: Option<&AskPass>,
) -> Result<(), String> {
    if mode.len() != 4 || !mode.chars().all(|c| c.is_ascii_digit()) {
        return Err("modo de permissão inválido (use 4 dígitos, ex.: 0644)".into());
    }
    run_batch(server, &format!("chmod {} {}\n", mode, sftp_quote(path)), askpass).map(|_| ())
}

/// Lê um arquivo de texto remoto baixando para um temporário local.
pub fn read_text(server: &Server, remote: &str, askpass: Option<&AskPass>) -> Result<String, String> {
    let tmp = temp_path("devops-read");
    let script = format!("get {} {}\n", sftp_quote(remote), sftp_quote(&tmp));
    let r = run_batch(server, &script, askpass);
    let bytes = match r {
        Ok(_) => std::fs::read(&tmp).map_err(|e| format!("falha ao ler o download: {e}"))?,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&tmp);
    if bytes.len() > 4_000_000 {
        return Err("arquivo grande demais para edição (limite: 4 MB)".into());
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// Grava texto no remoto com atomicidade aproximada: sobe como `.devops-tmp`
/// e renomeia por cima do destino.
pub fn write_text(
    server: &Server,
    remote: &str,
    content: &str,
    askpass: Option<&AskPass>,
) -> Result<(), String> {
    let tmp = temp_path("devops-write");
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| format!("falha ao preparar o upload: {e}"))?;
    let part = format!("{remote}.devops-tmp");
    let script = format!(
        "put {} {}\nrename {} {}\n",
        sftp_quote(&tmp),
        sftp_quote(&part),
        sftp_quote(&part),
        sftp_quote(remote)
    );
    let r = run_batch(server, &script, askpass);
    let _ = std::fs::remove_file(&tmp);
    r.map(|_| ())
}

fn temp_path(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir()
        .join(format!("{prefix}-{}-{nanos}.tmp", std::process::id()))
        .to_string_lossy()
        .replace('\\', "/")
}

// ---------- parsing ----------

fn looks_like_perms(col: &str) -> bool {
    let chars: Vec<char> = col.chars().collect();
    matches!(chars.len(), 10 | 11)
        && matches!(
            chars[0],
            'd' | '-' | 'l' | 's' | 'S' | 'c' | 'b' | 'p'
        )
}

fn month_num(name: &str) -> Option<u32> {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m.eq_ignore_ascii_case(name))
    .map(|i| i as u32 + 1)
}

fn parse_ls_date(a: &str, b: &str, c: &str) -> Option<String> {
    let month = month_num(a)?;
    let day: u32 = b.parse().ok()?;
    if c.contains(':') {
        let (h, m) = c.split_once(':')?;
        let hour: u32 = h.parse().ok()?;
        let min: u32 = m.parse().ok()?;
        // ls omite o ano em datas recentes; "mês futuro" indica ano passado.
        let (y, current_m, current_d) = now_ymd();
        let mut year = y;
        if month > current_m || (month == current_m && day > current_d) {
            year -= 1;
        }
        Some(format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:00"))
    } else {
        let year: i32 = c.parse().ok()?;
        Some(format!("{year:04}-{month:02}-{day:02}T00:00:00"))
    }
}

/// Data civil atual (UTC) via algoritmo days-to-civil de Howard Hinnant.
fn now_ymd() -> (i32, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096 / 4) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Interpreta a saída de `ls -la` (formatos GNU/BSD) emitida pelo servidor.
pub fn parse_ls(raw: &str, dir: &str) -> Vec<RemoteEntry> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("total ") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 || !looks_like_perms(cols[0]) {
            continue;
        }
        let perms = cols[0].to_string();
        let kind = perms.chars().next().unwrap_or('-');
        let owner = cols[2].to_string();
        let group = cols[3].to_string();
        let size: Option<u64> = cols[4].parse().ok();
        let modified = parse_ls_date(cols[5], cols[6], cols[7]);
        // O oitavo campo em diante é o nome — pode conter espaços.
        let mut name = cols[8..].join(" ");
        let is_symlink = kind == 'l';
        if is_symlink {
            if let Some(pos) = name.find(" -> ") {
                name.truncate(pos);
            }
        }
        let is_dir = kind == 'd' || name.ends_with('/');
        if name.ends_with('/') {
            name.pop();
        }
        // O cliente sftp ecoa o caminho completo quando o argumento é absoluto
        // (`ls -la /var/www` devolve `/var/www/html`); sem reduzir ao nome, o
        // join abaixo dobraria as barras.
        let name = match name.rsplit_once('/') {
            Some((_, leaf)) => leaf.to_string(),
            None => name,
        };
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }
        let base = dir.trim_end_matches('/');
        let path = if base.is_empty() {
            format!("/{name}")
        } else {
            format!("{base}/{name}")
        };
        entries.push(RemoteEntry {
            name,
            path,
            is_dir,
            is_symlink,
            perms: Some(perms),
            size,
            owner: Some(owner),
            group: Some(group),
            modified,
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cita_escapa_aspas_e_barra_invertida() {
        assert_eq!(sftp_quote("/tmp/a b"), "\"/tmp/a b\"");
        assert_eq!(sftp_quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn parseia_ls_gnu_com_espacos_e_link_simbolico() {
        let raw = "total 24\n\
drwxr-xr-x 5 deploy staff 4096 Oct  8 14:32 .\n\
-rw-r--r-- 1 deploy staff 1234 Jan  2  2024 notas do dia.txt\n\
lrwxrwxrwx 1 root root 11 Sep 30 09:00 link -> current/rel\nd\n\
-rwxr-xr-x 1 root root 0 Oct 01 08:00 bin\run.exe\n";
        let entries = parse_ls(raw, "/srv/app");
        assert_eq!(entries.len(), 3);
        let doc = entries.iter().find(|e| e.name == "notas do dia.txt").expect("nome com espaço");
        assert_eq!(doc.size, Some(1234));
        assert_eq!(doc.modified.as_deref(), Some("2024-01-02T00:00:00"));
        let link = entries.iter().find(|e| e.name == "link").expect("link simbólico");
        assert!(link.is_symlink);
        assert_eq!(link.path, "/srv/app/link");
        let exe = entries.iter().find(|e| e.name.contains("bin")).expect("caractere estranho");
        assert_eq!(exe.perms.as_deref(), Some("-rwxr-xr-x"));
    }

    #[test]
    fn parseia_ls_bsd_com_marca_de_diretorio() {
        let raw = "drwxr-xr-x  2 deploy  staff  512  8 Out 14:32 publica/\n\
-rw-------  1 deploy  staff  77  1 Out  2025 id_ed25519\n";
        // BSD usa "dia mês"; datas não reconhecidas viram entrada sem data,
        // nunca uma falha na listagem.
        let entries = parse_ls(raw, "/home/deploy");
        assert_eq!(entries.len(), 2);
        let dir = entries.iter().find(|e| e.name == "publica").expect("diretório");
        assert!(dir.is_dir);
        assert_eq!(dir.path, "/home/deploy/publica");
        assert_eq!(dir.modified, None);
        assert_eq!(dir.size, Some(512));
    }

    #[test]
    fn ordena_diretorios_antes_de_arquivos() {
        let raw = "total 8\n\
-rw-r--r-- 1 u g 10 Jan 1 10:00 a.txt\n\
drwxr-xr-x 2 u g 4096 Jan 1 10:00 z-dir\n";
        let entries = parse_ls(raw, "/x");
        assert_eq!(entries[0].name, "z-dir");
        assert_eq!(entries[1].name, "a.txt");
        assert_eq!(entries[0].path, "/x/z-dir");
    }

    #[test]
    fn parseia_ls_com_caminho_absoluto_devolvido_pelo_sftp() {
        // `sftp ls -la /var/www` imprime `/var/www/html`, não `html`. Sem
        // reduzir ao nome, o join produzia /var/www//var/www/html.
        let raw = "drwxr-xr-x 5 root root 4096 Oct  8 14:32 /var/www/html\n\
-rw-r--r-- 1 root root 1234 Oct  8 14:32 /var/www/index.php\n";
        let entries = parse_ls(raw, "/var/www");
        assert_eq!(entries.len(), 2);
        let dir = entries.iter().find(|e| e.name == "html").expect("nome reduzido");
        assert!(dir.is_dir);
        assert_eq!(dir.path, "/var/www/html");
        assert_eq!(entries[1].path, "/var/www/index.php");
        assert!(!entries.iter().any(|e| e.path.contains("//")), "barra dobrada");
    }

    #[test]
    fn parseia_ls_da_raiz_sem_dobrar_barra() {
        let raw = "drwxr-xr-x 5 root root 4096 Oct  8 14:32 usr\n\
	lrwxrwxrwx 1 root root 11 Oct  8 14:32 tmp -> /var/tmp/\n";
        let entries = parse_ls(raw, "/");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/usr");
        assert_eq!(entries[1].path, "/tmp");
    }

    #[test]
    fn temp_path_usa_barra_normal() {
        assert!(!temp_path("t").contains('\\'));
        assert!(temp_path("t").ends_with(".tmp"));
    }

    #[test]
    fn modo_invalido_e_rejeitado() {
        assert!(set_perms_allowed("755").is_err());
        assert!(set_perms_allowed("0644").is_ok());
        assert!(set_perms_allowed("06ab").is_err());
    }

    fn set_perms_allowed(mode: &str) -> Result<(), String> {
        if mode.len() != 4 || !mode.chars().all(|c| c.is_ascii_digit()) {
            return Err("modo de permissão inválido (use 4 dígitos, ex.: 0644)".into());
        }
        Ok(())
    }

    fn server(auth_method: &str) -> Server {
        Server {
            id: 1,
            name: "web".into(),
            host: "example.invalid".into(),
            port: 2222,
            username: "deploy".into(),
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

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect()
    }

    #[test]
    fn modo_chave_usa_batchmode() {
        let cmd = sftp_cmd(&server("key"), true, None).unwrap();
        let argv = args(&cmd);
        assert!(argv.contains(&"BatchMode=yes".to_string()));
        assert!(argv.contains(&"StrictHostKeyChecking=yes".to_string()));
        assert!(!cmd.get_envs().any(|(k, _)| k == "SSH_ASKPASS"));
    }

    #[test]
    fn modo_senha_troca_batchmode_pelo_askpass() {
        let pass = crate::auth::AskPass::new("senha-de-teste").unwrap();
        let cmd = sftp_cmd(&server("password"), true, Some(&pass)).unwrap();
        let argv = args(&cmd);
        assert!(!argv.contains(&"BatchMode=yes".to_string()), "senha não pode bloquear o prompt");
        assert!(argv.contains(&"BatchMode=no".to_string()));
        assert!(argv.contains(&"NumberOfPasswordPrompts=1".to_string()));
        // A checagem de host continua estrita em qualquer modo de autenticação.
        assert!(argv.contains(&"StrictHostKeyChecking=yes".to_string()));
        assert!(cmd.get_envs().any(|(k, v)| k == "SSH_ASKPASS" && v == Some(pass.script_path().as_os_str())));
    }

    #[test]
    fn servidor_sem_host_ou_alias_e_erro() {
        let mut s = server("password");
        s.host = " ".into();
        assert!(sftp_cmd(&s, true, None).is_err());
    }
}
