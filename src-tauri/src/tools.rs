use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::models::{EditorInstalado, ToolVersions};

/// Prazo de cada sonda. O que não responde dentro dele é dito "sem resposta" na
/// tela: a janela nunca fica esperando um processo filho sem fim.
const PRAZO: Duration = Duration::from_secs(5);

/// O que uma chamada de linha de comando devolveu nesta máquina.
#[derive(Debug, PartialEq)]
enum Sonda {
    /// Executou. `code` é o código de saída — ausente quando o processo morreu
    /// por sinal em vez de terminar sozinho.
    Respondeu {
        stdout: String,
        stderr: String,
        code: Option<i32>,
    },
    /// Não deu para executar: o nome não está no PATH ou o sistema recusou.
    Ausente,
    /// Rodou e não terminou dentro do prazo.
    SemResposta,
}

impl Default for Sonda {
    /// Uma sonda da qual não chegou resposta — inclusive porque a thread que a
    /// rodava morreu — é "sem resposta", e não "ausente".
    fn default() -> Self {
        Sonda::SemResposta
    }
}

fn sonda(programa: &str, args: &[&str]) -> Sonda {
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
    let feito = crate::sshexec::collect(&mut child, PRAZO, &AtomicBool::new(false));
    if feito.timed_out || feito.cancelled {
        return Sonda::SemResposta;
    }
    Sonda::Respondeu {
        stdout: feito.stdout,
        stderr: feito.stderr,
        code: feito.exit_code,
    }
}

impl Sonda {
    /// stdout e código de saída de quem respondeu, se respondeu.
    fn saida(&self) -> Option<(&str, Option<i32>)> {
        match self {
            Sonda::Respondeu { stdout, code, .. } => Some((stdout.as_str(), *code)),
            _ => None,
        }
    }
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
    if login.is_empty() {
        None
    } else {
        Some(login)
    }
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
        .filter(|l| {
            !l.is_empty() && !l.contains("no identities") && !l.contains("The agent has no")
        })
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

fn versao_ssh(r: &Sonda) -> Option<String> {
    let (out, err) = match r {
        Sonda::Respondeu { stdout, stderr, .. } => (stdout.as_str(), stderr.as_str()),
        _ => return None,
    };
    // `ssh -V` escreve a versão na stderr, ex.: "OpenSSH_10.2p1, LibreSSL 3.9.2".
    let primeira = if err.trim().is_empty() { out } else { err };
    Some(
        primeira
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next()
            .unwrap_or("desconhecida")
            // a vírgula que separa a versão da biblioteca não é versão nenhuma
            .trim_end_matches(',')
            .to_string(),
    )
}

fn versao_git(r: &Sonda) -> Option<String> {
    let (out, _) = r.saida()?;
    let out = out.trim();
    Some(out.strip_prefix("git version ").unwrap_or(out).to_string())
}

fn versao_gh(r: &Sonda) -> Option<String> {
    let (out, _) = r.saida()?;
    let primeira = out.trim().lines().next().unwrap_or("");
    Some(
        primeira
            .strip_prefix("gh version ")
            .unwrap_or(primeira)
            .to_string(),
    )
}

/// Valor de um `git config --get`: vazio quando o git não respondeu com sucesso,
/// que é também o caso de a chave não existir.
fn configurado(r: &Sonda) -> String {
    match r.saida() {
        Some((out, Some(0))) => out.trim().to_string(),
        _ => String::new(),
    }
}

/// Agente: se o `ssh-add` existe, se o agente está de pé e quantas chaves ele
/// tem. As três respostas vêm da mesma chamada.
fn estado_agente(r: &Sonda) -> (Option<String>, Option<bool>, Option<i64>) {
    let Sonda::Respondeu { stdout, stderr, .. } = r else {
        return (None, None, None);
    };
    let parado = sem_agente(&format!("{stdout}\n{stderr}").to_lowercase());
    (
        Some("disponível".into()),
        Some(!parado),
        Some(if parado { 0 } else { count_agent_keys(stdout) }),
    )
}

fn conta_gh(r: &Sonda) -> Option<String> {
    match r {
        // o texto vai para stdout e para stderr dependendo da versão do gh
        Sonda::Respondeu {
            stdout,
            stderr,
            code: Some(0),
        } => parse_gh_account(stdout).or_else(|| parse_gh_account(stderr)),
        _ => None,
    }
}

/// Nomes dos programas que não responderam, sem repetição. O mesmo programa
/// sondado três vezes não vira três avisos na tela.
fn sem_resposta_de(probes: &[(&str, &Sonda)]) -> Vec<String> {
    let mut nomes: Vec<String> = Vec::new();
    for (programa, r) in probes {
        if matches!(r, Sonda::SemResposta) && !nomes.iter().any(|n| n == programa) {
            nomes.push((*programa).to_string());
        }
    }
    nomes
}

/// Editores que a aplicação sabe abrir. A lista oferecida na Configurações vem
/// do que existe neste computador: o que não for encontrado não é oferecido.
struct Candidato {
    id: &'static str,
    nome: &'static str,
    /// nomes de executável procurados dentro das pastas do PATH
    no_path: &'static [&'static str],
    /// subpastas procuradas dentro dos diretórios-base de instalação
    em_base: &'static [&'static str],
    /// caminhos absolutos fora das bases (Linux e macOS)
    absolutos: &'static [&'static str],
}

