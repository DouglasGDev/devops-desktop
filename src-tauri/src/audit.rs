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
    // A linha é a unidade, e dentro dela só a palavra sensível é trocada: uma saída
    // de comando achatada em linha única perde as quebras que o comando produziu, e
    // o log gravado do motor é justamente essa saída. O recuo e o espaçamento de
    // quem executou ficam onde estavam.
    text.lines()
        .map(scrub_palavras)
        .collect::<Vec<_>>()
        .join("\n")
}

fn scrub_palavras(linha: &str) -> String {
    let sensitive = ["password", "passwd", "secret", "token", "apikey", "api_key", "privatekey", "private_key"];
    let mut result = String::with_capacity(linha.len());
    for chunk in linha.split_inclusive(char::is_whitespace) {
        let palavra = chunk.trim_end_matches(char::is_whitespace);
        let lower = palavra.to_lowercase();
        let is_sensitive =
            palavra.contains('=') && sensitive.iter().any(|s| lower.starts_with(s) || lower.contains(&format!("_{s}=")) || lower.contains(&format!("{s}=")));
        if is_sensitive {
            let (key, _) = palavra.split_once('=').unwrap_or((palavra, ""));
            result.push_str(key);
            result.push_str("=***");
        } else {
            result.push_str(palavra);
        }
        result.push_str(&chunk[palavra.len()..]);
    }
    result
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

    #[test]
    fn a_mascara_nao_achata_as_linhas_da_saida() {
        // Saída de comando é multi-linha por natureza. Se a máscara dobrasse tudo em
        // uma linha só, o log gravado do motor perderia exatamente a forma do que o
        // comando mostrou — recuo, colunas e as linhas em branco.
        let saida = "copiando a.html\n    copying b.html\n\n\tcabeçalho password=SuperSecret-9f3c e host ok\nfim";
        assert_eq!(
            scrub_secrets(saida),
            "copiando a.html\n    copying b.html\n\n\tcabeçalho password=*** e host ok\nfim"
        );
    }
}
