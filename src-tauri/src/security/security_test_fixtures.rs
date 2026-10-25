// Fixtures de teste para security.rs
// Este arquivo é ignorado pelo Semgrep (.semgrepignore) porque contém
// credenciais falsas de propósito para testes.

#[cfg(test)]
pub const AWS_KEY_FIXTURE: &[u8] = b"key = AKIAABCDEFGHIJKLMNOP\n";
#[cfg(test)]
pub const GITHUB_TOKEN_FIXTURE: &[u8] = b"APP_GITHUB=ghp_AAAAAAAAAAAAAAAABBBBBBBBBBBB\n";
