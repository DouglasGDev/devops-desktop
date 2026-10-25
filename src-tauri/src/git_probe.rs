use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

/// Prazo de toda sondagem de pasta: o `stat` do caminho e cada `git` de leitura
/// que ela faz. `metadata()` espera o disco **dentro do kernel**, e um volume
/// externo adormecido — ou uma letra que deixou de existir com o cabo fora —
/// simplesmente não responde. Sem prazo, a tela gira para sempre; aqui a espera
/// é abandonada e o motivo aparece. A thread pode ficar presa no disco, o
/// aplicativo não.
pub(crate) const PRAZO_SONDA: Duration = Duration::from_secs(20);

/// Detecta se a pasta contém um repositório Git e coleta branch/remote com o
/// Git nativo (argumentos separados, sem shell). Somente leitura.
pub struct GitProbe {
    pub is_git_repo: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
}

/// `Err` é a pasta que não respondeu a tempo. Não confundir com
/// `is_git_repo: false`, que é o git tendo respondido "isto não é repositório":
/// um disco lento jamais viraria "pasta sem Git" na tela.
pub fn probe(path: &Path) -> Result<GitProbe, String> {
    let dentro = crate::git::sonde(path, &["rev-parse", "--is-inside-work-tree"], PRAZO_SONDA)?;
    if dentro.as_deref() != Some("true") {
        return Ok(GitProbe {
            is_git_repo: false,
            branch: None,
            remote: None,
        });
    }
    Ok(GitProbe {
        is_git_repo: true,
        branch: crate::git::sonde(path, &["rev-parse", "--abbrev-ref", "HEAD"], PRAZO_SONDA)?,
        remote: crate::git::sonde(path, &["remote", "get-url", "origin"], PRAZO_SONDA)?,
    })
}

/// Roda a sondagem com prazo, devolvendo o motivo quando ela não volta.
pub(crate) fn com_prazo<T>(
    alvo: &str,
    prazo: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(prazo) {
        Ok(resultado) => Ok(resultado),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "a pasta {alvo} não respondeu em {} — o disco pode estar adormecido, desconectado ou \
             com a letra trocada",
            crate::sshexec::texto_duracao(prazo)
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!(
            "a sondagem da pasta {alvo} foi interrompida antes de responder"
        )),
    }
}

pub fn canonical_dir(raw: &str) -> Result<PathBuf, String> {
    let path = crate::ssh_config::expand_tilde(raw);
    let alvo = path.display().to_string();
    com_prazo(&alvo, PRAZO_SONDA, move || {
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("a pasta {} não pôde ser acessada: {e}", path.display()))?;
        if !meta.is_dir() {
            return Err(format!("o caminho {} não é uma pasta", path.display()));
        }
        // `absolute` é léxica: resolve `.`/`..` e o prefixo verbatim sem perguntar
        // nada ao volume — que é justamente a chamada que trava em disco externo.
        let absoluto = std::path::absolute(&path)
            .map_err(|e| format!("falha ao normalizar {}: {e}", path.display()))?;
        Ok(simple(&absoluto))
    })
    .and_then(|resultado| resultado)
}

/// O que a leitura de um caminho devolve: o canônico, a resposta de que a pasta
/// não está lá, ou a ausência de resposta. As três são coisas diferentes na tela: a
/// segunda é o pedido recusado ("a pasta X não existe"), a terceira é o motivo da
/// espera abandonada — e jamais pode virar "não existe".
#[derive(Debug)]
pub enum Canon {
    Resolveu(PathBuf),
    Ausente(String),
    SemResposta(String),
}

/// `canonicalize()` resolve o caminho abrindo a pasta, e essa abertura espera pelo
/// disco dentro do kernel: num volume adormecido a espera não termina e a tela gira
/// para sempre. Aqui ela tem prazo — e o prazo é parâmetro porque é a única forma de
/// medir que a espera foi abandonada sem ter um disco morto em mãos. O que volta em
/// `Resolveu` já está depois do prefixo verbatim, para que a régua da contenção e o
/// caminho da tela sejam o mesmo.
pub fn canon(caminho: &Path, prazo: Duration) -> Canon {
    let bruto = caminho.to_path_buf();
    let alvo = bruto.display().to_string();
    match com_prazo(&alvo, prazo, move || bruto.canonicalize()) {
        Ok(Ok(resolvido)) => Canon::Resolveu(simple(&resolvido)),
        Ok(Err(e)) => Canon::Ausente(e.to_string()),
        Err(motivo) => Canon::SemResposta(motivo),
    }
}

