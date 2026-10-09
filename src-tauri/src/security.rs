use std::{fs, path::Path};

use serde::Serialize;

use crate::git;

/// Limite de leitura para a checagem de conteúdo. Arquivos maiores são
/// julgados apenas pelo nome.
const MAX_CONTENT_BYTES: u64 = 2 * 1024 * 1024;
const PROBE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub path: String,
    /// "nome", "extensao", "conteudo", "rastreado" ou "ignorado".
    pub kind: String,
    pub reason: String,
    /// "block" impede a operação até o usuário decidir; "warn" apenas alerta.
    pub severity: String,
    pub suggestion: String,
}

const BLOCK_NAMES: [&str; 22] = [
    ".env",
    ".environment",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "known_hosts",
    "credentials",
    "credentials.json",
    "service-account.json",
    ".npmrc",
    ".netrc",
    ".pgpass",
    ".htpasswd",
    "secrets.yml",
    "secrets.yaml",
    "secrets.toml",
    "settings.local.json",
    "config.local.php",
    "wp-config.php",
    "database.yml",
    "session_secret",
];

const BLOCK_EXTENSIONS: [&str; 17] = [
    "key", "pem", "p8", "p12", "pfx", "jks", "keystore", "ks", "ppk", "kdbx", "pwd", "crt.bak", "kdb", "asc", "gpg",
    "snk", "pypirc",
];

/// Pastas de cache, dependências e build: não devem estar no índice do Git.
const NOISE_DIRS: [&str; 12] = [
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".cache",
    "coverage",
    ".terraform",
];