const CANDIDATOS: &[Candidato] = &[
    Candidato {
        id: "vscode",
        nome: "Visual Studio Code",
        no_path: &["Code.exe", "code.exe", "code.cmd", "code"],
        em_base: &[
            "Programs/Microsoft VS Code/Code.exe",
            "Microsoft VS Code/Code.exe",
        ],
        absolutos: &[
            "/usr/share/code/code",
            "/usr/bin/code",
            "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
        ],
    },
    Candidato {
        id: "cursor",
        nome: "Cursor",
        no_path: &["Cursor.exe", "cursor.exe", "cursor.cmd", "cursor"],
        em_base: &["Programs/cursor/Cursor.exe", "cursor/Cursor.exe"],
        absolutos: &[
            "/usr/bin/cursor",
            "/opt/cursor/cursor",
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
        ],
    },
];

/// Pastas onde um instalador de editor costuma colocar o executável. Só entram
/// as variáveis que têm valor: ambiente vazio não inventa caminho.
fn bases_de_instalacao(ler: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    for chave in [
        "LOCALAPPDATA",
        "ProgramFiles",
        "ProgramW6432",
        "ProgramFiles(x86)",
    ] {
        if let Some(valor) = ler(chave) {
            let valor = valor.trim();
            if !valor.is_empty() {
                bases.push(PathBuf::from(valor));
            }
        }
    }
    if let Some(casa) = ler("HOME") {
        let casa = casa.trim();
        if !casa.is_empty() {
            bases.push(PathBuf::from(casa).join(".local/share"));
        }
    }
    bases
}

/// Primeiro executável com um destes nomes dentro das pastas do PATH.
fn no_path(path_var: &str, nomes: &[&str]) -> Option<String> {
    let separador = if cfg!(windows) { ';' } else { ':' };
    for pasta in path_var.split(separador) {
        let pasta = pasta.trim();
        if pasta.is_empty() {
            continue;
        }
        for nome in nomes {
            // No Windows só um `.exe` é executável: as pastas `bin` do VS Code e
            // do Cursor põem no PATH um script de shell sem extensão, que a
            // aplicação não consegue abrir e que não é prova de instalação.
            if cfg!(windows) && !nome.ends_with(".exe") {
                continue;
            }
            let candidato = Path::new(pasta).join(nome);
            if candidato.is_file() {
                return Some(candidato.display().to_string());
            }
        }
    }
    None
}

/// Localiza os editores instalados olhando só o sistema de arquivos: nada aqui
/// executa o editor, então não há como travar a detecção.
fn localizar(ler: &dyn Fn(&str) -> Option<String>) -> Vec<EditorInstalado> {
    let path = ler("PATH").unwrap_or_default();
    let bases = bases_de_instalacao(ler);
    CANDIDATOS
        .iter()
        .filter_map(|c| {
            let caminho = no_path(&path, c.no_path)
                .or_else(|| {
                    bases.iter().find_map(|base| {
                        c.em_base
                            .iter()
                            .map(|sub| base.join(sub))
                            .find(|p| p.is_file())
                            .map(|p| p.display().to_string())
                    })
                })
                .or_else(|| {
                    c.absolutos
                        .iter()
                        .find(|p| Path::new(p).is_file())
                        .map(|s| s.to_string())
                })?;
            Some(EditorInstalado {
                id: c.id.to_string(),
                nome: c.nome.to_string(),
                caminho,
            })
        })
        .collect()
}

