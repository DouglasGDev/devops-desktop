use rusqlite::Connection;

use crate::db::now_rfc3339;

const KNOWN_TOKEN_PREFIXES: &[&str] = &[
    "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "sk_live_", "sk_test_", "rk_live_", "xoxb-", "xoxa-", "xoxp-", "xoxr-",
    "xoxs-", "AKIA", "ASIA",
];

/// Remove padrões comuns de segredo de qualquer texto antes de gravar no log.
/// Defesa em profundidade: nenhum comando da aplicação deve sequer produzir esses
/// textos, mas se um caminho imprevisto o fizer, o valor nunca é persistido.
pub fn scrub_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for prefix in KNOWN_TOKEN_PREFIXES {
        let mut search_from = 0;
        while let Some(pos) = out[search_from..].find(prefix) {
            let start = search_from + pos;
            let mut end = start + prefix.len();
            let bytes = out.as_bytes();
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'-') {
                end += 1;
            }
            if end > start + prefix.len() {
                let replacement = format!("{prefix}***");
                out.replace_range(start..end, &replacement);
                search_from = start + replacement.len();
            } else {
                search_from = start + prefix.len();
            }
            if search_from >= out.len() {
                break;
            }
        }
    }
    // Blocos PEM de chave privada: substitui o conteúdo inteiro.
    if out.contains("-----BEGIN") && out.contains("PRIVATE KEY-----") {
        out = "[bloco de chave privada removido do log]".to_string();
    }
    // Pares chave=valor com nomes sensíveis.
    out = scrub_key_values(&out);
    out
}

fn scrub_key_values(text: &str) -> String {
    let sensitive = ["password", "passwd", "secret", "token", "apikey", "api_key", "privatekey", "private_key"];
    let mut result = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        let lower = word.to_lowercase();
        let is_sensitive = sensitive.iter().any(|s| lower.starts_with(s) || lower.contains(&format!("_{s}=")) || lower.contains(&format!("{s}=")));
        if is_sensitive && word.contains('=') {
            let (key, _) = word.split_once('=').unwrap_or((word, ""));
            result.push_str(key);
            result.push_str("=*** ");
        } else {
            result.push_str(word);
            result.push(' ');
        }
    }
    result.trim_end().to_string()
}

#[allow(clippy::too_many_arguments)]
pub fn log(
    conn: &Connection,
    level: &str,
    category: &str,
    operation: &str,
    server_name: Option<&str>,
    project_name: Option<&str>,
    exit_code: Option<i64>,
    detail: Option<&str>,
) -> Result<(), String> {
    let detail = detail.map(scrub_secrets);
    conn.execute(
        "INSERT INTO audit_log (ts, level, category, operation, server_name, project_name, exit_code, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![now_rfc3339(), level, category, operation, server_name, project_name, exit_code, detail],
    )
    .map_err(|e| format!("falha ao gravar auditoria: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_tokens_github() {
        let scrubbed = scrub_secrets("comando com ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234 no meio");
        assert!(!scrubbed.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ1234"));
        assert!(scrubbed.contains("ghp_***"));
    }

    #[test]
    fn remove_pares_sensiveis() {
        let scrubbed = scrub_secrets("conectando com password=SuperSecret e host ok");
        assert!(!scrubbed.contains("SuperSecret"));
        assert!(scrubbed.contains("password=***"));
    }

    #[test]
    fn remove_bloco_pem() {
        let scrubbed = scrub_secrets("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----");
        assert!(scrubbed.contains("removido"));
    }

    #[test]
    fn preserva_texto_comum() {
        let text = "conexao ok em 203.0.113.10:22 saida 0";
        assert_eq!(scrub_secrets(text), text);
    }
}