fn lower(name: &str) -> String {
    name.to_lowercase()
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Regras extras configuráveis: um padrão por linha, batido no nome do arquivo.
pub fn extra_patterns(raw: Option<&str>) -> Vec<String> {
    raw.map(|text| {
        text.lines()
            .map(|l| lower(l.trim()))
            .filter(|l| !l.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn finding(rel: &str, kind: &str, reason: String, suggestion: &str) -> Finding {
    Finding {
        path: rel.to_string(),
        kind: kind.to_string(),
        reason,
        severity: "block".to_string(),
        suggestion: suggestion.to_string(),
    }
}

fn name_finding(rel: &str, extra: &[String]) -> Option<Finding> {
    let file = lower(basename(rel));
    if let Some(name) = BLOCK_NAMES
        .iter()
        .find(|name| file == **name || file.starts_with(&format!("{name}.")) || file.ends_with(&format!(".{name}")))
    {
        return Some(finding(
            rel,
            "nome",
            format!("“{file}” é um nome associado a credenciais ou chaves (regra “{name}”)"),
            "não envie este arquivo; mantenha-o fora do Git e use um exemplo sem valores",
        ));
    }
    if let Some(pattern) = extra.iter().find(|p| file.contains(p.as_str())) {
        return Some(finding(
            rel,
            "nome",
            format!("o nome do arquivo corresponde à regra configurada “{pattern}”"),
            "revise a regra; se o arquivo guarda segredo, mantenha-o fora do repositório",
        ));
    }
    let ext = file.rsplit('.').next().unwrap_or("");
    if file.contains('.') && BLOCK_EXTENSIONS.contains(&ext) {
        return Some(finding(
            rel,
            "extensao",
            format!("a extensão .{ext} é usada para chaves e material criptográfico"),
            "confirme o conteúdo; se for uma chave privada, remova do índice com “git rm --cached”",
        ));
    }
    None
}

/// Detecta um valor atribuindo a uma chave de credencial (`token = "abc"`).
/// Feito à mão para não depender de regex: procuramos a chave, o `=`, e uma
/// sequência seguinte que pareça valor real (sem espaços, com 6+ caracteres).
fn looks_like_assignment(line: &str) -> Option<&'static str> {
    const KEYS: [&str; 7] = ["password", "passwd", "secret", "api_key", "apikey", "access_key", "auth_token"];
    let low = line.to_lowercase();
    for key in KEYS {
        let at = low.find(key)?;
        let after = low[at + key.len()..].trim_start();
        if !after.starts_with('=') && !after.starts_with(':') {
            continue;
        }
        let value = after[1..].trim().trim_start_matches(['"', '\'']);
        let meaningful: String = value
            .chars()
            .take_while(|c| !matches!(c, ' ' | '#' | ';' | ',' | '"' | '\''))
            .collect();
        let letters = meaningful.chars().filter(|c| !c.is_whitespace()).count();
        if letters >= 6 && !meaningful.starts_with('$') && !meaningful.starts_with("${") {
            return Some(key);
        }
        return None;
    }
    None
}

fn content_findings(rel: &str, bytes: &[u8]) -> Vec<Finding> {
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<Finding> = Vec::new();
    let mut push = |kind: &str, reason: String, suggestion: String| {
        if !out.iter().any(|f| f.reason == reason) {
            out.push(Finding {
                path: rel.to_string(),
                kind: kind.into(),
                reason,
                severity: "block".into(),
                suggestion,
            });
        }
    };

    if text.contains("PRIVATE KEY-----") || text.contains("BEGIN OPENSSH KEY") {
        push(
            "conteudo",
            "contém bloco de chave privada".into(),
            "chave privada não deve ser versionada; use o arquivo em ~/.ssh e referencie só o caminho".into(),
        );
    }
    for line in text.lines().take(4000) {
        if let Some(key) = line.find("AKIA") {
            let tail: String = line[key + 4..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).collect();
            if tail.len() >= 16 {
                push(
                    "conteudo",
                    "contém uma chave de acesso AWS (AKIA…)".into(),
                    "revogue a chave no provedor e substitua por uma referência de ambiente".into(),
                );
            }
        }
        for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "github_pat_", "xoxb-", "xoxp-"] {
            if let Some(at) = line.find(prefix) {
                let tail: String = line[at + prefix.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                if tail.len() >= 12 {
                    push(
                        "conteudo",
                        format!("contém um token com o prefixo {prefix}"),
                        "revogue o token e use um segredo gerenciado fora do repositório".into(),
                    );
                }
            }
        }
        if let Some(key) = looks_like_assignment(line) {
            push(
                "conteudo",
                format!("atribui um valor a “{key}” no arquivo"),
                "mov o valor para uma variável de ambiente ou arquivo fora do repositório".into(),
            );
        }
    }
    out
}

fn is_probably_text(head: &[u8]) -> bool {
    !head.contains(&0)
}

fn push_content(out: &mut Vec<Finding>, rel: &str, bytes: &[u8]) {
    for f in content_findings(rel, bytes) {
        if !out.iter().any(|e| e.reason == f.reason) {
            out.push(f);
        }
    }
}

/// Analisa conteúdo que já está em memória, com as mesmas regras do arquivo do
/// disco. É o que a checagem de commit usa: os bytes do índice, não os do
/// trabalho — o que vai ser publicado é o índice.
pub fn inspect_bytes(rel: &str, bytes: &[u8], extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(f) = name_finding(rel, extra) {
        out.push(f);
    }
    if bytes.len() as u64 > MAX_CONTENT_BYTES {
        return out;
    }
    let head = &bytes[..bytes.len().min(PROBE_BYTES)];
    if !is_probably_text(head) {
        return out;
    }
    push_content(&mut out, rel, bytes);
    out
}

/// Analisa um arquivo do disco. `rel` é o caminho relativo mostrado ao usuário.
pub fn inspect_file(abs: &Path, rel: &str, extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(f) = name_finding(rel, extra) {
        out.push(f);
        // nome já bloqueia; a leitura do conteúdo ainda é útil quando o arquivo é pequeno
    }
    let meta = match fs::metadata(abs) {
        Ok(m) => m,
        Err(_) => return out,
    };
    if !meta.is_file() || meta.len() > MAX_CONTENT_BYTES {
        return out;
    }
    let Ok(data) = fs::read(abs) else { return out };
    let head = &data[..data.len().min(PROBE_BYTES)];
    if !is_probably_text(head) {
        return out;
    }
    push_content(&mut out, rel, &data);
    out
}

/// Varre uma árvore local, pulando dependências, caches e o próprio .git.
pub fn scan_tree(root: &Path, extra: &[String], max_entries: usize) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => name.clone(),
            };
            let is_dir = path.is_dir();
            if is_dir {
                let base = lower(basename(&rel));
                if base == ".git" || NOISE_DIRS.contains(&base.as_str()) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            visited += 1;
            if visited > max_entries {
                return Ok(findings);
            }
            findings.extend(inspect_file(&path, &rel, extra));
        }
    }
    Ok(findings)
}

/// Checa exatamente os caminhos que serão enviados por um deploy.
pub fn check_paths(root: &Path, rels: &[String], extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for rel in rels {
        let abs = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if abs.is_dir() {
            if let Ok(found) = scan_tree(&abs, extra, 4000) {
                out.extend(found.into_iter().map(|mut f| {
                    f.path = format!("{rel}/{}", f.path);
                    f
                }));
            }
        } else if abs.is_file() {
            out.extend(inspect_file(&abs, rel, extra));
        }
    }
    out
}

/// Arquivos sensíveis já rastreados pelo Git. Estar no `.gitignore` não remove
/// o arquivo do índice nem do histórico: é exatamente isso que avisamos aqui.
pub fn tracked_sensitive(dir: &Path, extra: &[String]) -> Result<Vec<Finding>, String> {
    Ok(tracked_findings(&git::tracked_files(dir)?, extra))
}

/// Versão pura da checagem do índice, para poder ser testada sem Git.
pub fn tracked_findings(tracked: &[String], extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for rel in tracked {
        let normalised = rel.replace('\\', "/");
        if let Some(by_name) = name_finding(&normalised, extra) {
            out.push(Finding {
                path: normalised.clone(),
                kind: "rastreado".into(),
                reason: format!(
                    "o Git já rastreia este arquivo — {}",
                    by_name.reason.to_lowercase()
                ),
                severity: "block".into(),
                suggestion:
                    "adicione ao .gitignore e remova do índice com “git rm --cached”; o histórico só muda com reescrita explícita"
                        .into(),
            });
            continue;
        }
        let top = normalised.split('/').next().unwrap_or(&normalised).to_string();
        if NOISE_DIRS.contains(&top.as_str()) {
            out.push(Finding {
                path: normalised.clone(),
                kind: "rastreado".into(),
                reason: format!("a pasta de cache/dependência “{top}” está no índice do Git"),
                severity: "warn".into(),
                suggestion: "remova do índice e liste no .gitignore".into(),
            });
        }
    }
    out
}

/// Segredos no conteúdo do índice. Um caminho que só existe como exclusão não
/// tem blob no índice e é ignorado — remover um segredo do repositório é justo
/// o que recomendamos, não um motivo para bloquear.
pub fn index_findings(dir: &Path, extra: &[String]) -> Result<Vec<Finding>, String> {
    let mut out = Vec::new();
    for rel in git::staged_files(dir)? {
        if let Some(blob) = git::index_blob(dir, &rel) {
            out.extend(inspect_bytes(&rel.replace('\\', "/"), &blob, extra));
        }
    }
    Ok(summarize(out))
}

/// Segredos no conteúdo que um push publicaria: os caminhos escolhidos lidos na
/// revisão HEAD, que é a versão que seguiria para o remoto.
pub fn head_findings(dir: &Path, rels: &[String], extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for rel in rels {
        if let Some(blob) = git::blob_at(dir, "HEAD", rel) {
            out.extend(inspect_bytes(&rel.replace('\\', "/"), &blob, extra));
        }
    }
    summarize(out)
}

/// O que a etapa de verificação do pipeline e o commit precisam saber:
/// achados por caminho, já deduplicados e ordenados.
pub fn summarize(mut findings: Vec<Finding>) -> Vec<Finding> {
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.kind.cmp(&b.kind)));
    findings.dedup_by(|a, b| a.path == b.path && a.reason == b.reason);
    findings
}

