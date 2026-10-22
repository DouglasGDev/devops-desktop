use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::AtomicBool,
    time::Duration,
};

use crate::models::{AgentInfo, AgentKey, KeyFile, KnownHostsInfo, SshDiscoveryReport, SshHostCandidate};

const NON_KEY_FILES: &[&str] = &[
    "config", "config.old", "known_hosts", "known_hosts.old", "known_hosts2", "authorized_keys",
    "authorized_keys2", "environment", "rc", "ssh_config", "sshd_config",
];

pub fn home_ssh_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh"))
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn has_wildcard(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('!')
}

#[derive(Default)]
struct CandidateBuilder {
    patterns: Vec<String>,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<i64>,
    identity_files: Vec<String>,
    proxy_jump: Option<String>,
    source_file: String,
}

impl CandidateBuilder {
    fn into_candidate(self) -> Option<SshHostCandidate> {
        let concrete = self
            .patterns
            .iter()
            .find(|p| !has_wildcard(p))?
            .clone();
        Some(SshHostCandidate {
            is_concrete: true,
            alias: concrete,
            patterns: self.patterns,
            host_name: self.host_name,
            user: self.user,
            port: self.port,
            identity_files: self.identity_files,
            proxy_jump: self.proxy_jump,
            source_file: self.source_file,
        })
    }
}

struct ParserState {
    hosts: Vec<SshHostCandidate>,
    config_files: Vec<String>,
    warnings: Vec<String>,
    visited: HashSet<PathBuf>,
}