pub fn detectar_editores() -> Vec<EditorInstalado> {
    localizar(&|chave| std::env::var(chave).ok())
}

pub fn detect() -> ToolVersions {
    // As sondas correm em paralelo: o total deixa de ser a soma dos tempos e uma
    // ferramenta lenta não atrasa as outras.
    std::thread::scope(|escopo| {
        let ssh = escopo.spawn(|| sonda("ssh", &["-V"]));
        let git = escopo.spawn(|| sonda("git", &["--version"]));
        let nome = escopo.spawn(|| sonda("git", &["config", "--get", "user.name"]));
        let email = escopo.spawn(|| sonda("git", &["config", "--get", "user.email"]));
        let agente = escopo.spawn(|| sonda("ssh-add", &["-l"]));
        let gh = escopo.spawn(|| sonda("gh", &["--version"]));
        let sessao = escopo.spawn(|| sonda("gh", &["auth", "status"]));

        let ssh = ssh.join().unwrap_or_default();
        let git = git.join().unwrap_or_default();
        let nome = nome.join().unwrap_or_default();
        let email = email.join().unwrap_or_default();
        let agente = agente.join().unwrap_or_default();
        let gh = gh.join().unwrap_or_default();
        let sessao = sessao.join().unwrap_or_default();

        let sem_resposta = sem_resposta_de(&[
            ("ssh", &ssh),
            ("git", &git),
            ("git", &nome),
            ("git", &email),
            ("ssh-add", &agente),
            ("gh", &gh),
            ("gh", &sessao),
        ]);
        let (ssh_add, agent_running, agent_keys) = estado_agente(&agente);
        // `git config --get` resolve a configuração do usuário (e a de sistema) e
        // o git só recusa o commit quando falta qualquer uma das duas partes.
        let git_user = identidade(&configurado(&nome), &configurado(&email));

        ToolVersions {
            ssh: versao_ssh(&ssh),
            ssh_add,
            agent_running,
            agent_keys,
            git: versao_git(&git),
            git_user,
            // `gh` é opcional: o app funciona sem ele, mas poder estar
            // autenticado por dois caminhos é informação que a tela mostra.
            gh: versao_gh(&gh),
            gh_account: conta_gh(&sessao),
            sem_resposta,
            editors: detectar_editores(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn respondeu(stdout: &str, stderr: &str, code: Option<i32>) -> Sonda {
        Sonda::Respondeu {
            stdout: stdout.into(),
            stderr: stderr.into(),
            code,
        }
    }

    fn pasta_de_teste(nome: &str) -> PathBuf {
        let raiz = std::env::temp_dir().join(format!("sonda-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        std::fs::create_dir_all(&raiz).unwrap();
        raiz
    }

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
        assert_eq!(
            parse_gh_account("Please run `gh auth login` to get started.\n"),
            None
        );
        // só o host, sem afirmar login, não vira conta
        assert_eq!(
            parse_gh_account("github.com\n  - Active account: true\n"),
            None
        );
    }

    #[test]
    fn sessao_inativa_nao_conta_como_liberacao() {
        let saida = "github.com\n  ✓ Logged in to github.com account Velho (keyring)\n  - Active account: false\n";
        assert_eq!(parse_gh_account(saida), None);
    }

    #[test]
    fn contagem_de_chaves_do_agente() {
        let duas =
            "/c/Users/dou/.ssh/id_ed25519 (ecdsa-sha2-nistp256)\n/home/u/.ssh/id_rsa (RSA)\n";
        assert_eq!(count_agent_keys(duas), 2);
        assert_eq!(count_agent_keys("The agent has no identities.\n"), 0);
        assert_eq!(count_agent_keys(""), 0);
    }

    #[test]
    fn agente_parado_e_reconhecido_pelo_texto_das_duas_builds() {
        // respostas reais capturadas nesta máquina (código 1 ou 2 conforme a build)
        assert!(sem_agente(
            "could not open a connection to your authentication agent."
        ));
        assert!(sem_agente(
            "error connecting to agent: no such file or directory"
        ));
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

    #[test]
    fn sonda_real_de_quem_responde_e_de_quem_nao_existe() {
        let r = sonda("git", &["--version"]);
        let Sonda::Respondeu { stdout, code, .. } = &r else {
            panic!("o git desta máquina deveria responder: {r:?}");
        };
        assert_eq!(*code, Some(0));
        assert!(
            stdout.starts_with("git version"),
            "saída inesperada: {stdout}"
        );
        // nome que não existe no PATH é dito ausente, e não sem resposta
        assert_eq!(
            sonda("devops-desktop-sem-essa-ferramenta", &["--version"]),
            Sonda::Ausente
        );
    }

    #[test]
    fn sonda_real_devolve_sem_resposta_quando_o_programa_nao_termina() {
        // O caso da máquina onde a tela ficou girando: um programa que continua
        // vivo muito além do prazo da sondagem.
        let (programa, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "40", "127.0.0.1"])
        } else {
            ("sleep", &["40"])
        };
        let inicio = std::time::Instant::now();
        assert_eq!(sonda(programa, args), Sonda::SemResposta);
        assert!(
            inicio.elapsed() < Duration::from_secs(20),
            "a sonda esperou o programa inteiro terminar: {:?}",
            inicio.elapsed()
        );
    }

    #[test]
    fn cada_programa_sem_resposta_aparece_uma_sozinho() {
        let mudo = Sonda::Ausente;
        let travado = Sonda::SemResposta;
        let probes = [
            ("git", &travado),
            ("git", &travado),
            ("git", &mudo),
            ("gh", &travado),
            ("ssh", &mudo),
        ];
        assert_eq!(
            sem_resposta_de(&probes),
            vec!["git".to_string(), "gh".to_string()]
        );
        // nada travado e a lista fica vazia: a tela não inventa problema
        assert!(sem_resposta_de(&[("git", &mudo), ("ssh", &mudo)]).is_empty());
    }

    #[test]
    fn sonda_sem_resposta_nao_vira_versao_nem_conta() {
        for muda in [Sonda::Ausente, Sonda::SemResposta] {
            assert_eq!(versao_git(&muda), None);
            assert_eq!(versao_gh(&muda), None);
            assert_eq!(versao_ssh(&muda), None);
            assert_eq!(conta_gh(&muda), None);
            assert_eq!(configurado(&muda), "");
            assert_eq!(estado_agente(&muda), (None, None, None));
        }
    }

    #[test]
    fn versoes_sao_lidas_de_onde_cada_programa_escreve() {
        assert_eq!(
            versao_ssh(&respondeu("", "OpenSSH_10.2p1, LibreSSL 3.9.2\n", Some(0))).as_deref(),
            Some("OpenSSH_10.2p1")
        );
        assert_eq!(
            versao_ssh(&respondeu("OpenSSH_9.6p1\n", "", Some(0))).as_deref(),
            Some("OpenSSH_9.6p1")
        );
        assert_eq!(
            versao_git(&respondeu("git version 2.53.0.windows.2\n", "", Some(0))).as_deref(),
            Some("2.53.0.windows.2")
        );
        assert_eq!(
            versao_gh(&respondeu("gh version 2.40.1 (2024-01-01)\n", "", Some(0))).as_deref(),
            Some("2.40.1 (2024-01-01)")
        );
        // código de saída estranho não apaga a versão impressa: o programa está
        // aqui, e dizer o contrário mandaria o usuário reinstalar o git
        assert_eq!(
            versao_git(&respondeu("git version 9.9.9\n", "", None)).as_deref(),
            Some("9.9.9")
        );
    }

    #[test]
    fn configuracao_do_git_so_vale_com_codigo_zero() {
        assert_eq!(
            configurado(&respondeu("  DouglasGDev \n", "", Some(0))),
            "DouglasGDev"
        );
        // chave inexistente: o git sai com 1 e não há identidade a reportar
        assert_eq!(configurado(&respondeu("", "", Some(1))), "");
    }

    #[test]
    fn agente_respondendo_devolve_as_chaves_carregadas() {
        let (presente, rodando, chaves) = estado_agente(&respondeu(
            "256 SHA256:abc douglas@notebook (ed25519)\n",
            "",
            Some(0),
        ));
        assert_eq!(presente.as_deref(), Some("disponível"));
        assert_eq!(rodando, Some(true));
        assert_eq!(chaves, Some(1));

        let (presente, rodando, chaves) = estado_agente(&respondeu(
            "",
            "Could not open a connection to your authentication agent.\n",
            Some(2),
        ));
        assert_eq!(presente.as_deref(), Some("disponível"));
        assert_eq!(rodando, Some(false));
        assert_eq!(chaves, Some(0));
    }

    #[test]
    fn sessao_do_gh_exige_codigo_zero_e_linha_de_login() {
        assert_eq!(
            conta_gh(&respondeu(
                "",
                "  ✓ Logged in to github.com account dou (keyring)\n",
                Some(0)
            ))
            .as_deref(),
            Some("dou")
        );
        // sem código zero a leitura não afirma nada
        assert_eq!(
            conta_gh(&respondeu(
                "Logged in to github.com account dou\n",
                "",
                Some(1)
            )),
            None
        );
    }

    #[test]
    fn bases_usam_so_variaveis_com_valor() {
        let ambiente = |chave: &str| match chave {
            "LOCALAPPDATA" => Some("C:\\Users\\dou\\AppData\\Local".to_string()),
            "ProgramFiles" => Some("   ".to_string()),
            _ => None,
        };
        assert_eq!(
            bases_de_instalacao(&ambiente),
            vec![PathBuf::from("C:\\Users\\dou\\AppData\\Local")]
        );
    }

    #[test]
    fn no_path_encontra_o_executavel_e_ignora_o_resto() {
        let raiz = pasta_de_teste("path");
        let exe = raiz.join("Code.exe");
        std::fs::write(&exe, "").unwrap();

        let separador = if cfg!(windows) { ";" } else { ":" };
        let variavel = format!("{}{}C:\\nao-existe", raiz.display(), separador);
        assert_eq!(
            no_path(&variavel, &["Cursor.exe", "Code.exe"]).as_deref(),
            Some(exe.display().to_string().as_str())
        );
        // pasta vazia e nome que não está nela não produzem caminho
        assert_eq!(
            no_path(&format!("{separador}{separador}"), &["Code.exe"]),
            None
        );
        assert_eq!(no_path(&raiz.display().to_string(), &["cursor"]), None);
        // o shim de shell sem extensão que o instalador põe no PATH não é
        // executável no Windows, então não pode virar escolha na tela
        #[cfg(windows)]
        {
            std::fs::write(raiz.join("code"), "#!/bin/sh\n").unwrap();
            assert_eq!(
                no_path(&raiz.display().to_string(), &["code", "code.cmd"]),
                None
            );
        }
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn localizar_acha_o_editor_do_path_com_id_e_nome() {
        let raiz = pasta_de_teste("localizar-path");
        let exe = raiz.join("Code.exe");
        std::fs::write(&exe, "").unwrap();

        let achados = localizar(&|chave| match chave {
            "PATH" => Some(raiz.display().to_string()),
            _ => None,
        });
        assert_eq!(achados.len(), 1, "esperava só o VS Code: {achados:?}");
        assert_eq!(achados[0].id, "vscode");
        assert_eq!(achados[0].nome, "Visual Studio Code");
        assert_eq!(achados[0].caminho, exe.display().to_string());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn localizar_acha_a_instalacao_na_pasta_base() {
        let base = pasta_de_teste("localizar-base");
        let pasta = base.join("Programs").join("Microsoft VS Code");
        std::fs::create_dir_all(&pasta).unwrap();
        let exe = pasta.join("Code.exe");
        std::fs::write(&exe, "").unwrap();

        let achados = localizar(&|chave| match chave {
            "LOCALAPPDATA" => Some(base.display().to_string()),
            _ => None,
        });
        assert!(
            achados
                .iter()
                .any(|e| e.id == "vscode" && Path::new(&e.caminho).is_file()),
            "a instalação na base não foi encontrada: {achados:?}"
        );
        // nada que não existe no disco entra na lista
        assert!(achados.iter().all(|e| Path::new(&e.caminho).is_file()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sem_ambiente_nenhum_editor_e_oferecido() {
        let achados = localizar(&|chave| {
            if chave == "PATH" {
                Some(String::new())
            } else {
                None
            }
        });
        // os caminhos absolutos do candidato só entram se existirem de verdade
        assert!(achados.iter().all(|e| Path::new(&e.caminho).is_file()));
    }
}