pub fn blocks(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == "block")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devops-secret-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nome_de_arquivo_de_credencial_e_bloqueado() {
        let extra: Vec<String> = Vec::new();
        let f = name_finding("srv/.env", &extra).unwrap();
        assert_eq!(f.severity, "block");
        assert!(f.reason.contains(".env"), "{}", f.reason);
        assert!(name_finding("src/.env.example", &extra).is_some());
        assert!(name_finding("README.md", &extra).is_none());
        assert!(name_finding("src/main.rs", &extra).is_none());
    }

    #[test]
    fn extensoes_de_chave_sao_pegadas() {
        let extra: Vec<String> = Vec::new();
        assert_eq!(name_finding("certs/server.pem", &extra).unwrap().kind, "extensao");
        assert!(name_finding("backup/site.p12", &extra).is_some());
        assert!(name_finding("src/app.perf", &extra).is_none());
    }

    #[test]
    fn regra_configuravel_extra_e_usada() {
        let extra = extra_patterns(Some("firefly-token\n  token-prod  "));
        assert_eq!(extra, vec!["firefly-token", "token-prod"]);
        let f = name_finding("config/firefly-token.yaml", &extra).unwrap();
        assert!(f.reason.contains("regra configurada"), "{}", f.reason);
    }

    #[test]
    fn conteudo_de_chave_privada_e_detectado() {
        let dir = tmp("conteudo");
        let path = dir.join("id_ed25519_fake");
        fs::write(
            &path,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEA\n-----END OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();
        let found = content_findings("id_ed25519_fake", &fs::read(&path).unwrap());
        assert!(found.iter().any(|f| f.reason.contains("chave privada")), "{found:?}");

        let token = "APP_GITHUB=ghp_AAAAAAAAAAAAAAAABBBBBBBBBBBB\n".as_bytes();
        let found = content_findings("t.txt", token);
        assert!(found.iter().any(|f| f.reason.contains("ghp_")), "{found:?}");

        let aws = "key = AKIAABCDEFGHIJKLMNOP\n".as_bytes();
        let found = content_findings("t.txt", aws);
        assert!(found.iter().any(|f| f.reason.contains("AWS")), "{found:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atribuicoes_de_password_real_vs_reference() {
        let found = content_findings("app.conf", "DB_PASSWORD=\"s3nh4-forte\"\n".as_bytes());
        assert!(found.iter().any(|f| f.reason.contains("password")), "{found:?}");

        // referência de ambiente e placeholders conhecidos não disparam bloqueio
        let found = content_findings(
            "app.conf",
            "DB_PASSWORD=${DB_PASSWORD}\npassword = \nPASSWD=changeme-placeholder\n".as_bytes(),
        );
        assert!(!found.iter().any(|f| f.reason.contains("password")), "{found:?}");
    }

    #[test]
    fn binarios_e_grandes_nao_tem_conteudo_lido() {
        let dir = tmp("binario");
        let path = dir.join("dados.bin");
        fs::write(&path, [0u8, 1, 2, 3, 255, 0, 7]).unwrap();
        let extra: Vec<String> = Vec::new();
        let found = inspect_file(&path, "dados.bin", &extra);
        assert!(found.is_empty(), "{found:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn indice_rastreando_credencial_e_cache_aparece_no_alerta() {
        let extra: Vec<String> = Vec::new();
        let found = tracked_findings(
            &["srv/.env".into(), "node_modules/react/index.js".into(), "src/app.rs".into()],
            &extra,
        );
        let env = found.iter().find(|f| f.path == "srv/.env").expect("env");
        assert_eq!(env.severity, "block");
        assert_eq!(env.kind, "rastreado");
        let cache = found.iter().find(|f| f.path.starts_with("node_modules")).expect("cache");
        assert_eq!(cache.severity, "warn");
        assert!(!found.iter().any(|f| f.path == "src/app.rs"));
    }

    #[test]
    fn o_que_bloqueia_o_commit_e_o_conteudo_do_indice() {
        let dir = tmp("indice");
        git::run(&dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]).unwrap();
        git::run(&dir, &["config", "user.name", "Teste"]).unwrap();
        git::run(&dir, &["config", "user.email", "teste@local"]).unwrap();
        git::run(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        fs::create_dir_all(dir.join("srv")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("srv/.env"), "DB_PASSWORD=\"s3nh4-forte\"\n").unwrap();
        fs::write(dir.join("src/app.rs"), "fn main() {}\n").unwrap();
        git::run(&dir, &["add", "-A"]).unwrap();

        let extra: Vec<String> = Vec::new();
        let found = index_findings(&dir, &extra).unwrap();
        assert!(blocks(&found), "{found:?}");
        assert!(found.iter().any(|f| f.path == "srv/.env"), "{found:?}");
        assert!(!found.iter().any(|f| f.path == "src/app.rs"), "{found:?}");

        git::run(&dir, &["commit", "-q", "-m", "base"]).unwrap();
        let no_push = head_findings(&dir, &["srv/.env".into(), "src/app.rs".into()], &extra);
        assert!(blocks(&no_push), "{no_push:?}");

        // um segredo que o commit apenas retira do índice não é motivo de bloqueio
        git::run(&dir, &["rm", "--cached", "-q", "--", "srv/.env"]).unwrap();
        let found = index_findings(&dir, &extra).unwrap();
        assert!(!found.iter().any(|f| f.path == "srv/.env"), "{found:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarize_ordena_e_remove_duplicados() {
        let extra: Vec<String> = Vec::new();
        let mut found = vec![
            name_finding("b/.env", &extra).unwrap(),
            name_finding("a/.env", &extra).unwrap(),
            name_finding("a/.env", &extra).unwrap(),
        ];
        found.extend(content_findings("a/.env", "TOKEN=ghp_AAAAAAAAAAAAAAAABBBBBBBBBBBB\n".as_bytes()));
        let clean = summarize(found);
        assert_eq!(clean[0].path, "a/.env");
        assert_eq!(clean.iter().filter(|f| f.path == "a/.env" && f.kind == "nome").count(), 1);
        assert!(blocks(&clean));
        assert!(!blocks(&[Finding {
            path: "x".into(),
            kind: "rastreado".into(),
            reason: "aviso".into(),
            severity: "warn".into(),
            suggestion: String::new(),
        }]));
    }

    #[test]
    fn varre_arvore_pulando_caches_e_encontra_o_env() {
        let dir = tmp("arvore");
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join("app")).unwrap();
        fs::write(dir.join("app/.env"), "API_KEY=123456\n").unwrap();
        fs::write(dir.join("app/readme.md"), "nada demais aqui\n").unwrap();
        fs::write(dir.join("node_modules/pkg/.env"), "VAZIO\n").unwrap();
        let extra: Vec<String> = Vec::new();
        let found = summarize(scan_tree(&dir, &extra, 500).unwrap());
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"app/.env"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("node_modules")), "{paths:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
