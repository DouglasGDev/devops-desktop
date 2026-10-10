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

/// Client ID de OAuth App ou de GitHub App. O GitHub já emitiu o identificador
/// de três jeitos: 40 (ou 20) caracteres hexadecimais, os de hoje em base62 com
/// maiúsculas e minúsculas (`Ov23li…`, ~20 caracteres) e `Iv1.` + hexadecimal
/// para GitHub App. A regra é estreita de propósito: os tokens de acesso têm `_`
/// ou `-` e por isso são recusados — colar um token aqui seria gravar um segredo
/// no banco, o que a §4 proíbe.
///
/// O Client **secret** continua hexadecimal e cabe na regra, então formato
/// nenhum distingue os dois — o que distingue é o campo de onde foi copiado.
fn valid_client_id(value: &str) -> bool {
    let trimmed = value.trim();
    let core = trimmed.split_once('.').map_or(trimmed, |(_, id)| id);
    (16..=40).contains(&core.chars().count()) && core.chars().all(|c| c.is_ascii_alphanumeric())
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
            "Client ID inválido: cole o campo “Client ID” da página da OAuth App (os atuais têm ~20 letras e \
             números; os antigos são hexadecimais; um GitHub App começa com `Iv1.`). Token de acesso e Client \
             secret não servem neste campo."
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
        404 => "o GitHub não reconhece o que foi enviado",
        422 => "o GitHub validou os dados e recusou (nome de repositório inválido ou já em uso)",
        _ => "a API do GitHub respondeu com erro",
    }
}

/// O que o GitHub escreveu no corpo, nas três formas que ele usa: `message` na
/// API REST, `error_description`/`error` nos endpoints OAuth. Sem isto, um 404
/// do fluxo de dispositivo virava texto genérico e a pessoa não descobria que
/// era o Client ID que não existe.
fn detalhe_do_corpo(v: &Value) -> Option<String> {
    ["error_description", "message", "error"]
        .iter()
        .find_map(|chave| str_field(v, chave))
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
}