/// O caminho canônico **até onde o disco consegue responder**. Uma pasta que ainda
/// não existe — o destino da primeira publicação — não tem o que abrir, então o que
/// é resolvido é o ancestral mais fundo que está lá, e o resto do caminho vem como
/// foi escrito. Sem isso o lado ausente fica na forma digitada enquanto o outro veio
/// do disco, e uma comparação entre eles não significa nada: é assim que a régua que
/// impede o destino de ficar dentro do projeto simplesmente não dispara.
pub fn canon_ou_ancestral(caminho: &Path, prazo: Duration) -> Canon {
    enum Leitura {
        Resolveu(PathBuf),
        Ausente(String),
    }
    let bruto = caminho.to_path_buf();
    let alvo = bruto.display().to_string();
    let resposta = com_prazo(&alvo, prazo, move || -> Leitura {
        let absoluto = std::path::absolute(&bruto)
            .map(|a| simple(&a))
            .unwrap_or(bruto);
        match absoluto.canonicalize() {
            Ok(resolvido) => Leitura::Resolveu(simple(&resolvido)),
            Err(e) => {
                let mut resto: Vec<std::ffi::OsString> = Vec::new();
                let mut atual = absoluto;
                loop {
                    let Some(pai) = atual.parent() else {
                        return Leitura::Ausente(e.to_string());
                    };
                    match atual.file_name() {
                        Some(nome) => resto.push(nome.to_os_string()),
                        None => return Leitura::Ausente(e.to_string()),
                    }
                    atual = pai.to_path_buf();
                    if let Ok(c) = atual.canonicalize() {
                        let mut resolvido = simple(&c);
                        for parte in resto.iter().rev() {
                            resolvido.push(parte);
                        }
                        return Leitura::Resolveu(resolvido);
                    }
                }
            }
        }
    });
    match resposta {
        Ok(Leitura::Resolveu(resolvido)) => Canon::Resolveu(resolvido),
        Ok(Leitura::Ausente(motivo)) => Canon::Ausente(motivo),
        Err(motivo) => Canon::SemResposta(motivo),
    }
}