/// Lê um arquivo ssh_config. Semânticas seguidas do OpenSSH: primeira ocorrência
/// de HostName/User/Port/ProxyJump vence; IdentityFile acumula; Include aceita
/// múltiplos caminhos, globs e caminhos relativos ao arquivo que inclui.
fn parse_config_file(path: &Path, state: &mut ParserState, depth: usize) {
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) => {
            state.warnings.push(format!("não foi possível ler {}: {e}", path.display()));
            return;
        }
    };
    if state.visited.contains(&canonical) {
        return; // proteção contra ciclos de Include
    }
    state.visited.insert(canonical.clone());
    state.config_files.push(canonical.display().to_string());

    if depth > 16 {
        state.warnings.push(format!("limite de profundidade de Include excedido em {}", path.display()));
        return;
    }

    let content = match fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(e) => {
            state.warnings.push(format!("não foi possível ler {}: {e}", canonical.display()));
            return;
        }
    };

    let mut current: Option<CandidateBuilder> = None;
    let mut skip_until_next_host = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword_raw, value_raw) = match line.split_once('=') {
            Some((k, v)) if !k.contains(char::is_whitespace) && !v.is_empty() => {
                (k.trim().to_string(), v.trim().to_string())
            }
            _ => match line.split_once(char::is_whitespace) {
                Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                None => (line.to_string(), String::new()),
            },
        };
        let keyword = keyword_raw.to_lowercase();
        let value = value_raw.trim().to_string();
        if value.is_empty() {
            continue;
        }

        match keyword.as_str() {
            "host" => {
                skip_until_next_host = false;
                if let Some(b) = current.take() {
                    if let Some(candidate) = b.into_candidate() {
                        merge_candidate(&mut state.hosts, candidate);
                    }
                }
                let patterns: Vec<String> = value.split_whitespace().map(|s| s.to_string()).collect();
                current = Some(CandidateBuilder {
                    patterns,
                    source_file: canonical.display().to_string(),
                    ..Default::default()
                });
            }
            "match" => {
                // Blocos Match são condicionais ao momento da conexão; os hosts que
                // os seguem não são interpretados para evitar cadastros incorretos.
                if let Some(b) = current.take() {
                    if let Some(candidate) = b.into_candidate() {
                        merge_candidate(&mut state.hosts, candidate);
                    }
                }
                skip_until_next_host = true;
            }
            "include" => {
                for pattern in value.split_whitespace() {
                    let expanded = expand_tilde(pattern);
                    let base = if expanded.is_absolute() {
                        expanded
                    } else {
                        canonical.parent().unwrap_or(Path::new(".")).join(expanded)
                    };
                    let matches = match glob::glob(&base.to_string_lossy()) {
                        Ok(g) => g.filter_map(Result::ok).collect::<Vec<PathBuf>>(),
                        Err(_) => vec![base.clone()],
                    };
                    if matches.is_empty() {
                        state.warnings.push(format!("Include sem correspondência: {pattern} (em {})", canonical.display()));
                    }
                    for m in matches {
                        parse_config_file(&m, state, depth + 1);
                    }
                }
            }
            _ if skip_until_next_host => {}
            "hostname" => {
                if let Some(b) = current.as_mut() {
                    if b.host_name.is_none() {
                        b.host_name = Some(value);
                    }
                }
            }
            "user" => {
                if let Some(b) = current.as_mut() {
                    if b.user.is_none() {
                        b.user = Some(value);
                    }
                }
            }
            "port" => {
                if let Some(b) = current.as_mut() {
                    if b.port.is_none() {
                        b.port = value.parse::<i64>().ok();
                    }
                }
            }
            "identityfile" => {
                if let Some(b) = current.as_mut() {
                    b.identity_files.push(expand_tilde(&value).display().to_string());
                }
            }
            "proxyjump" => {
                if let Some(b) = current.as_mut() {
                    if b.proxy_jump.is_none() {
                        b.proxy_jump = Some(value);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(b) = current.take() {
        if let Some(candidate) = b.into_candidate() {
            merge_candidate(&mut state.hosts, candidate);
        }
    }
}

fn merge_candidate(hosts: &mut Vec<SshHostCandidate>, candidate: SshHostCandidate) {
    if let Some(existing) = hosts.iter_mut().find(|h| h.alias == candidate.alias) {
        if existing.host_name.is_none() {
            existing.host_name = candidate.host_name.clone();
        }
        if existing.user.is_none() {
            existing.user = candidate.user.clone();
        }
        if existing.port.is_none() {
            existing.port = candidate.port;
        }
        if existing.proxy_jump.is_none() {
            existing.proxy_jump = candidate.proxy_jump.clone();
        }
        for idf in candidate.identity_files {
            if !existing.identity_files.contains(&idf) {
                existing.identity_files.push(idf);
            }
        }
        existing.patterns = existing
            .patterns
            .iter()
            .chain(candidate.patterns.iter())
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
    } else {
        hosts.push(candidate);
    }
}

fn read_key_type(first_line: &str) -> Option<String> {
    // Linha pública: "<tipo> <base64> <comentário>". Lê apenas o primeiro token,
    // sem tocar no material da chave.
    let token = first_line.split_whitespace().next()?;
    if token.starts_with("ssh-") || token.starts_with("ecdsa-") || token.starts_with("sk-") {
        Some(token.to_string())
    } else {
        None
    }
}

fn scan_key_files(ssh_dir: &Path) -> (Vec<KeyFile>, Vec<KeyFile>) {
    let mut private_keys = Vec::new();
    let mut public_keys = Vec::new();
    let entries = match fs::read_dir(ssh_dir) {
        Ok(e) => e,
        Err(_) => return (private_keys, public_keys),
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if NON_KEY_FILES.iter().any(|f| file_name.eq_ignore_ascii_case(f)) {
            continue;
        }
        if file_name.ends_with(".pub") {
            let key_type = fs::read_to_string(&path)
                .ok()
                .and_then(|c| c.lines().next().and_then(read_key_type));
            let base = path.with_extension("");
            public_keys.push(KeyFile {
                path: path.display().to_string(),
                kind: "public".into(),
                key_type,
                has_pair: base.is_file(),
                encrypted: None,
            });
        } else {
            // Lê apenas as primeiras linhas para detectar formato/criptografia; o
            // material da chave nunca sai do arquivo original.
            let header = fs::File::open(&path)
                .ok()
                .map(|mut f| {
                    use std::io::Read;
                    let mut buf = [0u8; 256];
                    let n = f.read(&mut buf).unwrap_or(0);
                    String::from_utf8_lossy(&buf[..n]).to_string()
                });
            let (key_type, encrypted) = match header {
                Some(h) => {
                    let first = h.lines().next().unwrap_or("");
                    let is_openssh = first.contains("BEGIN OPENSSH PRIVATE KEY");
                    let is_pem = first.contains("BEGIN RSA PRIVATE KEY")
                        || first.contains("BEGIN EC PRIVATE KEY")
                        || first.contains("BEGIN DSA PRIVATE KEY");
                    let key_type = if is_openssh {
                        Some("openssh".to_string())
                    } else if is_pem {
                        first
                            .split_whitespace()
                            .nth(1)
                            .map(|s| s.to_lowercase())
                    } else {
                        None
                    };
                    (key_type, h.contains("ENCRYPTED"))
                }
                None => (None, false),
            };
            private_keys.push(KeyFile {
                has_pair: PathBuf::from(format!("{}.pub", path.display())).is_file(),
                path: path.display().to_string(),
                kind: "private".into(),
                key_type,
                encrypted: if encrypted { Some(true) } else { None },
            });
        }
    }
    (private_keys, public_keys)
}

fn read_known_hosts(ssh_dir: &Path) -> Option<KnownHostsInfo> {
    let path = ssh_dir.join("known_hosts");
    if !path.is_file() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    let mut total = 0i64;
    let mut hashed = 0i64;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        total += 1;
        if line.starts_with("|1|") {
            hashed += 1;
        }
    }
    Some(KnownHostsInfo {
        path: path.display().to_string(),
        total,
        hashed,
    })
}

/// Prazo da leitura do agente. Um `ssh-add` esperando a confirmação de um token não
/// volta nunca, e a descoberta de chaves deixaria a tela presa; como em toda sonda
/// desta máquina, "não respondeu" é um estado próprio — nem "não está no PATH", nem
/// "agente vivo sem chaves".
const PRAZO_AGENTE: Duration = Duration::from_secs(5);

enum Sonda {
    /// Rodou e devolveu um código de saída.
    Respondeu { code: Option<i32>, stdout: String },
    /// Não deu para executar: o nome não está no PATH, ou o sistema recusou.
    Ausente,
    /// Rodou e não terminou dentro do prazo. Traz o motivo com a duração lida.
    SemResposta { motivo: String },
}

fn sonda(programa: &str, args: &[&str], prazo: Duration) -> Sonda {
    let mut child = match crate::proc::command(programa)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Sonda::Ausente,
    };
    let feito = crate::sshexec::collect(&mut child, prazo, &AtomicBool::new(false));
    if feito.timed_out || feito.cancelled {
        return Sonda::SemResposta {
            motivo: format!("não respondeu em {}", crate::sshexec::texto_duracao(prazo)),
        };
    }
    Sonda::Respondeu { code: feito.exit_code, stdout: feito.stdout }
}

fn read_agent() -> AgentInfo {
    veredito_agente(sonda("ssh-add", &["-l"], PRAZO_AGENTE))
}

/// O que a tela vê, a partir do que a sondagem respondeu. Nenhum ramo aqui afirma
/// que o agente existe sem que ele tenha dito.
fn veredito_agente(leitura: Sonda) -> AgentInfo {
    match leitura {
        Sonda::Ausente => AgentInfo {
            available: false,
            keys: Vec::new(),
            note: Some("ssh-add não encontrado no PATH.".into()),
        },
        Sonda::SemResposta { motivo } => AgentInfo {
            available: false,
            keys: Vec::new(),
            note: Some(format!(
                "o ssh-add {motivo} e a leitura do agente foi abandonada."
            )),
        },
        Sonda::Respondeu { code: Some(0), stdout } => AgentInfo {
            available: true,
            keys: stdout.lines().filter_map(parse_agent_line).collect(),
            note: None,
        },
        Sonda::Respondeu { code: Some(1), .. } => AgentInfo {
            available: true,
            keys: Vec::new(),
            note: Some("Agente em execução, sem chaves carregadas.".into()),
        },
        Sonda::Respondeu { .. } => AgentInfo {
            available: false,
            keys: Vec::new(),
            note: Some("SSH Agent não está em execução (ou não há socket disponível).".into()),
        },
    }
}

fn parse_agent_line(line: &str) -> Option<AgentKey> {
    // Formato: "<bits> SHA256:<hash> <comentário opcional> (<tipo>)"
    let mut parts = line.split_whitespace();
    let _bits = parts.next()?;
    let fingerprint = parts.next()?.to_string();
    if !fingerprint.starts_with("SHA256:") {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    let key_type = rest
        .iter()
        .rev()
        .find(|s| s.starts_with('(') && s.ends_with(')'))
        .map(|s| s[1..s.len() - 1].to_string())
        .unwrap_or_else(|| "desconhecido".into());
    let comment_end = rest
        .iter()
        .position(|s| s.starts_with('(') && s.ends_with(')'))
        .unwrap_or(rest.len());
    let comment = rest[..comment_end].join(" ");
    Some(AgentKey {
        fingerprint,
        key_type,
        comment: if comment.is_empty() { None } else { Some(comment) },
    })
}

pub fn discover() -> SshDiscoveryReport {
    let mut state = ParserState {
        hosts: Vec::new(),
        config_files: Vec::new(),
        warnings: Vec::new(),
        visited: HashSet::new(),
    };

    let ssh_dir = home_ssh_dir();
    let config_path = ssh_dir.as_ref().map(|d| d.join("config"));

    match (&ssh_dir, &config_path) {
        (Some(dir), Some(cfg)) if dir.is_dir() => {
            if cfg.is_file() {
                parse_config_file(cfg, &mut state, 0);
            } else {
                state
                    .warnings
                    .push("Nenhum arquivo ~/.ssh/config encontrado — cadastro manual disponível.".into());
            }
        }
        _ => {
            state
                .warnings
                .push("Diretório ~/.ssh não encontrado neste computador.".into());
        }
    }

    let (private_keys, public_keys) = ssh_dir
        .as_ref()
        .filter(|d: &&PathBuf| d.is_dir())
        .map(|d: &PathBuf| scan_key_files(d))
        .unwrap_or((Vec::new(), Vec::new()));

    let known_hosts = ssh_dir
        .as_ref()
        .filter(|d: &&PathBuf| d.is_dir())
        .and_then(|d: &PathBuf| read_known_hosts(d));
    let agent = read_agent();

    state.hosts.sort_by(|a, b| a.alias.cmp(&b.alias));
    state.config_files.sort();
    state.config_files.dedup();

    SshDiscoveryReport {
        config_files: state.config_files,
        hosts: state.hosts,
        public_keys,
        private_keys,
        known_hosts,
        agent,
        warnings: state.warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_basico_host_hostname() {
        let dir = std::env::temp_dir().join(format!("ddt-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        fs::write(&cfg, "# comentario\nHost web-prod\n  HostName 203.0.113.10\n  User deploy\n  Port 2222\n  IdentityFile ~/.ssh/id_ed25519\n").unwrap();

        let mut state = ParserState {
            hosts: Vec::new(),
            config_files: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
        };
        parse_config_file(&cfg, &mut state, 0);
        assert_eq!(state.hosts.len(), 1);
        let host = &state.hosts[0];
        assert_eq!(host.alias, "web-prod");
        assert_eq!(host.host_name.as_deref(), Some("203.0.113.10"));
        assert_eq!(host.user.as_deref(), Some("deploy"));
        assert_eq!(host.port, Some(2222));
        assert!(host.is_concrete);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignora_hosts_com_curinga() {
        let dir = std::env::temp_dir().join(format!("ddt-w-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        fs::write(&cfg, "Host *\n  User fallback\n\nHost web1\n  HostName 1.2.3.4\n").unwrap();

        let mut state = ParserState {
            hosts: Vec::new(),
            config_files: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
        };
        parse_config_file(&cfg, &mut state, 0);
        assert_eq!(state.hosts.len(), 1);
        assert_eq!(state.hosts[0].alias, "web1");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn include_relativo_e_glob() {
        let dir = std::env::temp_dir().join(format!("ddt-i-{}", std::process::id()));
        fs::create_dir_all(dir.join("conf.d")).unwrap();
        fs::write(dir.join("config"), "Include conf.d/*.conf\n").unwrap();
        fs::write(dir.join("conf.d/a.conf"), "Host incluido\n  HostName 5.6.7.8\n").unwrap();

        let mut state = ParserState {
            hosts: Vec::new(),
            config_files: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
        };
        parse_config_file(&dir.join("config"), &mut state, 0);
        assert_eq!(state.hosts.len(), 1);
        assert_eq!(state.hosts[0].alias, "incluido");
        assert_eq!(state.config_files.len(), 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn primeira_ocorrencia_vence() {
        let dir = std::env::temp_dir().join(format!("ddt-f-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        fs::write(
            &cfg,
            "Host srv\n  HostName primeiro.example\n\nHost srv\n  HostName segundo.example\n",
        )
        .unwrap();
        let mut state = ParserState {
            hosts: Vec::new(),
            config_files: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
        };
        parse_config_file(&cfg, &mut state, 0);
        assert_eq!(state.hosts.len(), 1);
        assert_eq!(state.hosts[0].host_name.as_deref(), Some("primeiro.example"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ciclo_de_include_nao_trava() {
        let dir = std::env::temp_dir().join(format!("ddt-c-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        fs::write(&cfg, "Include config\nHost a\n  HostName 1.1.1.1\n").unwrap();
        let mut state = ParserState {
            hosts: Vec::new(),
            config_files: Vec::new(),
            warnings: Vec::new(),
            visited: HashSet::new(),
        };
        parse_config_file(&cfg, &mut state, 0);
        assert_eq!(state.hosts.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_linha_agente() {
        let key = parse_agent_line("3072 SHA256:AbCdEf user@host (RSA)").unwrap();
        assert_eq!(key.fingerprint, "SHA256:AbCdEf");
        assert_eq!(key.key_type, "RSA");
        assert_eq!(key.comment.as_deref(), Some("user@host"));
    }

    #[test]
    fn os_quatro_vereditos_do_agente_sao_coisas_diferentes() {
        // Quem não está no PATH, quem não respondeu, quem respondeu "sem chaves" e
        // quem respondeu com chaves: quatro telas diferentes, e nenhuma é a outra.
        let ausente = veredito_agente(Sonda::Ausente);
        assert!(!ausente.available);
        assert!(ausente.note.as_deref().unwrap_or("").contains("não encontrado no PATH"));

        let mudo = veredito_agente(Sonda::SemResposta { motivo: "não respondeu em 5 s".into() });
        assert!(!mudo.available);
        let nota = mudo.note.expect("a tela precisa dizer por que parou");
        assert!(nota.contains("não respondeu em 5 s"), "{nota}");
        assert!(!nota.contains("não encontrado"), "{nota}");

        let vazio = veredito_agente(Sonda::Respondeu { code: Some(1), stdout: String::new() });
        assert!(vazio.available, "o agente respondeu que está vivo, sem chaves");
        assert!(vazio.keys.is_empty());

        let cheio = veredito_agente(Sonda::Respondeu {
            code: Some(0),
            stdout: "3072 SHA256:AbCdEf user@host (RSA)\n".into(),
        });
        assert!(cheio.available);
        assert_eq!(cheio.keys.len(), 1);
        assert!(cheio.note.is_none());

        let recusa = veredito_agente(Sonda::Respondeu { code: Some(2), stdout: String::new() });
        assert!(!recusa.available);
        assert!(recusa.note.as_deref().unwrap_or("").contains("não está em execução"));
    }

    #[test]
    fn uma_sonda_de_agente_que_nao_termina_e_abandonada_dentro_do_prazo() {
        // O caso que prendia a descoberta de chaves: um processo vivo muito além do
        // prazo. A técnica é a da sonda de ferramentas — um programa real que demora,
        // não um prazo inventado.
        let (programa, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "40", "127.0.0.1"])
        } else {
            ("sleep", &["40"])
        };
        let inicio = std::time::Instant::now();
        let info = veredito_agente(sonda(programa, args, Duration::from_secs(1)));
        let decorrido = inicio.elapsed();
        assert!(decorrido < Duration::from_secs(10), "esperou o programa inteiro: {decorrido:?}");
        assert!(!info.available);
        let nota = info.note.expect("sem resposta tem de dizer o motivo");
        assert!(nota.contains("não respondeu em 1 s"), "{nota}");
    }

    #[test]
    fn a_leitura_real_do_agente_volta_com_um_veredito() {
        // Nesta máquina o ssh-add responde em dezenas de milissegundos com código 2
        // (o serviço está parado): a chamada volta, e o que a tela mostra é o que ele
        // respondeu — nunca uma espera aberta.
        let inicio = std::time::Instant::now();
        let info = read_agent();
        let decorrido = inicio.elapsed();
        assert!(decorrido < Duration::from_secs(20), "a leitura esperou sem prazo: {decorrido:?}");
        if !info.available {
            assert!(info.note.is_some(), "agente indisponível sem dizer o motivo");
        }
        assert!(
            !info.note.as_deref().unwrap_or("").contains("não existe"),
            "silêncio chamado de ausência"
        );
    }
}
