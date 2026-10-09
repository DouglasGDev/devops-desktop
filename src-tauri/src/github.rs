//! Integração com a API do GitHub: conexão da conta pelo fluxo de dispositivo
//! do OAuth e leitura/criação de repositórios reais.
//!
//! Nada aqui é simulado: cada campo exibido na interface vem de uma resposta de
//! `api.github.com`. O host da API é fixo — o usuário nunca fornece URL. O token
//! resultante vive apenas na memória desta execução (mesma regra das senhas de
//! SSH em `auth.rs`): não vai para o SQLite, para os logs nem para o disco. Ao
//! fechar o aplicativo é preciso reconectar.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audit;
use crate::db;

/// Base fixa da API pública. Não é configurável para impedir chamada a host
/// arbitrário.
const API_BASE: &str = "https://api.github.com";
const DEVICE_ENDPOINT: &str = "https://github.com/login/device/code";
const TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
/// Escopo mínimo: ler a conta e gerenciar repositórios próprios.
const OAUTH_SCOPE: &str = "repo,user";
const SETTINGS_KEY: &str = "github_client_id";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 20;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubStatus {
    pub has_client_id: bool,
    pub logged_in: bool,
    /// Login lido de `/user` na autenticação. Só vive na memória.
    pub login: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFlow {
    /// Código curto que o usuário digita em github.com/login/device.
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectResult {
    /// "pending" | "connected" | "error"
    pub status: &'static str,
    pub message: Option<String>,
    pub login: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepo {
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
    pub clone_url: String,
    pub ssh_url: String,
    pub size_kb: i64,
    pub language: Option<String>,
    pub pushed_at: Option<String>,
    pub archived: bool,
    pub fork: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPull {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub author: Option<String>,
    pub head_ref: String,
    pub base_ref: String,
    pub draft: bool,
    pub updated_at: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Clone)]
struct PendingDevice {
    device_code: String,
    expires_at: Instant,
    interval: Duration,
    last_poll: Instant,
}

#[derive(Clone, Default)]
pub struct GitHubState {
    token: Arc<Mutex<Option<String>>>,
    login: Arc<Mutex<Option<String>>>,
    device: Arc<Mutex<Option<PendingDevice>>>,
}

impl GitHubState {
    pub fn new() -> Self {
        Self::default()
    }

    fn token(&self) -> Option<String> {
        self.token.lock().ok().and_then(|g| g.clone())
    }

    fn clear(&self) {
        if let Ok(mut g) = self.token.lock() {
            g.take();
        }
        if let Ok(mut g) = self.login.lock() {
            g.take();
        }
        self.clear_device();
    }

    /// Descarta apenas a autorização em andamento: uma tentativa que expira não
    /// desconecta uma conta já conectada.
    fn clear_device(&self) {
        if let Ok(mut g) = self.device.lock() {
            g.take();
        }
    }
}

// ---------------------------------------------------------------- client id

/// Client ID de OAuth App (20 hex) ou GitHub App (`Iv1.` + hex). A regra é
/// estreita de propósito: um token pessoal colado por engano aqui seria gravado
/// no banco em texto puro, e isso a §4 proíbe.
fn valid_client_id(value: &str) -> bool {
    let trimmed = value.trim();
    let core = trimmed.strip_prefix("Iv1.").unwrap_or(trimmed);
    (16..=40).contains(&core.len()) && core.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn store_client_id(conn: &Connection, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        conn.execute("DELETE FROM settings WHERE key = ?1", [SETTINGS_KEY])
            .map_err(|e| format!("falha ao limpar o Client ID: {e}"))?;
        return audit::log(conn, "info", "github", "Client ID removido", None, None, None, None);
    }
    if !valid_client_id(value) {
        return Err(
            "Client ID inválido: use o identificador da OAuth App (20 caracteres hexadecimais) ou do \
             GitHub App (`Iv1.` seguido de hexadecimal). Não cole um token de acesso aqui."
                .into(),
        );
    }
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        rusqlite::params![SETTINGS_KEY, value, db::now_rfc3339()],
    )
    .map_err(|e| format!("falha ao salvar o Client ID: {e}"))?;
    // O log registra só o prefixo: o valor completo não aparece no histórico.
    let prefix: String = value.chars().take(4).collect();
    audit::log(
        conn,
        "info",
        "github",
        "Client ID do GitHub configurado",
        None,
        None,
        None,
        Some(&format!("prefixo: {prefix}")),
    )
}

fn client_id(conn: &Connection) -> Result<String, String> {
    db::setting_value(conn, SETTINGS_KEY)?.ok_or_else(|| {
        "a conta GitHub precisa de um Client ID: crie uma OAuth App em github.com/settings/developers, \
         marque \"Device Flow\" e cole o identificador em Configurações → GitHub".to_string()
    })
}

pub fn status(conn: &Connection, state: &GitHubState) -> Result<GitHubStatus, String> {
    let has_client_id = db::setting_value(conn, SETTINGS_KEY)?.is_some();
    let login = state.login.lock().ok().and_then(|g| g.clone());
    Ok(GitHubStatus {
        has_client_id,
        logged_in: state.token().is_some(),
        login,
    })
}

// --------------------------------------------------------------------- HTTP

fn error_for(code: u16) -> &'static str {
    match code {
        401 => "a sessão do GitHub expirou ou o token foi revogado; reconecte a conta",
        403 => "o GitHub recusou a chamada (limite de requisições estourado ou escopo insuficiente)",
        404 => "o GitHub não encontrou esse recurso para a conta conectada",
        422 => "o GitHub validou os dados e recusou (nome de repositório inválido ou já em uso)",
        _ => "a API do GitHub respondeu com erro",
    }
}

/// Converte o erro do ureq em mensagem legível, sempre incluindo o que a API
/// informou quando houver corpo parseável.
fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let api_message = read_json(resp)
                .ok()
                .and_then(|v| str_field(&v, "message").filter(|m| !m.trim().is_empty()));
            match api_message {
                Some(m) => format!("{} (GitHub: {m})", error_for(code)),
                None => error_for(code).to_string(),
            }
        }
        ureq::Error::Transport(_) => {
            "não foi possível falar com a API do GitHub (sem rede, proxy ou DNS)".to_string()
        }
    }
}

