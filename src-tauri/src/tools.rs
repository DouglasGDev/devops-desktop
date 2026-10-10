use crate::models::ToolVersions;

fn run_capture(program: &str, args: &[&str]) -> Option<(String, String, Option<i32>)> {
    let output = crate::proc::command(program).args(args).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Some((stdout, stderr, output.status.code()))
}

/// Extrai o login de uma linha "Logged in to …". Formatos reais:
/// `github.com account DouglasGDev (keyring)` e o antigo `github.com as octocat`.
/// Sem um dos dois marcadores a linha não afirma um login.
fn extract_login(line: &str) -> Option<String> {
    if !line.contains("Logged in to ") {
        return None;
    }
    let alvo = if let Some(i) = line.find("account ") {
        &line[i + "account ".len()..]
    } else if let Some(i) = line.find(" as ") {
        &line[i + " as ".len()..]
    } else {
        return None;
    };
    let login = alvo
        .split([' ', '\t', '(', '\n', '\r'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if login.is_empty() { None } else { Some(login) }
}

/// Saída de `gh auth status` → nome da conta ativa, ou `None` quando não há
/// sessão. A linha do token é ignorada de propósito: nunca sai daqui.
pub fn parse_gh_account(raw: &str) -> Option<String> {
    let linhas: Vec<&str> = raw.lines().collect();
    for (i, linha) in linhas.iter().enumerate() {
        let Some(login) = extract_login(linha) else {
            continue;
        };
        // Nas versões com mais de uma conta a linha seguinte diz se é a ativa.
        let flag = linhas
            .get(i + 1)
            .map(|s| s.replace([' ', '\t', '-'], ""))
            .unwrap_or_default();
        if flag.contains("Activeaccount:false") {
            continue;
        }
        return Some(login);
    }
    None
}

/// Quantas chaves o agente tem carregadas, pelo stdout do `ssh-add -l`: uma
/// linha por chave (`256 SHA256:... comentário (ed25519)`).
fn count_agent_keys(stdout: &str) -> i64 {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("no identities") && !l.contains("The agent has no"))
        .count() as i64
}

/// O `ssh-add -l` encontrou o agente? As duas respostas reais abaixo vêm com
/// código 1 ou 2 conforme a build do OpenSSH (Git para `Could not open a
/// connection…`, Windows para `Error connecting to agent: No such file…`), então
/// a classificação é feita pelo texto, que é o que não muda.
fn sem_agente(texto: &str) -> bool {
    [
        "could not open a connection",
        "error connecting to agent",
        "could not connect to",
        "unable to connect",
        "no such file or directory",
    ]
    .iter()
    .any(|marca| texto.contains(marca))
}

/// Identidade que o git tem nesta máquina. Falta qualquer uma das duas partes e
/// o commit é recusado pelo próprio git, então só há identidade com as duas.
fn identidade(nome: &str, email: &str) -> Option<String> {
    if nome.is_empty() || email.is_empty() {
        return None;
    }
    Some(format!("{nome} <{email}>"))
}

pub fn detect() -> ToolVersions {
    let ssh = run_capture("ssh", &["-V"]).map(|(out, err, _)| {
        // `ssh -V` escreve a versão na stderr, ex.: "OpenSSH_10.2p1, ...".
        let first_line = if err.trim().is_empty() { out } else { err };
        first_line
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next()
            .unwrap_or("desconhecida")
            .to_string()
    });

    let git = run_capture("git", &["--version"]).map(|(out, _, _)| {
        out.trim()
            .strip_prefix("git version ")
            .unwrap_or_else(|| out.trim())
            .to_string()
    });

    // Identidade que o git usaria para um commit nesta máquina. `git config
    // --get` resolve a configuração do usuário (e a de sistema) e o git só
    // recusa o commit quando falta qualquer uma das duas.
    let configurado = |chave: &str| {
        run_capture("git", &["config", "--get", chave])
            .map(|(out, _, code)| if code == Some(0) { out.trim().to_string() } else { String::new() })
            .unwrap_or_default()
    };
    let user_name = configurado("user.name");
    let user_email = configurado("user.email");
    let git_user = identidade(&user_name, &user_email);

    let (ssh_add, agent_running, agent_keys) = match run_capture("ssh-add", &["-l"]) {
        None => (None, None, None),
        Some((out, err, _)) => {
            let texto = format!("{out}\n{err}").to_lowercase();
            let sem_agente = sem_agente(&texto);
            (
                Some("disponível".into()),
                Some(!sem_agente),
                Some(if sem_agente { 0 } else { count_agent_keys(&out) }),
            )
        }
    };

    // `gh` é opcional: o app funciona sem ele, mas poder estar autenticado por
    // dois caminhos é informação que a tela de Acesso mostra.
    let gh = run_capture("gh", &["--version"]).map(|(out, _, _)| {
        out.trim()
            .lines()
            .next()
            .unwrap_or("")
            .strip_prefix("gh version ")
            .unwrap_or(out.trim())
            .to_string()
    });
    let gh_account = run_capture("gh", &["auth", "status"]).and_then(|(out, err, code)| {
        if code != Some(0) {
            return None;
        }
        // o texto vai para stdout e para stderr dependendo da versão do gh
        parse_gh_account(&out).or_else(|| parse_gh_account(&err))
    });

    ToolVersions {
        ssh,
        ssh_add,
        agent_running,
        agent_keys,
        git,
        git_user,
        gh,
        gh_account,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conta_ativa_do_gh_e_lida_sem_o_token() {
        let saida = "github.com\n  ✓ Logged in to github.com account DouglasGDev (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: gho_1234567890abcdef\n";
        assert_eq!(parse_gh_account(saida).as_deref(), Some("DouglasGDev"));
        // nenhum trecho do token aparece no que a função devolve
        assert!(!parse_gh_account(saida).unwrap().contains("gho_"));
    }

    #[test]
    fn formato_antigo_com_as() {
        let saida = "github.com\n  ✓ Logged in to github.com as octocat\n";
        assert_eq!(parse_gh_account(saida).as_deref(), Some("octocat"));
        // sem linha de sessão não há conta
        assert_eq!(parse_gh_account("Please run `gh auth login` to get started.\n"), None);
        // só o host, sem afirmar login, não vira conta
        assert_eq!(parse_gh_account("github.com\n  - Active account: true\n"), None);
    }

    #[test]
    fn sessao_inativa_nao_conta_como_liberacao() {
        let saida = "github.com\n  ✓ Logged in to github.com account Velho (keyring)\n  - Active account: false\n";
        assert_eq!(parse_gh_account(saida), None);
    }

    #[test]
    fn contagem_de_chaves_do_agente() {
        let duas = "/c/Users/dou/.ssh/id_ed25519 (ecdsa-sha2-nistp256)\n/home/u/.ssh/id_rsa (RSA)\n";
        assert_eq!(count_agent_keys(duas), 2);
        assert_eq!(count_agent_keys("The agent has no identities.\n"), 0);
        assert_eq!(count_agent_keys(""), 0);
    }

    #[test]
    fn agente_parado_e_reconhecido_pelo_texto_das_duas_builds() {
        // respostas reais capturadas nesta máquina (código 1 ou 2 conforme a build)
        assert!(sem_agente("could not open a connection to your authentication agent."));
        assert!(sem_agente("error connecting to agent: no such file or directory"));
        // uma listagem verdadeira não é confundida com agente ausente
        assert!(!sem_agente("256 sha256:abc… douglas@notebook (ed25519)\n"));
        assert!(!sem_agente("the agent has no identities."));
    }

    #[test]
    fn identidade_precisa_do_nome_e_do_email() {
        assert_eq!(
            identidade("Douglas", "d@exemplo.com").as_deref(),
            Some("Douglas <d@exemplo.com>")
        );
        // falta qualquer metade e o git recusa o commit
        assert_eq!(identidade("Douglas", ""), None);
        assert_eq!(identidade("", "d@exemplo.com"), None);
    }
}