/// Converte o erro do ureq em mensagem legível, sempre incluindo o que a API
/// informou quando houver corpo parseável.
fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let detalhe = read_json(resp).ok().and_then(|v| detalhe_do_corpo(&v));
            match detalhe {
                Some(d) => format!("{} (GitHub: {d})", error_for(code)),
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

/// Um 404 no endpoint de dispositivo tem uma causa só: o Client ID enviado não
/// existe no GitHub. Confirmado contra o endpoint real — um ID que existe devolve
/// 200 com o código, um ID inventado devolve 404 `{"error":"Not Found"}`. A frase
/// diz o que está salvo sem expor o valor inteiro.
fn dica_do_client_id(erro: &str, id: &str) -> String {
    if !erro.contains("não reconhece") {
        return erro.to_string();
    }
    let prefixo: String = id.chars().take(6).collect();
    format!(
        "{erro} O valor guardado começa com `{prefixo}` e tem {} caracteres. Esse endpoint só responde 404 quando o \
         identificador não existe: abra a página do app em github.com/settings/developers e confira se o que foi \
         colado é o **Client ID** (em cima) e não o **Client secret** (40 hex). Se o identificador estiver certo, \
         marque **Enable Device Flow** no app. Salvou o secret sem querer? Gere um secret novo nessa página — o \
         anterior ficou no banco local em texto puro até você trocar o valor.",
        id.chars().count()
    )
}

pub fn connect_start(conn: &Connection, state: &GitHubState) -> Result<DeviceFlow, String> {
    let id = client_id(conn)?;
    let body = post_json(DEVICE_ENDPOINT, json!({ "client_id": id.clone(), "scope": OAUTH_SCOPE }))
        .map_err(|e| dica_do_client_id(&e, &id))?;
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

    if let Ok(mut guard) = state.device.lock() {
        if let Some(p) = guard.as_mut() {
            p.last_poll = Instant::now();
        }
    }

    let body = post_form(
        TOKEN_ENDPOINT,
        &[
            ("client_id", id.clone()),
            ("device_code", pending.device_code.clone()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code".to_string()),
        ],
    )
    .map_err(|e| dica_do_client_id(&e, &id))?;

    if let Some(token) = str_field(&body, "access_token") {
        // O token foi concedido pelo github.com e passa a valer antes mesmo de
        // api.github.com responder: ler o login é conveniência, não condição.
        // Se a leitura falhar (proxy, escopo), a autorização fica guardada e a
        // mensagem diz o que falhou, em vez de jogar a autorização fora e
        // aparecer como "conexão falhou" o que foi só uma leitura a mais.
        if let Ok(mut guard) = state.token.lock() {
            *guard = Some(token.clone());
        }
        if let Ok(mut guard) = state.device.lock() {
            guard.take();
        }
        let (login, aviso) = match get_json(&format!("{API_BASE}/user"), &token) {
            Ok(user) => match str_field(&user, "login") {
                Some(l) => (Some(l), None),
                None => (None, Some("o GitHub respondeu sem o login da conta".to_string())),
            },
            Err(e) => (None, Some(e)),
        };
        if let Some(ref l) = login {
            if let Ok(mut guard) = state.login.lock() {
                *guard = Some(l.clone());
            }
        }
        let detalhe = match (&login, &aviso) {
            (Some(l), _) => format!("login: {l}"),
            (None, Some(e)) => e.clone(),
            (None, None) => "a resposta não informou o login".to_string(),
        };
        audit::log(
            conn,
            if login.is_some() { "info" } else { "warning" },
            "github",
            if login.is_some() { "Conta GitHub conectada" } else { "Conta autorizada, login não lido" },
            None,
            None,
            None,
            Some(&detalhe),
        )?;
        return Ok(ConnectResult {
            status: "connected",
            message: aviso,
            login,
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

// ------------------------------------------------------------- workspace
//
// As rotas abaixo são o que faz a seção GitHub ser um posto de trabalho e não
// uma lista: a bandeja com o que é meu, o pull request com o diff real e as
// ações de revisar/mergear, as issues, e o estado do CI. Os métodos HTTP são
// literais deste arquivo; o que vem da interface é no máximo o `dono/repo`
// (validado por `parse_slug`) e números, então nenhuma URL é montada a partir
// de texto livre.

/// Chama a API autenticada. Rotas de escrita respondem 201/204 sem corpo — ali
/// o corpo vazio é sucesso, não erro de leitura.
fn api_json(method: &str, url: &str, token: &str, body: Option<Value>) -> Result<Value, String> {
    let req = ureq::request(method, url)
        .timeout(REQUEST_TIMEOUT)
        .set("Accept", "application/vnd.github+json")
        .set("Authorization", &format!("Bearer {token}"))
        .set("X-GitHub-Api-Version", "2022-11-28");
    let resp = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    }
    .map_err(describe)?;
    let codigo = resp.status();
    match read_json(resp) {
        Ok(v) => Ok(v),
        Err(_) if codigo == 201 || codigo == 204 => Ok(json!({ "ok": true })),
        Err(e) => Err(e),
    }
}

/// `https://api.github.com/repos/<dono>/<nome><rota>`, com o slug validado e
/// cada segmento codificado.
fn repo_url(slug: &str, rota: &str) -> Result<String, String> {
    let (owner, name) = parse_slug(slug)?;
    Ok(format!(
        "{API_BASE}/repos/{}/{}{rota}",
        urlencoded(&owner),
        urlencoded(&name)
    ))
}

fn corpo_texto(body: &str, limite: usize, o_que: &str) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(format!("escreva {o_que} antes de enviar"));
    }
    if trimmed.len() > limite || trimmed.contains('\0') {
        return Err(format!("{o_que} deve ter no máximo {limite} caracteres"));
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSearchItem {
    pub number: i64,
    pub title: String,
    /// `dono/repo` — vem embutido na resposta da busca.
    pub repo: String,
    pub author: Option<String>,
    pub state: String,
    pub is_pull_request: bool,
    pub draft: bool,
    pub url: String,
    pub updated_at: Option<String>,
}

fn parse_search_item(v: &Value) -> Option<GitHubSearchItem> {
    let number = v.get("number").and_then(Value::as_i64)?;
    let repo = v.get("repository").and_then(|r| str_field(r, "full_name"))?;
    Some(GitHubSearchItem {
        number,
        title: str_field(v, "title").unwrap_or_else(|| "(sem título)".to_string()),
        repo,
        author: v.get("user").and_then(|u| str_field(u, "login")),
        state: str_field(v, "state").unwrap_or_else(|| "open".to_string()),
        is_pull_request: e_pull_request(v),
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
        url: str_field(v, "html_url").unwrap_or_default(),
        updated_at: str_field(v, "updated_at"),
    })
}

fn search(state: &GitHubState, query: &str) -> Result<Vec<GitHubSearchItem>, String> {
    let token = require_token(state)?;
    let url = format!(
        "{API_BASE}/search/issues?per_page=30&sort=updated&order=desc&q={}",
        urlencoded(query)
    );
    let body = get_json(&url, &token)?;
    let items = body
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "a busca do GitHub veio em formato inesperado".to_string())?;
    Ok(items.iter().filter_map(parse_search_item).collect())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubInbox {
    pub review_requests: Vec<GitHubSearchItem>,
    pub my_pulls: Vec<GitHubSearchItem>,
    pub my_issues: Vec<GitHubSearchItem>,
}

/// A bandeja da conta: o que pediram para você revisar, o que você abriu e o
/// que está atribuído a você. As três leituras são da API de busca no momento;
/// um item sem `repository` na resposta é descartado, porque sem o repositório
/// não há onde abri-lo.
pub fn inbox(state: &GitHubState) -> Result<GitHubInbox, String> {
    Ok(GitHubInbox {
        review_requests: search(state, "is:pr review-requested:@me state:open")?,
        my_pulls: search(state, "is:pr author:@me state:open")?,
        my_issues: search(state, "is:issue assignees:@me state:open")?,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDetail {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub author: Option<String>,
    pub head_ref: String,
    pub base_ref: String,
    /// SHA da cabeça no momento da leitura: é o commit que vai entrar no merge,
    /// e a interface mergeia exatamente ele.
    pub head_sha: String,
    pub draft: bool,
    /// `None` enquanto o GitHub ainda calcula — a tela diz isso em vez de
    /// afirmar que dá (ou não) para mergear.
    pub mergeable: Option<bool>,
    pub mergeable_state: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub body: Option<String>,
    pub updated_at: Option<String>,
    pub url: String,
}

fn parse_pull_detail(v: &Value) -> Result<PullDetail, String> {
    let number = v
        .get("number")
        .and_then(Value::as_i64)
        .ok_or_else(|| "o pull request veio sem número na resposta".to_string())?;
    Ok(PullDetail {
        number,
        title: str_field(v, "title").unwrap_or_else(|| "(sem título)".to_string()),
        state: str_field(v, "state").unwrap_or_else(|| "open".to_string()),
        author: v.get("user").and_then(|u| str_field(u, "login")),
        head_ref: v.get("head").and_then(|h| str_field(h, "ref")).unwrap_or_default(),
        base_ref: v.get("base").and_then(|b| str_field(b, "ref")).unwrap_or_default(),
        head_sha: v.get("head").and_then(|h| str_field(h, "sha")).unwrap_or_default(),
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
        mergeable: v.get("mergeable").and_then(Value::as_bool),
        mergeable_state: str_field(v, "mergeable_state"),
        additions: v.get("additions").and_then(Value::as_i64).unwrap_or(0),
        deletions: v.get("deletions").and_then(Value::as_i64).unwrap_or(0),
        changed_files: v.get("changed_files").and_then(Value::as_i64).unwrap_or(0),
        body: str_field(v, "body").filter(|b| !b.trim().is_empty()),
        updated_at: str_field(v, "updated_at"),
        url: str_field(v, "html_url").unwrap_or_default(),
    })
}

pub fn pull_detail(state: &GitHubState, slug: &str, number: i64) -> Result<PullDetail, String> {
    let token = require_token(state)?;
    let url = repo_url(slug, &format!("/pulls/{number}"))?;
    let body = get_json(&url, &token)?;
    parse_pull_detail(&body)
}

/// O que o GitHub chama de status vira o caractere que a tela de diff já usa.
fn letra_do_status(status: &str) -> char {
    match status {
        "added" => 'A',
        "removed" => 'D',
        "renamed" => 'R',
        "copied" => 'C',
        _ => 'M',
    }
}

/// O GitHub manda o patch **sem** o cabeçalho `diff --git`, então ele é
/// recomposto para o arquivo passar pelo mesmo parser do diff local: a revisão
/// de um pull request usa exatamente a mesma tela de um diff da própria pasta.
pub fn diff_de_arquivo_de_pr(item: &Value) -> Option<Vec<crate::git::FileDiff>> {
    let filename = str_field(item, "filename")?;
    let status = str_field(item, "status").unwrap_or_else(|| "modified".to_string());
    let mut texto = format!("diff --git a/{filename} b/{filename}\n");
    if let Some(anterior) = str_field(item, "previous_filename") {
        texto.push_str(&format!("rename from {anterior}\nrename to {filename}\n"));
    }
    match str_field(item, "patch") {
        Some(p) => {
            texto.push_str(&p);
            texto.push('\n');
        }
        // Sem patch o GitHub não devolve conteúdo (arquivo binário ou grande
        // demais): mostra o arquivo com a mudança anotada, sem hunks.
        None => texto.push_str("Binary files differ\n"),
    }
    let letra = letra_do_status(&status);
    let mut arquivos = crate::git::parse_diff(&texto);
    for f in arquivos.iter_mut() {
        f.status_char = letra;
    }
    Some(arquivos)
}

pub fn pull_files(state: &GitHubState, slug: &str, number: i64) -> Result<Vec<crate::git::FileDiff>, String> {
    let token = require_token(state)?;
    let url = format!(
        "{}?per_page=100",
        repo_url(slug, &format!("/pulls/{number}/files"))?
    );
    let body = get_json(&url, &token)?;
    let itens = body
        .as_array()
        .ok_or_else(|| "a lista de arquivos do pull request veio em formato inesperado".to_string())?;
    let mut out = Vec::new();
    for item in itens {
        if let Some(mut partes) = diff_de_arquivo_de_pr(item) {
            out.append(&mut partes);
        }
    }
    Ok(out)
}

/// O SHA que pode ir no merge: `>= 7` hex, lido do detalhe do pull request.
fn sha_suficiente(sha: &str) -> bool {
    sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit())
}

/// Na API do GitHub todo pull request também é uma issue. Isto diz quando o
/// objeto é um PR — a lista de issues da tela não mostra pull request.
fn e_pull_request(v: &Value) -> bool {
    v.get("pull_request").is_some()
}

/// A descrição vem na própria lista da API. Vazio não vira linha na tela e o
/// texto gigante é cortado no fim de uma char, com aviso de que há mais.
fn descricao_da_lista(v: &Value, limite: usize) -> Option<String> {
    let bruta = v.get("body").and_then(Value::as_str)?.trim();
    if bruta.is_empty() {
        return None;
    }
    let mut texto = bruta.to_string();
    if texto.len() > limite {
        let mut fim = limite;
        while !texto.is_char_boundary(fim) {
            fim -= 1;
        }
        texto.truncate(fim);
        texto.push_str("… (descrição cortada; abra no site para ler inteira)");
    }
    Some(texto)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub author: Option<String>,
    pub labels: Vec<String>,
    pub comments: i64,
    pub body: Option<String>,
    pub updated_at: Option<String>,
    pub url: String,
}

fn parse_issue(v: &Value) -> Option<GitHubIssue> {
    let number = v.get("number").and_then(Value::as_i64)?;
    let labels = v
        .get("labels")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|x| str_field(x, "name"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(GitHubIssue {
        number,
        title: str_field(v, "title").unwrap_or_else(|| "(sem título)".to_string()),
        state: str_field(v, "state").unwrap_or_else(|| "open".to_string()),
        author: v.get("user").and_then(|u| str_field(u, "login")),
        labels,
        comments: v.get("comments").and_then(Value::as_i64).unwrap_or(0),
        body: descricao_da_lista(v, 4000),
        updated_at: str_field(v, "updated_at"),
        url: str_field(v, "html_url").unwrap_or_default(),
    })
}

/// A rota `/issues` do GitHub devolve pull requests misturados (todo PR é uma
/// issue). Eles são filtrados aqui: a aba de issues mostra issues.
pub fn issues(state: &GitHubState, slug: &str) -> Result<Vec<GitHubIssue>, String> {
    let token = require_token(state)?;
    let url = format!(
        "{}?state=open&per_page=50&sort=updated",
        repo_url(slug, "/issues")?
    );
    let body = get_json(&url, &token)?;
    let itens = body
        .as_array()
        .ok_or_else(|| "a lista de issues veio em formato inesperado".to_string())?;
    Ok(itens
        .iter()
        .filter(|v| !e_pull_request(v))
        .filter_map(parse_issue)
        .collect())
}

pub fn create_issue(
    conn: &Connection,
    state: &GitHubState,
    slug: &str,
    title: &str,
    body: Option<&str>,
) -> Result<GitHubIssue, String> {
    let token = require_token(state)?;
    let title = corpo_texto(title, 256, "o título da issue")?;
    let url = repo_url(slug, "/issues")?;
    let resposta = api_json(
        "POST",
        &url,
        &token,
        Some(json!({
            "title": title,
            "body": body.map(str::trim).filter(|b| !b.is_empty()),
        })),
    )?;
    let issue = parse_issue(&resposta).ok_or_else(|| "a issue criada não veio completa na resposta".to_string())?;
    audit::log(
        conn,
        "warning",
        "github",
        "Issue aberta no GitHub",
        None,
        None,
        None,
        Some(&format!("{} · #{}", issue.number, slug_para_log(slug))),
    )?;
    Ok(issue)
}

/// Comentário em issue **ou** pull request: é a mesma rota do GitHub (todo PR é
/// uma issue), por isso uma função só.
pub fn comment(
    conn: &Connection,
    state: &GitHubState,
    slug: &str,
    number: i64,
    body: &str,
) -> Result<(), String> {
    let token = require_token(state)?;
    let body = corpo_texto(body, 65_000, "o comentário")?;
    let url = repo_url(slug, &format!("/issues/{number}/comments"))?;
    api_json("POST", &url, &token, Some(json!({ "body": body })))?;
    audit::log(
        conn,
        "info",
        "github",
        "Comentário publicado no GitHub",
        None,
        None,
        None,
        Some(&format!("#{number} · {}", slug_para_log(slug))),
    )
}

pub fn close_issue(conn: &Connection, state: &GitHubState, slug: &str, number: i64) -> Result<(), String> {
    let token = require_token(state)?;
    let url = repo_url(slug, &format!("/issues/{number}"))?;
    api_json("PATCH", &url, &token, Some(json!({ "state": "closed" })))?;
    audit::log(
        conn,
        "warning",
        "github",
        "Issue fechada no GitHub",
        None,
        None,
        None,
        Some(&format!("#{number} · {}", slug_para_log(slug))),
    )
}

/// Os três eventos que a API aceita para revisão de pull request.
fn valid_event_review(event: &str) -> Result<&'static str, String> {
    match event {
        "APPROVE" => Ok("APPROVE"),
        "REQUEST_CHANGES" => Ok("REQUEST_CHANGES"),
        "COMMENT" => Ok("COMMENT"),
        other => Err(format!("tipo de revisão inválido: {other}")),
    }
}

/// Os três métodos de merge do GitHub. `sha` é a cabeça lida no detalhe do PR:
/// mergear sem ele poderia incluir commits que chegaram depois da revisão.
fn valid_metodo_merge(method: &str) -> Result<&'static str, String> {
    match method {
        "merge" => Ok("merge"),
        "squash" => Ok("squash"),
        "rebase" => Ok("rebase"),
        other => Err(format!("método de merge inválido: {other}")),
    }
}

pub fn review_pull(
    conn: &Connection,
    state: &GitHubState,
    slug: &str,
    number: i64,
    event: &str,
    body: Option<&str>,
) -> Result<(), String> {
    let token = require_token(state)?;
    let event = valid_event_review(event)?;
    let url = repo_url(slug, &format!("/pulls/{number}/reviews"))?;
    api_json(
        "POST",
        &url,
        &token,
        Some(json!({
            "event": event,
            "body": body.map(str::trim).filter(|b| !b.is_empty()),
        })),
    )?;
    let o_que = match event {
        "APPROVE" => "Pull request aprovado no GitHub",
        "REQUEST_CHANGES" => "Alterações pedidas no GitHub",
        _ => "Revisão comentada no GitHub",
    };
    audit::log(
        conn,
        "warning",
        "github",
        o_que,
        None,
        None,
        None,
        Some(&format!("#{number} · {}", slug_para_log(slug))),
    )
}

pub fn merge_pull(
    conn: &Connection,
    state: &GitHubState,
    slug: &str,
    number: i64,
    method: &str,
    sha: &str,
) -> Result<String, String> {
    let token = require_token(state)?;
    let method = valid_metodo_merge(method)?;
    if !sha_suficiente(sha) {
        return Err("o commit que seria mergido não foi lido do GitHub; atualize o pull request".into());
    }
    let url = repo_url(slug, &format!("/pulls/{number}/merge"))?;
    let resposta = api_json(
        "PUT",
        &url,
        &token,
        Some(json!({ "sha": sha, "merge_method": method })),
    )?;
    let mensagem = str_field(&resposta, "message").unwrap_or_else(|| "merge aceito".to_string());
    audit::log(
        conn,
        "warning",
        "github",
        "Pull request mesclado no GitHub",
        None,
        None,
        None,
        Some(&format!("#{number} · {method} · {}", slug_para_log(slug))),
    )?;
    Ok(mensagem)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRun {
    pub id: i64,
    pub name: String,
    pub display_title: String,
    pub head_branch: String,
    pub event: String,
    /// "queued" | "in_progress" | "completed" …
    pub status: String,
    /// Só existe quando o trabalho terminou: success, failure, cancelled…
    pub conclusion: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub url: String,
}

fn parse_run(v: &Value) -> Option<GitHubRun> {
    let id = v.get("id").and_then(Value::as_i64)?;
    Some(GitHubRun {
        id,
        name: str_field(v, "name").unwrap_or_else(|| "(sem nome)".to_string()),
        display_title: str_field(v, "display_title").unwrap_or_default(),
        head_branch: str_field(v, "head_branch").unwrap_or_default(),
        event: str_field(v, "event").unwrap_or_default(),
        status: str_field(v, "status").unwrap_or_else(|| "desconhecido".to_string()),
        conclusion: str_field(v, "conclusion"),
        created_at: str_field(v, "created_at"),
        updated_at: str_field(v, "updated_at"),
        url: str_field(v, "html_url").unwrap_or_default(),
    })
}

/// As execuções de workflow do repositório — o que costuma ser a primeira
/// pergunta de quem faz deploy: o que rodou, o que passou, o que quebrou.
pub fn runs(state: &GitHubState, slug: &str) -> Result<Vec<GitHubRun>, String> {
    let token = require_token(state)?;
    let url = format!("{}?per_page=25", repo_url(slug, "/actions/runs")?);
    let body = get_json(&url, &token)?;
    let itens = body
        .get("workflow_runs")
        .and_then(Value::as_array)
        .ok_or_else(|| "a lista de execuções do Actions veio em formato inesperado".to_string())?;
    Ok(itens.iter().filter_map(parse_run).collect())
}

pub fn rerun(conn: &Connection, state: &GitHubState, slug: &str, run_id: i64) -> Result<(), String> {
    let token = require_token(state)?;
    let url = repo_url(slug, &format!("/actions/runs/{run_id}/reruns"))?;
    api_json("POST", &url, &token, None)?;
    audit::log(
        conn,
        "warning",
        "github",
        "Execução do Actions reexecutada",
        None,
        None,
        None,
        Some(&format!("run {run_id} · {}", slug_para_log(slug))),
    )
}

/// O log nunca recebe o slug cru com o que o usuário digitou: só o que já foi
/// validado como `dono/repo`.
fn slug_para_log(slug: &str) -> String {
    match parse_slug(slug) {
        Ok((owner, name)) => format!("{owner}/{name}"),
        Err(_) => "repositório?".to_string(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullNumberInput {
    pub repo: String,
    pub number: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentInput {
    pub repo: String,
    pub number: i64,
    pub body: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInput {
    pub repo: String,
    pub number: i64,
    pub event: String,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeInput {
    pub repo: String,
    pub number: i64,
    pub method: String,
    pub sha: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewIssueInput {
    pub repo: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseIssueInput {
    pub repo: String,
    pub number: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RerunInput {
    pub repo: String,
    pub run_id: i64,
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
        assert!(valid_client_id("1234567890abcdef1234567890abcdef12345678"));
        // OAuth App criada hoje: base62 de 20 caracteres, misto de maiúsculas e
        // minúsculas. Exigir hexadecimal aqui recusava o identificador verdadeiro.
        assert!(valid_client_id("Ov23liZK4pQr7mTxbN1a"));
        // vazio, curto demais, com espaço ou com caractere de estrutura
        assert!(!valid_client_id(""));
        assert!(!valid_client_id("abc"));
        assert!(!valid_client_id("1234567890 abcdef1234"));
        assert!(!valid_client_id("12345678-90ab-cdef-1234-567890abcdef"));
        assert!(!valid_client_id(&"a".repeat(65)));
        // Tokens de acesso jamais são aceitos neste campo: ele é gravado no
        // banco em texto puro, e um token ali colado viraria segredo persistido.
        for token in [
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234",
            "gho_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234",
            "ghs_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234",
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
        assert!(error_for(404).contains("não reconhece"));
        assert!(error_for(422).contains("recusou"));
        assert!(error_for(500).contains("API do GitHub"));
    }

    #[test]
    fn o_que_o_github_escreveu_no_corpo_nao_se_perde() {
        // As três formas que o GitHub usa: `message` na API REST,
        // `error_description`/`error` nos endpoints OAuth.
        let rest: Value = serde_json::from_str(r#"{"message":"Not Found"}"#).unwrap();
        assert_eq!(detalhe_do_corpo(&rest).as_deref(), Some("Not Found"));
        let oauth: Value = serde_json::from_str(
            r#"{"error":"invalid_client","error_description":"Client ID not found"}"#,
        )
        .unwrap();
        assert_eq!(
            detalhe_do_corpo(&oauth).as_deref(),
            Some("Client ID not found"),
            "a descrição é mais específica que o código"
        );
        let so_codigo: Value = serde_json::from_str(r#"{"error":"Not Found"}"#).unwrap();
        assert_eq!(detalhe_do_corpo(&so_codigo).as_deref(), Some("Not Found"));
        let vazio: Value = serde_json::from_str(r#"{"error":"   "}"#).unwrap();
        assert_eq!(detalhe_do_corpo(&vazio), None, "branco não é diagnóstico");
    }

    #[test]
    fn client_id_inexistente_diz_o_que_esta_salvo() {
        let id = "Iv1.abcd1234ef567890";
        let dica = dica_do_client_id(error_for(404), id);
        assert!(dica.contains("Iv1.ab"), "prefixo: {dica}");
        assert!(dica.contains("20 caracteres"), "tamanho: {dica}");
        assert!(dica.contains("github.com/settings/developers"));
        assert!(dica.contains("Client secret"), "nomeia a confusão: {dica}");
        // nenhum outro erro é reescrito
        assert_eq!(
            dica_do_client_id(error_for(401), id),
            error_for(401).to_string()
        );
    }

    #[test]
    fn a_bandeja_so_tem_item_se_a_busca_diz_o_repositorio() {
        let completo: Value = serde_json::from_str(
            r#"{"number":7,"title":"Ajuste","state":"open","html_url":"https://github.com/a/b/pull/7",
                "draft":false,"updated_at":"2026-10-10T12:00:00Z","user":{"login":"douglas"},
                "pull_request":{"url":"https://api.github.com/x"},"repository":{"full_name":"a/b"}}"#,
        )
        .unwrap();
        let item = parse_search_item(&completo).expect("item com repositório");
        assert_eq!(item.repo, "a/b");
        assert!(item.is_pull_request, "a busca marca o que é pull request");
        assert_eq!(item.author.as_deref(), Some("douglas"));

        // Um item sem `repository` não tem onde ser aberto: é descartado, e a
        // tela não recebe linha morta.
        let sem_repo: Value = serde_json::from_str(r#"{"number":8,"title":"X"}"#).unwrap();
        assert!(parse_search_item(&sem_repo).is_none());
    }

    #[test]
    fn o_detalhe_do_pr_nao_inventa_que_da_para_mergear() {
        // O GitHub responde `mergeable: null` enquanto calcula. Ler isso como
        // "falso" faria a tela bloquear um merge que está liberado.
        let calculando: Value = serde_json::from_str(
            r#"{"number":3,"title":"t","state":"open","head":{"ref":"x","sha":"abc1234"},
                "base":{"ref":"main"},"mergeable":null,"additions":4,"deletions":1,"changed_files":2}"#,
        )
        .unwrap();
        let d = parse_pull_detail(&calculando).unwrap();
        assert_eq!(d.mergeable, None, "null não é false");
        assert_eq!(d.head_sha, "abc1234");
        assert_eq!((d.additions, d.deletions, d.changed_files), (4, 1, 2));

        let liberado: Value = serde_json::from_str(
            r#"{"number":3,"title":"t","mergeable":true,"mergeable_state":"clean","draft":false,
                "head":{"ref":"x","sha":"abc1234"},"base":{"ref":"main"}}"#,
        )
        .unwrap();
        let d = parse_pull_detail(&liberado).unwrap();
        assert_eq!(d.mergeable, Some(true));
        assert_eq!(d.mergeable_state.as_deref(), Some("clean"));

        // corpo em branco não vira texto para mostrar
        let sem_corpo: Value =
            serde_json::from_str(r#"{"number":4,"title":"t","body":"   ","head":{},"base":{}}"#).unwrap();
        assert_eq!(parse_pull_detail(&sem_corpo).unwrap().body, None);
    }

    #[test]
    fn o_patch_do_pr_passa_pelo_mesmo_parser_do_diff_local() {
        let novo: Value = serde_json::from_str(
            r#"{"filename":"src/app.ts","status":"added","patch":"@@ -0,0 +1,2 @@\n+oi\n+tchau"}"#,
        )
        .unwrap();
        let arquivos = diff_de_arquivo_de_pr(&novo).unwrap();
        assert_eq!(arquivos.len(), 1);
        assert_eq!(arquivos[0].path, "src/app.ts");
        assert_eq!(arquivos[0].status_char, 'A');
        assert_eq!(arquivos[0].hunks[0].lines.len(), 2);
        assert_eq!(arquivos[0].hunks[0].lines[0].text, "oi");

        // renomeado: o nome anterior vem à esquerda do diff como no git local
        let renomeado: Value = serde_json::from_str(
            r#"{"filename":"novo.ts","previous_filename":"velho.ts","status":"renamed","patch":"@@ -1 +1 @@\n-a\n+b"}"#,
        )
        .unwrap();
        let arquivos = diff_de_arquivo_de_pr(&renomeado).unwrap();
        assert_eq!(arquivos[0].status_char, 'R');
        assert_eq!(arquivos[0].old_path.as_deref(), Some("velho.ts"));

        // sem patch (binário ou grande demais) o arquivo aparece, mas sem hunks
        // inventados.
        let binario: Value = serde_json::from_str(r#"{"filename":"logo.png","status":"added"}"#).unwrap();
        let arquivos = diff_de_arquivo_de_pr(&binario).unwrap();
        assert!(arquivos[0].hunks.is_empty(), "sem patch não há linhas para mostrar");
        assert!(arquivos[0].binary);

        let removido: Value =
            serde_json::from_str(r#"{"filename":"x.ts","status":"removed","patch":"@@ -1 +0,0 @@\n-y"}"#).unwrap();
        assert_eq!(diff_de_arquivo_de_pr(&removido).unwrap()[0].status_char, 'D');
    }

    #[test]
    fn issue_e_pull_request_se_distinguem_pelo_que_o_github_manda() {
        let pr: Value = serde_json::from_str(r#"{"number":9,"title":"t","pull_request":{"url":"x"}}"#).unwrap();
        let issue: Value = serde_json::from_str(
            r#"{"number":10,"title":"t","state":"open","labels":[{"name":"bug"},{"name":"good first issue"}],"comments":2}"#,
        )
        .unwrap();
        assert!(e_pull_request(&pr));
        assert!(!e_pull_request(&issue));
        let lida = parse_issue(&issue).unwrap();
        assert_eq!(lida.labels, vec!["bug".to_string(), "good first issue".to_string()]);
        assert_eq!(lida.comments, 2);
        assert_eq!(lida.body, None, "sem body na resposta não pode virar linha vazia");
        // a rota /issues devolve PR também; a tela de issues usa este filtro.
        assert!(parse_issue(&pr).is_some(), "PR é um objeto issue válido para a API");

        let com_corpo: Value =
            serde_json::from_str(r#"{"number":11,"title":"t","body":"  texto da  issue  "}"#).unwrap();
        assert_eq!(parse_issue(&com_corpo).unwrap().body.as_deref(), Some("texto da  issue"));
        let gigante: Value =
            serde_json::from_str(&format!(r#"{{"number":12,"title":"t","body":"{}"}}"#, "aé".repeat(3000))).unwrap();
        let cortada = parse_issue(&gigante).unwrap().body.unwrap();
        assert!(cortada.ends_with("(descrição cortada; abra no site para ler inteira)"));
        assert!(cortada.chars().count() < 4100, "corte não respeitou o limite");
    }

    #[test]
    fn revisar_e_mergear_so_aceitam_o_que_o_github_conhece() {
        assert_eq!(valid_event_review("APPROVE").unwrap(), "APPROVE");
        assert_eq!(valid_event_review("REQUEST_CHANGES").unwrap(), "REQUEST_CHANGES");
        assert_eq!(valid_event_review("COMMENT").unwrap(), "COMMENT");
        for inventado in ["approve ", "MERGE", "REJECT", ""] {
            assert!(valid_event_review(inventado).is_err(), "aceitou {inventado:?}");
        }
        for metodo in ["merge", "squash", "rebase"] {
            assert_eq!(valid_metodo_merge(metodo).unwrap(), metodo);
        }
        assert!(valid_metodo_merge("fast-forward").is_err());
    }

    #[test]
    fn o_commit_do_merge_vem_lido_e_nao_chutado() {
        assert!(sha_suficiente("abc1234"));
        assert!(sha_suficiente(
            "0e3f5a7b9c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f"
        ));
        assert!(!sha_suficiente("abc12"), "curto demais para identificar o commit");
        assert!(!sha_suficiente("main"), "nome de ramo não é commit");
        assert!(!sha_suficiente("abc123456789012345678901234567890123456789g"));
    }

    #[test]
    fn o_log_do_repositorio_nao_leva_o_que_digitaram() {
        assert_eq!(slug_para_log("DouglasGDev/devops-desktop"), "DouglasGDev/devops-desktop");
        // texto que não é `dono/repo` nunca chega ao histórico
        assert_eq!(slug_para_log("máquina/../caminho livre"), "repositório?");
    }

    #[test]
    fn o_url_da_rota_sai_do_slug_validado() {
        let url = repo_url("a/b", "/pulls/12").unwrap();
        assert_eq!(url, "https://api.github.com/repos/a/b/pulls/12");
        assert!(repo_url("qualquer coisa solta", "/issues").is_err());
        assert!(repo_url("a/b", "/issues").unwrap().ends_with("/repos/a/b/issues"));
    }

    #[test]
    fn texto_vazio_ou_enorme_nao_vira_comentario() {
        assert_eq!(corpo_texto("  ola  ", 100, "o comentário").unwrap(), "ola");
        assert!(corpo_texto("   ", 100, "o comentário").is_err());
        assert!(corpo_texto(&"a".repeat(200), 100, "o comentário").is_err());
        assert!(corpo_texto("com \0 dentro", 100, "o comentário").is_err());
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