/// No Windows, `canonicalize()` devolve caminhos verbatim (`\\?\C:\...` e
/// `\\?\UNC\servidor\share`). Eles vão para o banco e para a tela, e nenhum
/// outro programa os reconhece, então o prefixo é desfeito aqui.
pub fn simple(path: &Path) -> PathBuf {
    let texto = path.to_string_lossy();
    if let Some(resto) = texto.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{resto}"));
    }
    if let Some(resto) = texto.strip_prefix(r"\\?\") {
        return PathBuf::from(resto);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desaz_o_caminho_verbatim_do_windows() {
        assert_eq!(
            simple(Path::new(r"\\?\C:\repos\app")),
            PathBuf::from(r"C:\repos\app")
        );
        assert_eq!(
            simple(Path::new(r"\\?\UNC\servidor\share\app")),
            PathBuf::from(r"\\servidor\share\app")
        );
        assert_eq!(
            simple(Path::new("/home/deploy/app")),
            PathBuf::from("/home/deploy/app")
        );
    }

    #[test]
    fn uma_sondagem_lenta_e_abandonada_com_o_motivo() {
        // O que a sondagem faz quando o disco não responde: a espera é deixada
        // para trás com a pasta e a duração no texto, e o valor lento nunca é
        // devolvido como se fosse um veredito.
        let alvo = Path::new(r"C:\somewhere\projeto");
        let texto = alvo.display().to_string();
        let rapido = com_prazo(&texto, Duration::from_secs(5), || 7usize).unwrap();
        assert_eq!(rapido, 7);

        let antes = std::time::Instant::now();
        let lento = com_prazo(&texto, Duration::from_millis(50), || {
            thread::sleep(Duration::from_millis(400));
            "resposta que ninguém esperou"
        });
        let decorrido = antes.elapsed();
        let erro = lento.expect_err("50 ms não é tempo de o disco acordar");
        assert!(erro.contains("não respondeu em 50 ms"), "{erro}");
        assert!(erro.contains("projeto"), "{erro}");
        assert!(
            decorrido < Duration::from_millis(250),
            "abandonou tarde: {decorrido:?}"
        );
    }

    #[test]
    fn canon_separa_nao_existe_de_nao_respondeu() {
        let pasta = std::env::temp_dir().join(format!(
            "devops-canon-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&pasta).expect("criar a pasta temporária");

        // Pasta que está lá: o caminho canônico responde, e vem na forma que
        // outro programa abre.
        match canon(&pasta, PRAZO_SONDA) {
            Canon::Resolveu(c) => {
                assert_eq!(c, pasta, "o canônico é a própria pasta");
                assert!(!c.to_string_lossy().starts_with(r"\\?\"), "{c:?}");
            }
            Canon::Ausente(e) => panic!("a pasta existe, e não respondeu ausência: {e}"),
            Canon::SemResposta(e) => panic!("a pasta existe, e não ficou sem resposta: {e}"),
        }

        // Pasta que não está lá: o sistema respondeu, e o que veio é a frase dele.
        let fora = pasta.join("filho-inexistente");
        match canon(&fora, PRAZO_SONDA) {
            Canon::Ausente(e) => {
                assert!(!e.contains("não respondeu"), "ausência virou silêncio: {e}");
            }
            Canon::Resolveu(c) => panic!("não existe e mesmo assim resolveu: {c:?}"),
            Canon::SemResposta(e) => panic!("não existe é resposta, não silêncio: {e}"),
        }

        // Prazo menor que qualquer resposta de disco possível: a espera é
        // abandonada com o motivo, nomeando a pasta — e é essa a frase que a tela
        // lê, nunca a de "não existe".
        let antes = std::time::Instant::now();
        match canon(&pasta, Duration::ZERO) {
            Canon::SemResposta(e) => {
                assert!(e.contains("não respondeu em 0 ms"), "{e}");
                assert!(e.contains("devops-canon"), "{e}");
            }
            Canon::Resolveu(c) => panic!("0 ms não é resposta, é prazo: {c:?}"),
            Canon::Ausente(e) => panic!("prazo vencido não é ausência: {e}"),
        }
        assert!(
            antes.elapsed() < Duration::from_millis(250),
            "abandonou tarde"
        );
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn a_regua_do_destino_resolve_o_lado_ausente_pelo_ancestral() {
        let pasta = std::env::temp_dir().join(format!(
            "devops-ancestral-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&pasta).expect("criar a pasta temporária");

        // O destino da primeira publicação: ninguém criou a pasta, e nem a mãe
        // dela. O que a régua devolve é o ancestral resolvido mais o resto, na
        // mesma forma do outro lado — senão a comparação vira sempre "fora".
        let futuro = pasta.join("publicado").join("v1");
        match canon_ou_ancestral(&futuro, PRAZO_SONDA) {
            Canon::Resolveu(c) => {
                assert_eq!(c, futuro, "o resto do caminho vem como foi escrito");
            }
            Canon::Ausente(e) => panic!("a mãe existe, e não respondeu ausência: {e}"),
            Canon::SemResposta(e) => panic!("prazo de 20 s não venceu: {e}"),
        }

        // A pasta que está lá responde igual ao `canon`.
        match canon_ou_ancestral(&pasta, PRAZO_SONDA) {
            Canon::Resolveu(c) => assert_eq!(c, pasta),
            other => panic!("a pasta existe: {other:?}"),
        }

        // E o prazo continua mandando: a espera é abandonada com o motivo, que é
        // a frase do volume adormecido, não a de "não existe".
        match canon_ou_ancestral(&pasta, Duration::ZERO) {
            Canon::SemResposta(e) => {
                assert!(e.contains("não respondeu em 0 ms"), "{e}");
                assert!(e.contains("devops-ancestral"), "{e}");
            }
            Canon::Resolveu(c) => panic!("0 ms não é resposta, é prazo: {c:?}"),
            Canon::Ausente(e) => panic!("prazo vencido não é ausência: {e}"),
        }
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn sonda_distinge_pasta_sem_git_de_pasta_que_nao_respondeu() {
        let raiz = std::env::temp_dir().join(format!(
            "devops-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&raiz).expect("criar a pasta temporária");

        // Fora de qualquer repositório, o git responde "não é repositório".
        let pasta_comum = probe(&raiz).expect("o git respondeu sobre a pasta comum");
        assert!(!pasta_comum.is_git_repo);
        assert_eq!(pasta_comum.branch, None);

        // Dentro de um repositório ainda sem commit: é repositório, e a branch é
        // ausência dita, não silêncio.
        crate::git::run(&raiz, &["-c", "init.defaultBranch=main", "init", "-q", "."])
            .expect("git init");
        let repo = probe(&raiz).expect("o git respondeu sobre o repositório");
        assert!(repo.is_git_repo);
        assert_eq!(repo.branch, None, "sem commit não há branch");
        assert_eq!(repo.remote, None, "nenhum remoto foi configurado");

        let _ = std::fs::remove_dir_all(&raiz);
    }
}