fn read_json(resp: ureq::Response) -> Result<Value, String> {
    resp.into_json::<Value>()
        .map_err(|e| format!("a resposta do GitHub não é um JSON legível ({e})"))
}

fn post_json(url: &str, body: Value) -> Result<Value, String> {
    ureq::post(url)
        .timeout(REQUEST_TIMEOUT)
        .set("Accept", "application/json")
        .send_json(body)
        .map_err(describe)
        .and_then(read_json)
}

fn post_form(url: &str, fields: &[(&str, String)]) -> Result<Value, String> {
    let body = fields
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoded(v)))
        .collect::<Vec<_>>()
        .join("&");
    ureq::post(url)
        .timeout(REQUEST_TIMEOUT)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .set("Accept", "application/json")
        .send_string(&body)
        .map_err(describe)
        .and_then(read_json)
}

fn get_json(url: &str, token: &str) -> Result<Value, String> {
    ureq::get(url)
        .timeout(REQUEST_TIMEOUT)
        .set("Accept", "application/vnd.github+json")
        .set("Authorization", &format!("Bearer {token}"))
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(describe)
        .and_then(read_json)
}

fn urlencoded(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

fn require_token(state: &GitHubState) -> Result<String, String> {
    state.token().ok_or_else(|| {
        "conecte a conta GitHub primeiro (Configurações → GitHub ou a seção de contas desta tela)".to_string()
    })
}

// -------------------------------------------------------------- dispositivo

pub fn connect_start(conn: &Connection, state: &GitHubState) -> Result<DeviceFlow, String> {
    let id = client_id(conn)?;
    let body = post_json(
        DEVICE_ENDPOINT,
        json!({ "client_id": id, "scope": OAUTH_SCOPE }),
    )?;
    if let Some(err) = str_field(&body, "error") {
        return Err(format!("o GitHub recusou a iniciação do fluxo: {err}"));
    }
    let device_code = str_field(&body, "device_code")
        .ok_or_else(|| "a resposta do GitHub não trouxe o código de dispositivo".to_string())?;
    let user_code = str_field(&body, "user_code")
        .ok_or_else(|| "a resposta do GitHub não trouxe o código de verificação".to_string())?;
    let verification_uri = str_field(&body, "verification_uri").unwrap_or_else(|| "https://github.com/login/device".to_string());
    let expires_in = body.get("expires_in").and_then(Value::as_i64).unwrap_or(900);
    let interval = body.get("interval").and_then(Value::as_i64).unwrap_or(5).clamp(1, 60);

    let now = Instant::now();
    if let Ok(mut guard) = state.device.lock() {
        *guard = Some(PendingDevice {
            device_code,
            expires_at: now + Duration::from_secs(expires_in.max(1) as u64),
            interval: Duration::from_secs(interval as u64),
            last_poll: now,
        });
    }
    Ok(DeviceFlow {
        user_code,
        verification_uri,
        expires_in,
        interval,
    })
}

/// Uma tentativa de conclusão. O frontend repete no intervalo informado pelo
/// próprio GitHub; aqui também há trava de cadência para não tomar 429.
pub fn connect_poll(conn: &Connection, state: &GitHubState) -> Result<ConnectResult, String> {
    let id = client_id(conn)?;
    let Some(pending) = state.device.lock().ok().and_then(|g| g.clone()) else {
        return Ok(ConnectResult::error(
            "não há autorização em andamento; inicie a conexão novamente",
        ));
    };
    if Instant::now() >= pending.expires_at {
        state.clear_device();
        return Ok(ConnectResult::error(
            "o código de verificação expirou; gere um novo",
        ));
    }
    if Instant::now().duration_since(pending.last_poll) < pending.interval {
        return Ok(ConnectResult::pending());
    }

    let body = post_form(
        TOKEN_ENDPOINT,
        &[
            ("client_id", id),
            ("device_code", pending.device_code.clone()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code".to_string()),
        ],
    )?;

    if let Some(token) = str_field(&body, "access_token") {
        let user = get_json(&format!("{API_BASE}/user"), &token)?;
        let login = str_field(&user, "login")
            .ok_or_else(|| "o GitHub não informou o login da conta autenticada".to_string())?;
        if let Ok(mut guard) = state.token.lock() {
            *guard = Some(token);
        }
        if let Ok(mut guard) = state.login.lock() {
            *guard = Some(login.clone());
        }
        if let Ok(mut guard) = state.device.lock() {
            guard.take();
        }
        audit::log(
            conn,
            "info",
            "github",
            "Conta GitHub conectada",
            None,
            None,
            None,
            Some(&format!("login: {login}")),
        )?;
        return Ok(ConnectResult {
            status: "connected",
            message: None,
            login: Some(login),
        });
    }

    match str_field(&body, "error").as_deref() {
        Some("authorization_pending") => Ok(ConnectResult::pending()),
        Some("slow_down") => {
            if let Ok(mut guard) = state.device.lock() {
                if let Some(p) = guard.as_mut() {
                    p.interval += Duration::from_secs(5);
                }
            }
            Ok(ConnectResult::pending())
        }
        Some("expired_token") => {
            state.clear_device();
            Ok(ConnectResult::error("o código expirou; gere um novo"))
        }
        Some("access_denied") => {
            state.clear_device();
            Ok(ConnectResult::error("você negou a autorização no navegador"))
        }
        Some(code) => {
            state.clear_device();
            Ok(ConnectResult::error(format!(
                "o GitHub não completou a autorização ({code})"
            )))
        }
        None => {
            state.clear_device();
            Ok(ConnectResult::error(
                "a resposta do GitHub não trouxe token nem erro reconhecível",
            ))
        }
    }
}

pub fn disconnect(conn: &Connection, state: &GitHubState) -> Result<(), String> {
    state.clear();
    audit::log(conn, "info", "github", "Conta GitHub desconectada", None, None, None, None)
}

// ------------------------------------------------------------------- dados

fn parse_repo(v: &Value) -> Result<GitHubRepo, String> {
    Ok(GitHubRepo {
        name: str_field(v, "name").ok_or_else(|| "repositório sem nome na resposta".to_string())?,
        full_name: str_field(v, "full_name").unwrap_or_default(),
        private: v.get("private").and_then(Value::as_bool).unwrap_or(false),
        default_branch: str_field(v, "default_branch").unwrap_or_default(),
        html_url: str_field(v, "html_url").unwrap_or_default(),
        clone_url: str_field(v, "clone_url").unwrap_or_default(),
        ssh_url: str_field(v, "ssh_url").unwrap_or_default(),
        size_kb: v.get("size").and_then(Value::as_i64).unwrap_or(0),
        language: str_field(v, "language"),
        pushed_at: str_field(v, "pushed_at"),
        archived: v.get("archived").and_then(Value::as_bool).unwrap_or(false),
        fork: v.get("fork").and_then(Value::as_bool).unwrap_or(false),
    })
}

pub fn repos(state: &GitHubState) -> Result<Vec<GitHubRepo>, String> {
    let token = require_token(state)?;
    let mut out = Vec::new();
    // O GitHub devolve no máximo 100 itens por página; percorremos as páginas
    // até esgotar (limite alto o suficiente para qualquer conta pessoal).
    for page in 1..=MAX_PAGES {
        let url = format!(
            "{API_BASE}/user/repos?per_page={PAGE_SIZE}&page={page}&sort=pushed&affiliation=owner"
        );
        let body = get_json(&url, &token)?;
        let items = body
            .as_array()
            .ok_or_else(|| "a lista de repositórios veio em formato inesperado".to_string())?;
        let count = items.len();
        for item in items {
            out.push(parse_repo(item)?);
        }
        if count < PAGE_SIZE {
            break;
        }
    }
    Ok(out)
}

/// Aceita apenas `dono/repo` sem espaços nem caracteres de controle; o segmento
/// é codificado antes de virar URL.
pub fn parse_slug(input: &str) -> Result<(String, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 200 || trimmed.chars().any(|c| c <= ' ' || c == '\u{7f}') {
        return Err("informe o repositório como dono/nome".into());
    }
    let Some((owner, name)) = trimmed.split_once('/') else {
        return Err("informe o repositório como dono/nome".into());
    };
    let valid = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if !valid(owner) || !valid(name) {
        return Err("nome de repositório do GitHub inválido".into());
    }
    Ok((owner.to_string(), name.to_string()))
}

pub fn pulls(state: &GitHubState, slug: &str) -> Result<Vec<GitHubPull>, String> {
    let token = require_token(state)?;
    let (owner, name) = parse_slug(slug)?;
    let url = format!(
        "{API_BASE}/repos/{}/{}/pulls?state=open&per_page=50",
        urlencoded(&owner),
        urlencoded(&name)
    );
    let body = get_json(&url, &token)?;
    let items = body
        .as_array()
        .ok_or_else(|| "a lista de pull requests veio em formato inesperado".to_string())?;
    Ok(items
        .iter()
        .filter_map(|v| {
            let number = v.get("number").and_then(Value::as_i64)?;
            let head = v.get("head").and_then(|h| h.get("ref")).and_then(Value::as_str);
            let base = v.get("base").and_then(|b| b.get("ref")).and_then(Value::as_str);
            Some(GitHubPull {
                number,
                title: str_field(v, "title").unwrap_or_else(|| "(sem título)".to_string()),
                state: str_field(v, "state").unwrap_or_else(|| "open".to_string()),
                author: str_field(v.get("user")?, "login"),
                head_ref: head.unwrap_or_default().to_string(),
                base_ref: base.unwrap_or_default().to_string(),
                draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
                updated_at: str_field(v, "updated_at"),
                html_url: str_field(v, "html_url").unwrap_or_default(),
            })
        })
        .collect())
}

/// Nome de repositório aceito pelo GitHub (sem espaço, sem caracteres de
/// controle).
fn valid_repo_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    name.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn create_repo(
    conn: &Connection,
    state: &GitHubState,
    name: &str,
    private: bool,
    description: Option<&str>,
) -> Result<GitHubRepo, String> {
    let token = require_token(state)?;
    let name = name.trim();
    if !valid_repo_name(name) {
        return Err(
            "nome de repositório inválido: use letras, números, ponto, hífen ou underline (até 100 caracteres, \
             começando por letra ou número)".into(),
        );
    }
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    if description.is_some_and(|d| d.len() > 500 || d.contains('\0')) {
        return Err("a descrição deve ter no máximo 500 caracteres".into());
    }
    let mut payload = json!({ "name": name, "private": private });
    if let Some(d) = description {
        payload["description"] = json!(d);
    }
    let body = ureq::post(&format!("{API_BASE}/user/repos"))
        .timeout(REQUEST_TIMEOUT)
        .set("Accept", "application/vnd.github+json")
        .set("Authorization", &format!("Bearer {token}"))
        .set("X-GitHub-Api-Version", "2022-11-28")
        .send_json(payload)
        .map_err(describe)
        .and_then(read_json)?;
    let repo = parse_repo(&body)?;
    audit::log(
        conn,
        "warning",
        "github",
        "Repositório criado no GitHub",
        None,
        None,
        None,
        Some(&format!(
            "{} · {}",
            repo.full_name,
            if repo.private { "privado" } else { "público" }
        )),
    )?;
    Ok(repo)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepoInput {
    pub name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub description: Option<String>,
}

impl ConnectResult {
    fn pending() -> Self {
        Self {
            status: "pending",
            message: None,
            login: None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            status: "error",
            message: Some(message.into()),
            login: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_aceita_formato_oficial_e_recusa_token() {
        assert!(valid_client_id("Iv1.b50e9c5a61f47c2b"));
        assert!(valid_client_id("1234567890abcdef1234"));
        // vazio, curto demais, com espaço ou com caractere de estrutura
        assert!(!valid_client_id(""));
        assert!(!valid_client_id("abc"));
        assert!(!valid_client_id("1234567890 abcdef1234"));
        assert!(!valid_client_id(&"a".repeat(65)));
        // Tokens de acesso jamais são aceitos neste campo: ele é gravado no
        // banco em texto puro, e um token ali colado viraria segredo persistido.
        for token in [
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234",
            "gho_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234",
            "github_pat_11ABCDEFG0abcdefghij_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
        ] {
            assert!(!valid_client_id(token), "aceitou um token: {token}");
        }
    }

    #[test]
    fn client_id_invalido_nao_chega_ao_banco() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        let err = store_client_id(&conn, "abc").unwrap_err();
        assert!(err.contains("Client ID inválido"));
        assert!(db::setting_value(&conn, SETTINGS_KEY).unwrap().is_none());
    }

    #[test]
    fn client_id_valido_fica_no_banco_e_o_prefixo_no_log() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        store_client_id(&conn, "Iv1.b50e9c5a61f47c2b").unwrap();
        assert_eq!(
            db::setting_value(&conn, SETTINGS_KEY).unwrap().as_deref(),
            Some("Iv1.b50e9c5a61f47c2b")
        );
        let logged: String = conn
            .query_row(
                "SELECT detail FROM audit_log WHERE category = 'github' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // O histórico guarda só o prefixo, nunca o identificador inteiro.
        assert_eq!(logged, "prefixo: Iv1.");
    }

    #[test]
    fn sem_client_id_a_mensagem_explica_onde_criar() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        let err = client_id(&conn).unwrap_err();
        assert!(err.contains("github.com/settings/developers"));
        assert!(err.contains("Device Flow"));
    }

    #[test]
    fn status_nunca_vaza_token() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        let state = GitHubState::new();
        *state.token.lock().unwrap() = Some("ghp_SEGREDOINTEIRO".to_string());
        *state.login.lock().unwrap() = Some("DouglasGDev".to_string());
        store_client_id(&conn, "Iv1.b50e9c5a61f47c2b").unwrap();
        let json = serde_json::to_string(&status(&conn, &state).unwrap()).unwrap();
        assert!(json.contains("DouglasGDev"));
        assert!(!json.contains("SEGREDOINTEIRO"));
        assert!(!json.contains("Iv1.b50e9c5a61f47c2b"));
    }

    #[test]
    fn autorizacao_que_falha_nao_derruba_conta_ja_conectada() {
        let state = GitHubState::new();
        *state.token.lock().unwrap() = Some("ghp_segredo".to_string());
        *state.login.lock().unwrap() = Some("DouglasGDev".to_string());
        // expirou/negou: só a pendência do fluxo de dispositivo sai
        state.clear_device();
        assert!(state.token.lock().unwrap().is_some());
        assert!(state.login.lock().unwrap().is_some());
        // desconectar de fato limpa o token
        state.clear();
        assert!(state.token.lock().unwrap().is_none());
        assert!(state.login.lock().unwrap().is_none());
    }

    #[test]
    fn deslogado_marca_que_falta_conexao() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        let state = GitHubState::new();
        assert!(repos(&state).unwrap_err().contains("conecte a conta"));
        assert!(pulls(&state, "a/b").unwrap_err().contains("conecte a conta"));
        assert!(create_repo(&conn, &state, "repo", false, None)
            .unwrap_err()
            .contains("conecte a conta"));
    }

    #[test]
    fn slug_de_repositorio_e_validado_antes_de_virar_url() {
        assert_eq!(parse_slug("DouglasGDev/devops-desktop").unwrap().1, "devops-desktop");
        assert_eq!(parse_slug(" acme/app.git ").unwrap().0, "acme");
        for caso in ["", "sem-barra", "a/b/c", "a/b c", "a/b\nrm -rf", "a//b", "a/", "/b"] {
            assert!(parse_slug(caso).is_err(), "aceitou {caso}");
        }
    }

    #[test]
    fn nome_de_repositorio_segue_as_regras_do_github() {
        for ok in ["a", "devops-desktop", "app_1.2", "X9"] {
            assert!(valid_repo_name(ok), "rejeitou {ok}");
        }
        for ruim in ["", "-inicio", ".inicio", "com espaço", "a\nb", &"a".repeat(101)] {
            assert!(!valid_repo_name(ruim), "aceitou {ruim}");
        }
    }

    #[test]
    fn percent_encode_protege_a_url() {
        assert_eq!(urlencoded("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(urlencoded("DouglasGDev.dev-1"), "DouglasGDev.dev-1");
    }

    #[test]
    fn repositorio_lido_da_resposta_real_do_github() {
        // Fixture de uma resposta `GET /user/repos`; o app só exibe dados vivos,
        // isto é um teste de parser.
        let v: Value = serde_json::from_str(
            r#"[{
              "name": "devops-desktop",
              "full_name": "DouglasGDev/devops-desktop",
              "private": false,
              "default_branch": "main",
              "html_url": "https://github.com/DouglasGDev/devops-desktop",
              "clone_url": "https://github.com/DouglasGDev/devops-desktop.git",
              "ssh_url": "git@github.com:DouglasGDev/devops-desktop.git",
              "size": 1024,
              "language": "Rust",
              "pushed_at": "2026-10-09T12:00:00Z",
              "archived": false,
              "fork": false
            }]"#,
        )
        .unwrap();
        let repo = parse_repo(&v[0]).unwrap();
        assert_eq!(repo.full_name, "DouglasGDev/devops-desktop");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.size_kb, 1024);
        assert_eq!(repo.language.as_deref(), Some("Rust"));
        assert!(!repo.private);
        // sem nome a entrada é descartada, não inventada
        assert!(parse_repo(&json!({ "private": true })).is_err());
    }

    #[test]
    fn erro_http_vira_mensagem_em_portugues() {
        assert!(error_for(401).contains("reconecte"));
        assert!(error_for(403).contains("escopo"));
        assert!(error_for(422).contains("recusou"));
        assert!(error_for(500).contains("API do GitHub"));
    }

    #[test]
    fn fluxo_de_dispositivo_guarda_o_codigo_no_backend() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        let state = GitHubState::new();
        // Sem client id a iniciação falha com diagnóstico, sem tocar na rede.
        let err = connect_start(&conn, &state).unwrap_err();
        assert!(err.contains("Client ID"));
        // Sem pendência registrada a consulta responde com orientação clara.
        let resultado = connect_poll(&conn, &state);
        assert!(resultado.is_err() || resultado.unwrap().status == "error");
    }
}
