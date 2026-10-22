use std::{fs, path::Path};

use serde::Serialize;

use crate::git;

/// Limite de leitura para a checagem de conteúdo. Arquivos maiores são
/// julgados apenas pelo nome. É público porque a análise de código lê exatamente
/// os mesmos bytes: um arquivo que escapa de uma varredura escapa da outra.
pub const MAX_CONTENT_BYTES: u64 = 2 * 1024 * 1024;
pub const PROBE_BYTES: usize = 8 * 1024;

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

/// Prefixos que denunciam um token de provedor. A máscara da análise e a checagem
/// de commit lêem esta mesma lista: o que sai da tela como «valor mascarado» é o
/// mesmo que bloqueia um commit.
const PREFIXOS_TOKEN: [&str; 13] = [
    "github_pat_", "ghp_", "gho_", "ghu_", "ghs_", "xoxb-", "xoxp-", "AKIA", "sk_live_", "sk_test_", "sk-ant-",
    "sk-proj-", "glpat-",
];

const BLOCK_EXTENSIONS: [&str; 17] = [
    "key", "pem", "p8", "p12", "pfx", "jks", "keystore", "ks", "ppk", "kdbx", "pwd", "crt.bak", "kdb", "asc", "gpg",
    "snk", "pypirc",
];

/// Pastas de cache, dependências e build: não devem estar no índice do Git.
pub const NOISE_DIRS: [&str; 12] = [
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

/// A única regra de "esta pasta não é código do projeto". Quem varre árvore no
/// aplicativo usa isto — a checagem de segredo e a análise de código pulam as
/// mesmas pastas porque é a mesma árvore sendo lida.
pub fn pular_dir(nome: &str) -> bool {
    let base = lower(nome);
    base == ".git" || NOISE_DIRS.contains(&base.as_str())
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

/// As chaves cuja atribuição denuncia um segredo. A detecção e a máscara leem
/// esta mesma lista e chegam aos mesmos intervalos: o que barra um commit é
/// exatamente o que some antes de qualquer gravação.
const CHAVES_DE_SEGREDO: [&str; 7] = [
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "access_key",
    "auth_token",
];

/// O texto que toma o lugar de um valor.
const MARCA_DE_MASCARA: &str = "«valor mascarado»";

/// Uma atribuição de segredo: a chave que a denunciou e o intervalo [início, fim)
/// do valor na linha que chegou. Os deslocamentos são bytes do texto original, e é
/// por isso que nada aqui vira a linha em minúsculas: em Unicode a caixa pode mudar
/// o número de bytes, e um deslocamento vindo da cópia cortaria o valor no meio de
/// um caractere — ou faria a máscara desistir, como fazia diante de um `İ`.
struct Atribuicao {
    chave: &'static str,
    inicio: usize,
    fim: usize,
}

/// A próxima ocorrência de `chave` a partir do byte `de`, ignorando a caixa ASCII.
/// Feito à mão porque o projeto não tem a dependency `regex` e porque a busca
/// precisa devolver a posição no texto original.
fn ocorrencia(bytes: &[u8], chave: &[u8], de: usize) -> Option<usize> {
    if chave.is_empty() || bytes.len() < de + chave.len() {
        return None;
    }
    (de..=bytes.len() - chave.len()).find(|i| bytes[*i..*i + chave.len()].eq_ignore_ascii_case(chave))
}

/// Todas as posições de `chave` na linha.
fn ocorrencias(linha: &str, chave: &[u8]) -> Vec<usize> {
    let bytes = linha.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(i) = ocorrencia(bytes, chave, cursor) {
        out.push(i);
        cursor = i + chave.len();
    }
    out
}

/// Onde termina a cauda de um token conhecido: no primeiro byte que não é de token.
fn fim_do_token(linha: &str, de: usize) -> usize {
    linha[de..]
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
        .map(|(i, _)| de + i)
        .unwrap_or(linha.len())
}

/// Onde termina um valor solto — o que vem depois de `Bearer` ou de um `=` sem
/// aspa. Caminhar por `char_indices` é o que garante o corte numa borda de
/// caractere num texto com acento.
fn fim_do_valor(linha: &str, de: usize) -> usize {
    linha[de..]
        .char_indices()
        .find(|(_, c)| {
            c.is_whitespace()
                || matches!(c, '"' | '\'' | ',' | ';' | '#' | '(' | ')' | '[' | ']' | '{' | '}')
        })
        .map(|(i, _)| de + i)
        .unwrap_or(linha.len())
}

/// O byte que fecha um valor entre aspas, ou o fim da linha quando a aspa não
/// fecha: um valor cortado no meio do arquivo não é motivo para mostrá-lo.
fn fim_do_valor_entre_aspas(linha: &str, de: usize, aspa: u8) -> usize {
    let bytes = linha.as_bytes();
    let mut i = de;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == aspa => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// O que já é a marca não é um valor. Sem isto, mascarar duas vezes escreveria
/// um segundo marcador colado no fim do primeiro.
fn esta_mascarado(linha: &str, de: usize) -> bool {
    linha[de..].starts_with(MARCA_DE_MASCARA)
}

/// O intervalo do valor que uma chave de segredo está recebendo, olhando a partir
/// do byte logo depois da chave. Não é valor quando:
/// * o sinal é uma comparação (`==`) — o texto à direita é um teste, não um segredo;
/// * o texto é uma referência (`$X`, `${X}`) — o valor mora no ambiente;
/// * o texto está solto e é curto (`password = x`) — um nome de variável, não um
///   literal.
///
/// Entre aspas o valor é tudo o que está entre aspas, **inclusive quando tem
/// espaços**: é a forma de uma frase-senha, e a máscara antiga parava no primeiro
/// espaço e deixava o resto à vista.
fn valor_de_uma_atribuicao(linha: &str, apos_chave: usize) -> Option<(usize, usize)> {
    let bytes = linha.as_bytes();
    let mut i = apos_chave;
    // a aspa que fecha o nome num JSON ou num array PHP: `"password": "…"`;
    if matches!(bytes.get(i), Some(b'"') | Some(b'\'')) {
        i += 1;
    }
    while matches!(bytes.get(i), Some(b' ') | Some(b'\t')) {
        i += 1;
    }
    let sinal = *bytes.get(i)?;
    if !matches!(sinal, b'=' | b':') {
        return None;
    }
    let mut j = i + 1;
    if sinal == b'=' && matches!(bytes.get(j), Some(b'=') | Some(b'~')) {
        return None;
    }
    while matches!(bytes.get(j), Some(b' ') | Some(b'\t')) {
        j += 1;
    }
    if esta_mascarado(linha, j) {
        return None;
    }
    match bytes.get(j)? {
        b'$' => None,
        aspa @ (b'"' | b'\'') => {
            let inicio = j + 1;
            let fim = fim_do_valor_entre_aspas(linha, inicio, *aspa);
            (fim > inicio).then_some((inicio, fim))
        }
        _ => {
            let fim = fim_do_valor(linha, j);
            let corpo = &linha[j..fim];
            (corpo.chars().count() >= 6).then_some((j, fim))
        }
    }
}

/// Todas as atribuições de segredo de uma linha, na ordem. Uma linha pode ter duas
/// (`password = …; secret = …`): a detecção antiga devolvia no máximo uma chave —
/// parava na primeira que não existia na linha, e na primeira que existia sem se
/// qualificar — então as outras ficavam invisíveis para a checagem **e** para a
/// máscara.
fn atribuicoes(linha: &str) -> Vec<Atribuicao> {
    let bytes = linha.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let Some((pos, chave)) = CHAVES_DE_SEGREDO
            .iter()
            .filter_map(|c| ocorrencia(bytes, c.as_bytes(), cursor).map(|i| (i, *c)))
            .min_by_key(|(i, _)| *i)
        else {
            break;
        };
        let depois = pos + chave.len();
        match valor_de_uma_atribuicao(linha, depois) {
            Some((inicio, fim)) => {
                out.push(Atribuicao { chave, inicio, fim });
                cursor = fim.max(depois);
            }
            None => cursor = depois,
        }
    }
    out
}

/// Os intervalos de uma linha que são valor de segredo: atribuições, cabeçalhos
/// `Bearer …` e caudas de token conhecido, fundidos e sem sobreposição. Uma
/// passada só sobre o texto original — é assim que uma máscara não corta a outra
/// no meio do marcador, em vez de depender da ordem de três passadas.
///
/// O prefixo de um token conhecido fica de fora: ele diz a espécie do segredo e não
/// é segredo, então uma máscara que o cobriria se divide ao redor dele.
fn intervalos_a_esconder(linha: &str) -> Vec<(usize, usize)> {
    let mut escondidos: Vec<(usize, usize)> = Vec::new();
    let mut a_vista: Vec<(usize, usize)> = Vec::new();

    for a in atribuicoes(linha) {
        escondidos.push((a.inicio, a.fim));
    }
    // `Bearer …`: a palavra fica, o que vem depois dela não — pode ser um token
    // opaco, sem prefixo de espécie nenhuma para a máscara reconhecer.
    for i in ocorrencias(linha, b"bearer ") {
        let inicio = i + 7;
        if esta_mascarado(linha, inicio) {
            continue;
        }
        let fim = fim_do_valor(linha, inicio);
        if fim > inicio {
            escondidos.push((inicio, fim));
        }
    }
    for prefixo in PREFIXOS_TOKEN {
        for i in ocorrencias(linha, prefixo.as_bytes()) {
            let inicio = i + prefixo.len();
            let fim = fim_do_token(linha, inicio);
            if fim > inicio {
                escondidos.push((inicio, fim));
            }
            a_vista.push((i, inicio));
        }
    }

    escondidos.sort_unstable();
    let mut fundidos: Vec<(usize, usize)> = Vec::new();
    for (inicio, fim) in escondidos {
        match fundidos.last_mut() {
            Some(u) if inicio <= u.1 => u.1 = u.1.max(fim),
            _ => fundidos.push((inicio, fim)),
        }
    }
    a_vista.sort_unstable();

    let mut saida = Vec::new();
    for (inicio, fim) in fundidos {
        let mut corte = inicio;
        for &(de, ate) in &a_vista {
            if ate <= corte || de >= fim {
                continue;
            }
            if de > corte {
                saida.push((corte, de));
            }
            corte = ate.max(corte);
        }
        if corte < fim {
            saida.push((corte, fim));
        }
    }
    saida
}

/// O valor de um segredo nunca deixa esta função. A linha mascarada é o que vai
/// para o banco, para a tela e para o pacote que uma IA lê — é a mesma regra da
/// checagem de segredo, aplicada antes de qualquer texto de arquivo ser guardado.
pub fn mascarar(line: &str) -> String {
    let intervalos = intervalos_a_esconder(line);
    if intervalos.is_empty() {
        return line.to_string();
    }
    let mut saida = String::with_capacity(line.len());
    let mut cursor = 0usize;
    for (inicio, fim) in intervalos {
        saida.push_str(&line[cursor..inicio]);
        saida.push_str(MARCA_DE_MASCARA);
        cursor = fim;
    }
    saida.push_str(&line[cursor..]);
    saida
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
        for at in ocorrencias(line, b"AKIA") {
            let tail: String = line[at + 4..].chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).collect();
            if tail.len() >= 16 {
                push(
                    "conteudo",
                    "contém uma chave de acesso AWS (AKIA…)".into(),
                    "revogue a chave no provedor e substitua por uma referência de ambiente".into(),
                );
            }
        }
        for prefix in PREFIXOS_TOKEN.iter().filter(|p| **p != "AKIA") {
            for at in ocorrencias(line, prefix.as_bytes()) {
                let cauda = &line[at + prefix.len()..fim_do_token(line, at + prefix.len())];
                if cauda.len() >= 12 {
                    push(
                        "conteudo",
                        format!("contém um token com o prefixo {prefix}"),
                        "revogue o token e use um segredo gerenciado fora do repositório".into(),
                    );
                }
            }
        }
        for at in ocorrencias(line, b"bearer ") {
            let inicio = at + 7;
            let tail = &line[inicio..fim_do_valor(line, inicio)];
            if tail.chars().count() >= 12 {
                push(
                    "conteudo",
                    "traz uma credencial no cabeçalho Bearer".into(),
                    "revogue a credencial e monte o cabeçalho a partir de um segredo de ambiente".into(),
                );
            }
        }
        for a in atribuicoes(line) {
            push(
                "conteudo",
                format!("atribui um valor a “{}” no arquivo", a.chave),
                "mova o valor para uma variável de ambiente ou arquivo fora do repositório".into(),
            );
        }
    }
    out
}

/// Mesma amostra da checagem de segredo: um arquivo binário não é código lido.
/// Público porque a análise de código varre a mesma árvore com a mesma régua.
pub fn he_texto(head: &[u8]) -> bool {
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
    if !he_texto(head) {
        return out;
    }
    push_content(&mut out, rel, bytes);
    out
}

/// O texto de um arquivo de código, lido com a régua compartilhada: arquivo
/// comum, dentro do limite de bytes e sem byte nulo no começo da amostra. `None`
/// é "não foi lido" — pasta, grande demais, binário ou ilegível — e quem varre
/// decide o que fazer com cada um desses casos.
pub fn texto_do_arquivo(abs: &Path) -> Option<Vec<u8>> {
    let meta = fs::metadata(abs).ok()?;
    if !meta.is_file() || meta.len() > MAX_CONTENT_BYTES {
        return None;
    }
    let data = fs::read(abs).ok()?;
    if !he_texto(&data[..data.len().min(PROBE_BYTES)]) {
        return None;
    }
    Some(data)
}

/// Analisa um arquivo do disco. `rel` é o caminho relativo mostrado ao usuário.
pub fn inspect_file(abs: &Path, rel: &str, extra: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(f) = name_finding(rel, extra) {
        out.push(f);
        // nome já bloqueia; a leitura do conteúdo ainda é útil quando o arquivo é pequeno
    }
    let Some(data) = texto_do_arquivo(abs) else { return out };
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
                if pular_dir(&name) {
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
/// o que recomendamos, não um motivo para bloquear. O que **não** é ignorado: a
/// leitura que não respondeu, que devolve erro em vez de uma varredura limpa.
pub fn index_findings(dir: &Path, extra: &[String]) -> Result<Vec<Finding>, String> {
    let mut out = Vec::new();
    for rel in git::staged_files(dir)? {
        if let Some(blob) = git::index_blob(dir, &rel)? {
            out.extend(inspect_bytes(&rel.replace('\\', "/"), &blob, extra));
        }
    }
    Ok(summarize(out))
}

/// Segredos no conteúdo que um push publicaria: os caminhos escolhidos lidos na
/// revisão HEAD, que é a versão que seguiria para o remoto. Err aqui quer dizer
/// que o git não respondeu — quem chama interrompe a operação em vez de assumir
/// que não havia nada para checar.
pub fn head_findings(dir: &Path, rels: &[String], extra: &[String]) -> Result<Vec<Finding>, String> {
    let mut out = Vec::new();
    for rel in rels {
        if let Some(blob) = git::blob_at(dir, "HEAD", rel)? {
            out.extend(inspect_bytes(&rel.replace('\\', "/"), &blob, extra));
        }
    }
    Ok(summarize(out))
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
    fn a_mascara_esconde_o_token_que_a_tela_mostra() {
        // A linha medida na janela real: um comando de shell com o cabeçalho de
        // autorização dentro. O achado é legítimo, o valor do token não. O valor
        // escrito aqui é visivelmente falso de propósito: um fixture com forma de
        // credencial real é recusado pelo protetor de push do GitHub, e a máscara não
        // depende da entropia do valor — depende do formato em volta dele.
        let linha = "    subprocess.call(\"curl -H 'Authorization: Bearer sk_live_NAO-E-UMA-CHAVE-REAL-2026' https://api.example\", shell=True)";
        let saida = mascarar(linha);
        assert!(saida.contains("Bearer "), "{saida}");
        assert!(saida.contains("«valor mascarado»"), "{saida}");
        assert!(!saida.contains("NAO-E-UMA-CHAVE-REAL-2026"), "{saida}");
        // Uma passada não corta a outra no meio do marcador.
        assert!(!saida.contains("mascarado» mascarado"), "{saida}");
    }

    #[test]
    fn o_prefixo_fica_a_vista_e_a_cauda_nao() {
        let saida = mascarar("stripe = \"sk_live_NAO-E-UMA-CHAVE-REAL-2026\"");
        assert!(saida.contains("sk_live_"), "{saida}");
        assert!(!saida.contains("NAO-E-UMA-CHAVE-REAL-2026"), "{saida}");
        let opaco = mascarar("curl -H 'Authorization: Bearer um-token-opaco-sem-prefixo-de-especie'");
        assert!(opaco.contains("Bearer «valor mascarado»"), "{opaco}");
        assert!(!opaco.contains("um-token-opaco"), "{opaco}");
    }

    #[test]
    fn toda_chave_de_segredo_tem_o_valor_escondido() {
        // A detecção antiga parava na primeira chave que não existia na linha: só
        // `password` era vista, e `secret = FICTICIO…` passava pela tela e pelo banco
        // com o valor à vista. As sete chaves têm de se comportar igual.
        for chave in CHAVES_DE_SEGREDO {
            let linha = format!("{chave} = FICTICIO_VALOR_DE_TESTE");
            let saida = mascarar(&linha);
            assert!(!saida.contains("FICTICIO_VALOR_DE_TESTE"), "{chave}: {saida}");
            assert!(saida.contains(MARCA_DE_MASCARA), "{chave}: {saida}");
            let found = content_findings("app.conf", format!("{linha}\n").as_bytes());
            assert!(
                found.iter().any(|f| f.reason.contains(chave)),
                "{chave} não barra: {found:?}"
            );
        }
    }

    #[test]
    fn uma_linha_pode_ter_mais_de_um_valor() {
        let saida = mascarar("password = FICTICIO_UM; secret = FICTICIO_DOIS");
        assert!(!saida.contains("FICTICIO_UM"), "{saida}");
        assert!(!saida.contains("FICTICIO_DOIS"), "{saida}");
        assert_eq!(saida.matches(MARCA_DE_MASCARA).count(), 2, "{saida}");
        let found = content_findings(
            "app.conf",
            b"password = FICTICIO_UM; secret = FICTICIO_DOIS\n",
        );
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn o_valor_entre_aspas_some_inteiro() {
        // Uma frase-senha tem espaços. O valor antigo cortava no primeiro e deixava
        // o resto legível no trecho gravado.
        let saida = mascarar("password = \"FICTICIO_UM FICTICIO_DOIS FICTICIO_TRES\"");
        assert!(!saida.contains("FICTICIO"), "{saida}");
        assert!(saida.starts_with("password = \""), "{saida}");
        assert!(saida.ends_with('"'), "{saida}");
        // aspas simples, e JSON com o nome entre aspas, são a mesma forma
        assert!(!mascarar("password = 'FICTICIO_UM FICTICIO_DOIS'").contains("FICTICIO"));
        let json = mascarar("{ \"password\": \"FICTICIO_UM FICTICIO_DOIS\" }");
        assert!(!json.contains("FICTICIO"), "{json}");
    }

    #[test]
    fn a_mascara_nao_desiste_diante_de_unicode() {
        // `İ` minúsculo ocupa dois bytes a mais: a máscara antiga comparava o
        // tamanho da cópia em minúsculas com o do original e devolvia a linha
        // inteira sem esconder nada.
        let bearer = mascarar("İ Bearer FICTICIO_TOKEN_OPACO_123");
        assert!(bearer.contains("Bearer "), "{bearer}");
        assert!(!bearer.contains("FICTICIO_TOKEN"), "{bearer}");
        let atribuicao = mascarar("İnfo: password = FICTICIO_SENHA_COM_ACENTUAÇÃO_çã");
        assert!(!atribuicao.contains("FICTICIO"), "{atribuicao}");
        assert!(atribuicao.contains("İnfo"), "{atribuicao}");
    }

    #[test]
    fn o_que_nao_e_valor_fica_a_vista() {
        // Esconder tudo não é esconder segredo: uma referência de ambiente, um nome
        // de variável, uma comparação e um texto comum têm de continuar legíveis —
        // senão a tela vira ruído e ninguém confia nela. Um `Bearer ` seguido de uma
        // palavra qualquer, esse é escondido: a forma de um token e a de uma palavra
        // não se distinguem aqui, e o erro de segurança é o único dos dois que custa.
        for linha in [
            "password = ${DB_PASSWORD}",
            "secret = $SEGREDO",
            "password = \"\"",
            "if password == hash_antiga {",
            "print(password)",
            "a senha do sistema usa password_hash()",
        ] {
            let saida = mascarar(linha);
            assert_eq!(saida, linha, "escondeu o que não é valor");
            let found = content_findings("app.conf", format!("{linha}\n").as_bytes());
            assert!(
                found.iter().all(|f| !f.reason.contains("valor a")),
                "bloqueou o que não é valor: {linha} → {found:?}"
            );
        }
    }

    #[test]
    fn a_mascara_e_idempotente() {
        // A linha mascarada pode passar pela máscara de novo (log, auditoria,
        // reexportação). Uma segunda passada não pode escrever um segundo marcador
        // colado no primeiro nem descobrir o que a primeira escondeu.
        for linha in [
            "password = \"FICTICIO_UM FICTICIO_DOIS\"",
            "curl -H 'Authorization: Bearer FICTICIO_TOKEN_OPACO_12345' https://api.example",
            "auth_token = FICTICIO_VALOR_SOLTO_123",
            "secret = 'FICTICIO_UM'; api_key = \"FICTICIO_DOIS COM ESPACO\"",
            "stripe = \"sk_live_NAO-E-UMA-CHAVE-REAL-2026\"",
        ] {
            let uma_vez = mascarar(linha);
            let duas_vezes = mascarar(&uma_vez);
            assert_eq!(uma_vez, duas_vezes, "a segunda passada mudou a linha");
            assert!(!duas_vezes.contains("FICTICIO"), "{duas_vezes}");
        }
    }

    #[test]
    fn o_prefixo_de_token_continua_dizendo_a_especie() {
        // Dentro de um valor entre aspas o prefixo também fica: a máscara se divide
        // ao redor dele em vez de cobri-lo.
        let saida = mascarar("auth = \"ghp_FICTICIO_CAUDA_123456\"");
        assert!(saida.contains("ghp_"), "{saida}");
        assert!(!saida.contains("FICTICIO_CAUDA"), "{saida}");
    }

    #[test]
    fn um_bearer_no_arquivo_bloqueia_o_commit() {
        let bruto = "HEADERS = {'Authorization': 'Bearer um-token-opaco-sem-prefixo-de-especie'}\n".as_bytes();
        let found = content_findings("app.py", bruto);
        assert!(found.iter().any(|f| f.reason.contains("Bearer")), "{found:?}");

        // um texto comum que só menciona o esquema não bloqueia
        let doc = "o cliente envia Bearer vazio quando não está autenticado\n".as_bytes();
        let found = content_findings("README.md", doc);
        assert!(!found.iter().any(|f| f.reason.contains("Bearer")), "{found:?}");
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
        let no_push = head_findings(&dir, &["srv/.env".into(), "src/app.rs".into()], &extra).unwrap();
        assert!(blocks(&no_push), "{no_push:?}");

        // um segredo que o commit apenas retira do índice não é motivo de bloqueio
        git::run(&dir, &["rm", "--cached", "-q", "--", "srv/.env"]).unwrap();
        let found = index_findings(&dir, &extra).unwrap();
        assert!(!found.iter().any(|f| f.path == "srv/.env"), "{found:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uma_leitura_impossivel_para_a_varredura_em_vez_de_devolver_limpo() {
        // O que está em jogo: quando a leitura de um caminho não pode ser feita
        // (não respondeu a tempo, ou o git não aceita o caminho), a varredura
        // **para** com o erro. Ela não devolve a lista do que conseguiu ler como
        // se fosse o veredito "sem segredos" — foi assim que um disco externo
        // silencioso viraria commit liberado.
        let dir = tmp("leitura-impossivel");
        git::run(&dir, &["-c", "init.defaultBranch=main", "init", "-q", "."]).unwrap();
        git::run(&dir, &["config", "user.name", "Teste"]).unwrap();
        git::run(&dir, &["config", "user.email", "teste@local"]).unwrap();
        git::run(&dir, &["config", "commit.gpgsign", "false"]).unwrap();
        fs::write(dir.join("app.rs"), "fn main() {}\n").unwrap();
        git::run(&dir, &["add", "-A"]).unwrap();
        git::run(&dir, &["commit", "-q", "-m", "base"]).unwrap();

        let extra: Vec<String> = Vec::new();
        let limpo = head_findings(&dir, &["app.rs".into()], &extra).unwrap();
        assert!(!limpo.iter().any(|f| f.path == "app.rs"), "{limpo:?}");

        let erro = head_findings(&dir, &["app.rs".into(), "../fora.env".into()], &extra)
            .expect_err("a leitura que não pôde ser feita interrompe a varredura");
        assert!(erro.contains(".."), "{erro}");
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
