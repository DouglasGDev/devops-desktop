//! Análise de código-fonte (plano CI/CD, etapa 5).
//!
//! Duas portas, uma mecânica: a tela de Análise e um nó `analise` do grafo
//! chamam a mesma [`executar`], que abre a corrida no banco, roda os instrumentos
//! pedidos, guarda os achados e fecha a corrida com a contagem real. Nada aqui
//! decide sozinho se a publicação segue — o que existe é um veredito lido da
//! contagem contra o corte declarado, e o motor barra quando esse veredito diz
//! que barra.
//!
//! Dois instrumentos:
//! * `embutido` — a tabela de regras deste arquivo comparando linhas. Não precisa
//!   de nada instalado e não abre processo filho nenhum.
//! * `semgrep` — a ferramenta de análise de código aberta, chamada como ela é:
//!   processo filho com prazo, JSON lido de um arquivo, métricas desligadas.
//!
//! Os identificadores de regra são os mesmos nos dois (`dd-1NN-…`), então o mesmo
//! achado tem o mesmo nome venha de quem vier, e `ambos` não produz duas listas
//! que ninguém consegue somar.
//!
//! Três mecânicas são herdadas de quem já as dona, e não repetidas aqui: a
//! disciplina de varredura de árvore e a mascaráção de segredo são de
//! [`crate::security`], e o processo filho com prazo é de [`crate::sshexec`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::models::{
    AnalysisCounts, AnalysisFinding, AnalysisInput, AnalysisRun, Project, SEVERIDADES,
};
use crate::{db, git_probe, proc, security, sshexec};

/// As regras embarcadas no aplicativo. Elas são copiadas para os dados do
/// aplicativo na primeira varredura porque o semgrep lê regras do disco: não há
/// como entregar-lhe um texto pela linha de comando.
pub const REGRAS_YAML: &str = include_str!("../regras/dd-seguranca.yaml");
pub const NOME_DAS_REGRAS: &str = "dd-seguranca.yaml";

/// Onde a tela grava o caminho de um semgrep fora do PATH — o caso real desta
/// máquina, com a ferramenta dentro de um ambiente virtual de teste.
pub const CHAVE_CONFIG: &str = "analise.semgrep.caminho";

/// Prazo de uma varredura quando o pedido não diz nada. Com p/default, a varredura
/// cobre múltiplas linguagens e pode demorar, especialmente na primeira execução
/// (baixa as regras do registry). Medido: 1181 arquivos com as regras embarcadas
/// levam ~7 s; com p/default em projetos poliglotas pode chegar a 2-5 minutos.
/// 600 s (10 minutos) é margem para um projeto grande sem deixar a janela presa.
pub const PRAZO_PADRAO: Duration = Duration::from_secs(600);

/// Prazo da sondagem de versão. `semgrep --version` quente levou 3,0 s medido; a
/// primeira execução depois da instalação levou 19,5 s nesta máquina, com o
/// sistema ainda lendo os arquivos novos. Os 20 s de antes transformavam essa
/// primeira leitura em "sem-resposta", então o prazo tem folga para a fria.
const PRAZO_SONDA: Duration = Duration::from_secs(30);

/// Arquivos abertos pelo analisador embutido antes de a varredura se declarar
/// cortada. Passou disso a nota diz que a cobertura é parcial: varrer sem limite
/// é como travar a janela.
const LIMITE_ARQUIVOS: usize = 20_000;

/// Achados guardados por varredura. Uma regra ruim sobre um monorepo gera dezenas
/// de milhares; o banco guarda os primeiros e a nota diz quantos ficaram de fora,
/// para a contagem nunca ser lida como se fosse o total.
const LIMITE_ACHADOS: usize = 5_000;

/// Teto de conteúdo em memória para ir buscar a linha que o semgrep apontou.
/// Depois disto o trecho some; o achado continua lá.
const LIMITE_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Quantas linhas o trecho de um achado mostra: contexto, não o arquivo inteiro.
const LINHAS_DE_TRECHO: usize = 3;

/// Quantos achados cabem no pacote que uma IA lê. Um modelo que recebe dez mil
/// linhas não decide melhor que um que lê as trezentas piores.
const LIMITE_DO_PACOTE: usize = 300;

// ---------- o que a máquina tem ----------

/// Estado da ferramenta externa, medido nesta máquina. Os três estados são ditos
/// separadamente: `ausente` é "não achei o programa", `sem-resposta` é "achei e
/// ele não respondeu dentro do prazo" — confundir os dois faria uma instalação
/// lenta parecer uma ausência.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ferramenta {
    /// `encontrado`, `ausente` ou `sem-resposta`.
    pub estado: String,
    pub caminho: Option<String>,
    pub versao: Option<String>,
    /// O que a sondagem levou, em ms — para a tela dizer "demorou" em vez de
    /// esconder um prazo estourado.
    pub duracao_ms: i64,
    /// Quantas regras o conjunto embarcado declara, lido do próprio texto.
    pub regras: usize,
    /// De onde o caminho veio: `configuração` ou `path`.
    pub origem: String,
    /// O valor que alguém **apontou** em Configurações, antes de qualquer
    /// resolução — é o que a linha de auditoria precisa dizer quando o apontado não
    /// existe, porque aí `caminho` é nulo e `origem` fica vazio.
    pub apontado: String,
    /// O que a ferramenta respondeu na sondagem de versão, em uma linha.
    pub resposta: String,
}

impl Ferramenta {
    pub fn encontrada(&self) -> bool {
        self.estado == "encontrado"
    }

    /// O valor apontado em Configurações não resolveu e o binário veio da busca
    /// automática. É o que a tela precisa dizer para ninguém achar que aquele
    /// caminho ainda manda na varredura.
    pub fn apontado_ignorado(&self) -> bool {
        !self.apontado.trim().is_empty() && !self.origem.is_empty() && self.origem != "configuração"
    }
}

/// Os nomes pelos quais o pacote se instala. No Windows o `pip` põe um `.exe` no
/// PATH; em POSIX o launcher é um script sem extensão.
fn nomes_da_ferramenta() -> &'static [&'static str] {
    if cfg!(windows) {
        // pysemgrep.exe é o executável real; semgrep.exe é um wrapper que precisa
        // de pysemgrep no PATH. Procuramos pysemgrep primeiro para evitar o wrapper.
        &["pysemgrep.exe", "semgrep.exe"]
    } else {
        &["semgrep"]
    }
}

/// Primeiro candidato dentro das pastas do PATH. A mesma regra de `tools.rs`: um
/// script de shell sem extensão no PATH não é um executável que a aplicação
/// consiga abrir, então no Windows só se procura o nome com `.exe`.
fn no_path(nome: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    for pasta in path.split(if cfg!(windows) { ';' } else { ':' }) {
        let pasta = pasta.trim();
        if pasta.is_empty() {
            continue;
        }
        let candidato = Path::new(pasta).join(nome);
        if candidato.is_file() {
            return Some(candidato.display().to_string());
        }
    }
    None
}

/// Procura o executável em diretórios comuns de instalação do Python no Windows
/// (user scripts e local programs). Retorna o primeiro caminho encontrado.
fn no_python_scripts(nome: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let home = dirs::home_dir()?;
    let appdata = home.join("AppData");
    // Tenta Roaming/Python/Python3X/Scripts (pip install --user)
    let roaming = appdata.join("Roaming").join("Python");
    if roaming.exists() {
        if let Ok(entries) = std::fs::read_dir(&roaming) {
            for entry in entries.flatten() {
                let scripts = entry.path().join("Scripts").join(nome);
                if scripts.is_file() {
                    return Some(scripts.display().to_string());
                }
            }
        }
    }
    // Tenta Local/Programs/Python/Python3X/Scripts (instalação padrão)
    let local = appdata.join("Local").join("Programs").join("Python");
    if local.exists() {
        if let Ok(entries) = std::fs::read_dir(&local) {
            for entry in entries.flatten() {
                let scripts = entry.path().join("Scripts").join(nome);
                if scripts.is_file() {
                    return Some(scripts.display().to_string());
                }
            }
        }
    }
    None
}

/// Resolve o caminho que alguém apontou na configuração. Um nome com barra é um
/// caminho e tem de existir como arquivo; um nome solto é um programa e só o
/// caminho de busca responde por ele.
fn resolver_explicito_com(
    bruto: &str,
    achar_no_path: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let nome = bruto.trim().trim_matches('"');
    if nome.is_empty() {
        return None;
    }
    if nome.contains('/') || nome.contains('\\') {
        let caminho = crate::ssh_config::expand_tilde(nome);
        return caminho.is_file().then(|| caminho.display().to_string());
    }
    achar_no_path(nome)
}

/// O binário que a varredura vai chamar e de onde ele veio. O caminho apontado
/// tenta primeiro; se ele não resolve — uma pasta, um caminho antigo —, a busca
/// automática continua em vez de declarar a ferramenta ausente. O valor apontado
/// fica com quem chamou para a tela poder dizer que ele não serviu.
fn procurar_binario(
    caminho_explicito: Option<&str>,
    achar_no_path: impl Fn(&str) -> Option<String>,
    achar_nos_scripts: impl Fn(&str) -> Option<String>,
) -> (Option<String>, String) {
    let mut origem = String::new();
    let mut binario = caminho_explicito
        .and_then(|bruto| resolver_explicito_com(bruto, &achar_no_path))
        .inspect(|_| origem = "configuração".to_string());
    if binario.is_none() {
        for nome in nomes_da_ferramenta() {
            if let Some(achado) = achar_no_path(nome) {
                binario = Some(achado);
                origem = "path".to_string();
                break;
            }
            if let Some(achado) = achar_nos_scripts(nome) {
                binario = Some(achado);
                origem = "python-scripts".to_string();
                break;
            }
        }
    }
    (binario, origem)
}

/// Localiza o semgrep e pergunta a versão dele. Nada aqui é presunção: o programa
/// é executado com prazo, e quem não responde é dito sem resposta.
pub fn localizar(caminho_explicito: Option<&str>) -> Ferramenta {
    localizar_com(caminho_explicito, no_path, no_python_scripts)
}

fn localizar_com(
    caminho_explicito: Option<&str>,
    achar_no_path: impl Fn(&str) -> Option<String>,
    achar_nos_scripts: impl Fn(&str) -> Option<String>,
) -> Ferramenta {
    let inicio = Instant::now();
    let regras = contar_regras(REGRAS_YAML);
    let apontado = caminho_explicito.unwrap_or("").trim().to_string();
    let (binario, origem) = procurar_binario(caminho_explicito, achar_no_path, achar_nos_scripts);
    let Some(binario) = binario else {
        return Ferramenta {
            estado: "ausente".to_string(),
            caminho: None,
            versao: None,
            duracao_ms: duracao_ms(inicio),
            regras,
            origem,
            apontado,
            resposta: String::new(),
        };
    };
    let chamada = rodar(
        &binario,
        &["--version"],
        None,
        PRAZO_SONDA,
        &AtomicBool::new(false),
    );
    if chamada.sem_resposta || chamada.code.is_none() && chamada.stderr.is_empty() {
        return Ferramenta {
            estado: "sem-resposta".to_string(),
            caminho: Some(binario),
            versao: None,
            duracao_ms: duracao_ms(inicio),
            regras,
            origem,
            apontado,
            resposta: cauda(&chamada.stderr),
        };
    }
    Ferramenta {
        estado: "encontrado".to_string(),
        versao: versao_da_resposta(&chamada.stdout, &chamada.stderr),
        caminho: Some(binario),
        duracao_ms: duracao_ms(inicio),
        regras,
        origem,
        apontado,
        resposta: cauda(&chamada.stdout),
    }
}

/// Tenta instalar o Semgrep automaticamente quando ele não está disponível.
/// Primeiro tenta `pip install --user semgrep`, depois `pipx install semgrep`.
/// Retorna o caminho do binário instalado ou erro.
pub fn instalar_semgrep(cancel: &AtomicBool) -> Result<String, String> {
    // Tenta pip primeiro (mais comum)
    let pip_cmd = if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    };
    let pip_args = ["-m", "pip", "install", "--user", "semgrep"];
    let pip_result = rodar(pip_cmd, &pip_args, None, Duration::from_secs(300), cancel);
    if pip_result.cancelado {
        return Err("a instalação do semgrep foi interrompida".to_string());
    }
    if pip_result.code == Some(0) {
        // Instalação via pip bem-sucedida; tenta localizar de novo
        let ferramenta = localizar(None);
        if let Some(caminho) = ferramenta.caminho {
            return Ok(caminho);
        }
    }
    // Tenta pipx como fallback
    let pipx_args = ["install", "semgrep"];
    let pipx_result = rodar("pipx", &pipx_args, None, Duration::from_secs(300), cancel);
    if pipx_result.cancelado {
        return Err("a instalação do semgrep foi interrompida".to_string());
    }
    if pipx_result.code == Some(0) {
        let ferramenta = localizar(None);
        if let Some(caminho) = ferramenta.caminho {
            return Ok(caminho);
        }
    }
    Err(
        "não foi possível instalar o semgrep automaticamente. Instale manualmente com `pip install semgrep` ou `pipx install semgrep`"
            .to_string(),
    )
}

fn duracao_ms(inicio: Instant) -> i64 {
    inicio.elapsed().as_millis() as i64
}

/// A primeira sequência de dígitos com ponto que a ferramenta escreveu. Formatos
/// reais medidos: `1.181.0` sozinho e `semgrep 1.181.0`.
fn versao_da_resposta(stdout: &str, stderr: &str) -> Option<String> {
    for texto in [stdout, stderr] {
        for linha in texto.lines() {
            let solto: String = linha
                .chars()
                .map(|c| {
                    if c.is_ascii_digit() || c == '.' {
                        c
                    } else {
                        ' '
                    }
                })
                .collect();
            if let Some(pedaco) = solto
                .split(' ')
                .filter(|p| p.len() > 1)
                .find(|p| p.contains('.') && !p.trim_matches('.').is_empty())
            {
                return Some(pedaco.trim_matches('.').to_string());
            }
        }
    }
    None
}

/// Quantas regras o YAML declara. É uma contagem do texto, não um número decorado:
/// se o arquivo muda, o número dito muda com ele.
pub fn contar_regras(yaml: &str) -> usize {
    yaml.lines()
        .filter(|l| l.trim_start().starts_with("- id:"))
        .count()
}

fn ids_do_yaml(texto: &str) -> Vec<String> {
    texto
        .lines()
        .filter_map(|l| {
            l.trim_start()
                .strip_prefix("- id:")
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty())
        })
        .collect()
}

/// Copia as regras embarcadas para os dados do aplicativo quando lá não estão ou
/// quando são outras. Uma regra antiga esquecida no disco é reescrita, não
/// respeitada: a varredura nunca pode ser feita com o conjunto do aplicativo
/// anterior.
pub fn publicar_regras(dados: &Path) -> Result<PathBuf, String> {
    let pasta = dados.join("regras");
    let destino = pasta.join(NOME_DAS_REGRAS);
    if std::fs::read_to_string(&destino).ok().as_deref() != Some(REGRAS_YAML) {
        std::fs::create_dir_all(&pasta)
            .map_err(|e| format!("não foi possível criar a pasta de regras: {e}"))?;
        std::fs::write(&destino, REGRAS_YAML)
            .map_err(|e| format!("não foi possível gravar as regras embarcadas: {e}"))?;
    }
    Ok(destino)
}

// ---------- o que a tela sabe sem varrer ----------

/// Uma regra embarcada, como a tela a lista. São as mesmas linhas que o
/// analisador interno usa — não uma segunda tabela escrita à mão para agradar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegraInfo {
    pub id: String,
    pub severidade: String,
    pub mensagem: String,
    pub cwe: String,
    pub familias: Vec<String>,
}

/// As regras do analisador embutido. A tela mostra esta lista e o número de
/// regras do conjunto, e é dela que a varredura sai — uma fonte, duas portas.
pub fn regras_embarcadas() -> Vec<RegraInfo> {
    REGRAS_EMBUTIDAS
        .iter()
        .map(|r| RegraInfo {
            id: r.id.to_string(),
            severidade: r.severidade.to_string(),
            mensagem: r.mensagem.to_string(),
            cwe: r.cwe.to_string(),
            familias: r.familias.iter().map(|f| f.to_string()).collect(),
        })
        .collect()
}

/// O que a conferência das regras respondeu. Os quatro estados são ditos
/// separadamente porque "a ferramenta não está aqui" não é "as regras estão boas".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validacao {
    /// `validadas`, `inválidas`, `sem-ferramenta` ou `sem-resposta`.
    pub estado: String,
    pub regras: usize,
    pub codigo: Option<i64>,
    pub mensagem: String,
}

/// A linha que uma verificação deixa na auditoria. Fica no módulo da análise, e não
/// no comando, porque é aqui que cada estado tem significado: `ausente` é aviso,
/// `sem-resposta` é erro, e nenhum dos dois vira "está tudo certo".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinhaVerificacao {
    pub nivel: &'static str,
    pub operacao: String,
    pub detalhe: String,
}

/// A sonda do semgrep: o que foi apontado, de onde o caminho saiu e o que a
/// máquina respondeu. Quando o apontado não existe, a linha diz o valor apontado —
/// é justamente o caso em que alguém mudou Configurações e quer saber no Histórico
/// o que aquilo produziu.
pub fn linha_da_sonda(f: &Ferramenta) -> LinhaVerificacao {
    let apontado = if f.apontado.is_empty() {
        "nada apontado em Configurações".to_string()
    } else if f.apontado_ignorado() {
        format!("apontado “{}” (não serve)", f.apontado)
    } else {
        format!("apontado “{}”", f.apontado)
    };
    // `origem` só existe junto com um binário resolvido; dizer "origem lugar nenhum"
    // ao lado de um valor apontado contradiria a própria linha.
    let origem = match (&f.caminho, f.origem.is_empty()) {
        (Some(_), false) => format!(" · origem {}", f.origem),
        (Some(_), true) => String::new(),
        (None, _) => " · nenhum binário resolvido a partir dali".to_string(),
    };
    let tempo = format!("{} ms", f.duracao_ms);
    match f.estado.as_str() {
        "encontrado" => LinhaVerificacao {
            nivel: "info",
            operacao: format!("Sonda do semgrep · encontrado · {} regra(s) embarcada(s)", f.regras),
            detalhe: format!(
                "{apontado}{origem} · caminho {} · versão {} · {tempo} · resposta: {}",
                f.caminho.as_deref().unwrap_or("(sem caminho)"),
                f.versao.as_deref().unwrap_or("não lida"),
                f.resposta
            ),
        },
        "sem-resposta" => LinhaVerificacao {
            nivel: "error",
            operacao: "Sonda do semgrep · sem resposta".to_string(),
            detalhe: format!(
                "{apontado}{origem} · caminho {} · {tempo} · o prazo passou e nada voltou",
                f.caminho.as_deref().unwrap_or("(sem caminho)"),
            ),
        },
        estado => LinhaVerificacao {
            nivel: "warning",
            operacao: format!("Sonda do semgrep · {estado}"),
            detalhe: format!(
                "{apontado}{origem} · {tempo} · o instrumento externo não está nesta máquina; o embutido continua disponível",
            ),
        },
    }
}

/// A conferência do conjunto embarcado com a ferramenta externa. Sem ferramenta a
/// linha diz que **não houve** conferência externa, nunca um "ok" presumido.
pub fn linha_da_conferencia(v: &Validacao) -> LinhaVerificacao {
    let codigo = v
        .codigo
        .map(|c| format!("código {c}"))
        .unwrap_or_else(|| "sem código de saída".to_string());
    let nivel = match v.estado.as_str() {
        "validadas" => "info",
        "inválidas" | "sem-resposta" => "error",
        _ => "warning",
    };
    LinhaVerificacao {
        nivel,
        operacao: format!(
            "Conferência das regras · {} · {} regra(s)",
            v.estado, v.regras
        ),
        detalhe: format!("{codigo} · {}", v.mensagem),
    }
}

/// Confere o conjunto embarcado com a ferramenta externa — o `--validate` do
/// semgrep, que lê as regras sem varrer arquivo nenhum. Sem ferramenta a resposta
/// diz que não houve conferência externa; jamais um "ok" presumido.
pub fn conferir_regras(conn: &Connection, dados: &Path) -> Result<Validacao, String> {
    let regras = publicar_regras(dados)?;
    let contagem = contar_regras(REGRAS_YAML);
    let ferramenta = sonda_ferramenta(conn)?;
    if !ferramenta.encontrada() {
        return Ok(Validacao {
            estado: if ferramenta.estado == "sem-resposta" {
                "sem-resposta".to_string()
            } else {
                "sem-ferramenta".to_string()
            },
            regras: contagem,
            codigo: None,
            mensagem: match ferramenta.estado.as_str() {
                "sem-resposta" => "o semgrep existe nesta máquina mas não respondeu à sondagem de versão".to_string(),
                _ => format!(
                    "não há semgrep nesta máquina — as {contagem} regras embarcadas não foram conferidas por fora"
                ),
            },
        });
    }
    let caminho = regras.to_string_lossy();
    let chamada = rodar(
        ferramenta.caminho.as_deref().unwrap_or("semgrep"),
        &[
            "scan",
            "--metrics=off",
            "--validate",
            "--config",
            caminho.as_ref(),
        ],
        None,
        PRAZO_SONDA,
        &AtomicBool::new(false),
    );
    let texto = if chamada.stdout.trim().is_empty() {
        &chamada.stderr
    } else {
        &chamada.stdout
    };
    Ok(Validacao {
        estado: estado_da_conferencia(&chamada).to_string(),
        regras: contagem,
        codigo: chamada.code.map(i64::from),
        mensagem: cauda(texto),
    })
}

/// O que a resposta do `--validate` quer dizer. Uma chamada que não respondeu não
/// vira "inválida" — e muito menos "validada": é o terceiro estado, dito pelo nome.
fn estado_da_conferencia(chamada: &Chamada) -> &'static str {
    if chamada.sem_resposta || chamada.cancelado {
        "sem-resposta"
    } else if chamada.code == Some(0) {
        "validadas"
    } else {
        "inválidas"
    }
}

// ---------- processo filho ----------

/// O que uma chamada de linha de comando devolveu. `sem_resposta` e `cancelado`
/// têm nome próprio: nenhum dos dois vira "a ferramenta não achou nada".
struct Chamada {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    sem_resposta: bool,
    cancelado: bool,
}

/// Uma chamada de processo filho com prazo, sem janela de console e com o `PATH`
/// herdado. `PYTHONIOENCODING=utf-8` não é enfeite: medido no semgrep 1.181 desta
/// máquina, sem ele o filho morre com `UnicodeEncodeError: 'charmap' codec can't
/// encode` num argv que com ele roda reto. stdout e stderr ficam separados, porque
/// o JSON é lido de um arquivo e não pode depender do que a ferramenta resolveu
/// imprimir no console.
fn rodar(
    binario: &str,
    args: &[&str],
    pasta: Option<&Path>,
    prazo: Duration,
    cancel: &AtomicBool,
) -> Chamada {
    let mut cmd = proc::command(binario);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONIOENCODING", "utf-8")
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    if let Some(pasta) = pasta {
        cmd.current_dir(pasta);
    }
    // Reduz a prioridade do processo no Windows para não afetar outras operações
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x00004000;
        cmd.creation_flags(BELOW_NORMAL_PRIORITY_CLASS);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Chamada {
                stdout: String::new(),
                stderr: format!("não foi possível iniciar o programa: {e}"),
                code: None,
                sem_resposta: false,
                cancelado: false,
            }
        }
    };
    let feito = sshexec::collect(&mut child, prazo, cancel);
    Chamada {
        stdout: feito.stdout,
        stderr: feito.stderr,
        code: feito.exit_code,
        sem_resposta: feito.timed_out,
        cancelado: feito.cancelled,
    }
}

// ---------- o analisador embutido ----------

/// Uma marca de texto. Comparada contra a linha em minúsculas; com [`colado`]
/// ligado os espaços são removidos primeiro, e é assim que `verify = False` e
/// `verify=False` são a mesma coisa sem expressão regular.
#[derive(Clone, Copy)]
struct Marca {
    texto: &'static str,
    colado: bool,
}

const fn m(texto: &'static str) -> Marca {
    Marca {
        texto,
        colado: false,
    }
}

const fn mc(texto: &'static str) -> Marca {
    Marca {
        texto,
        colado: true,
    }
}

/// Regra do analisador embutido. Uma alternativa casa quando **todas** as suas
/// marcas estão na linha; a regra casa quando qualquer alternativa casa. Não há
/// expressão regular porque o projeto não tem a dependency `regex` e nada daqui
/// precisa de uma: um marcador de código ruim é uma chamada, e uma chamada cabe
/// num texto fixo.
///
/// Toda marca que começa ou termina em caractere de identificador é conferida nas
/// bordas, o que separa "a ferramenta viu" de "a ferramenta adivinhou": `md5(` não
/// acusa `sem_md5(` e `pass` não acusa `password =`.
struct RegraEmbutida {
    id: &'static str,
    severidade: &'static str,
    mensagem: &'static str,
    cwe: &'static str,
    /// As famílias de arquivo às quais a regra se aplica; vazia é qualquer arquivo
    /// de texto — é o que pega um `# nosemgrep` escrito num `.txt`.
    familias: &'static [&'static str],
    alternativas: &'static [&'static [Marca]],
    /// Regra de duas linhas: a última linha com conteúdo anterior precisa conter
    /// isto. É como `dd-110` enxerga um `except: … pass`, que não cabe numa linha.
    anterior: &'static str,
}

const REGRAS_EMBUTIDAS: &[RegraEmbutida] = &[
    // ---------- comando de sistema montado no código ----------
    RegraEmbutida {
        id: "dd-101-shell-true",
        severidade: "ERROR",
        mensagem:
            "um comando é montado e entregue a um shell; se qualquer parte vier de fora, é injeção",
        cwe: "CWE-78: OS Command Injection",
        familias: &["py"],
        alternativas: &[
            &[m("subprocess."), mc("shell=true")],
            &[m("os.system(")],
            &[m("os.popen(")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-101-shell-true-php",
        severidade: "ERROR",
        mensagem: "entrada é levada a um comando de sistema",
        cwe: "CWE-78: OS Command Injection",
        familias: &["php"],
        alternativas: &[
            &[m("system(")],
            &[m("shell_exec(")],
            &[m("passthru(")],
            &[m("exec(")],
        ],
        anterior: "",
    },
    // ---------- SQL montado no código ----------
    RegraEmbutida {
        id: "dd-102-sql-montado",
        severidade: "ERROR",
        mensagem: "a consulta SQL é montada no código em vez de usar parâmetros ligados",
        cwe: "CWE-89: SQL Injection",
        familias: &["py"],
        alternativas: &[
            &[mc("execute(f\"")],
            &[mc("execute(\""), mc("\"+")],
            &[mc("execute(\""), mc("\"%")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-102-sql-montado-js",
        severidade: "ERROR",
        mensagem: "a consulta SQL é montada por template literal em vez de parâmetros ligados",
        cwe: "CWE-89: SQL Injection",
        familias: &["js"],
        alternativas: &[
            &[m("query(`")],
            &[m("raw(`")],
            &[m("execute(`")],
            &[mc("query(\""), mc("\"+")],
            &[mc("query('"), mc("'+")],
        ],
        anterior: "",
    },
    // ---------- HTML escrito sem escapamento ----------
    RegraEmbutida {
        id: "dd-103-html-injetado",
        severidade: "ERROR",
        mensagem: "HTML é escrito numa página sem escapamento",
        cwe: "CWE-79: Cross-site Scripting",
        familias: &["js"],
        alternativas: &[
            &[mc(".innerhtml=")],
            &[mc(".outerhtml=")],
            &[mc(".insertadjacenthtml(")],
            &[m("document.write(")],
        ],
        anterior: "",
    },
    // ---------- TLS sem conferência de certificado ----------
    RegraEmbutida {
        id: "dd-104-tls-desligado",
        severidade: "ERROR",
        mensagem: "a conferência do certificado TLS está desligada aqui",
        cwe: "CWE-295: Improper Certificate Validation",
        familias: &["py"],
        alternativas: &[
            &[mc("verify=false")],
            &[m("_create_unverified_context(")],
            &[mc("ssl.cert_none")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-104-tls-desligado-js",
        severidade: "ERROR",
        mensagem: "o cliente aceita qualquer certificado — o nome dele não é conferido",
        cwe: "CWE-295: Improper Certificate Validation",
        familias: &["js"],
        alternativas: &[&[mc("rejectunauthorized:false")], &[mc("strictssl:false")]],
        anterior: "",
    },
    // ---------- texto executado como código ----------
    RegraEmbutida {
        id: "dd-105-avaliacao-de-codigo",
        severidade: "ERROR",
        mensagem: "texto é executado como código",
        cwe: "CWE-94: Code Injection",
        familias: &["py"],
        alternativas: &[&[m("eval(")], &[m("exec(")], &[m("compile(")]],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-105-avaliacao-de-codigo-js",
        severidade: "ERROR",
        mensagem: "texto é executado como código",
        cwe: "CWE-94: Code Injection",
        familias: &["js"],
        alternativas: &[&[m("eval(")], &[mc("newfunction(")]],
        anterior: "",
    },
    // ---------- desserialização do que veio de fora ----------
    RegraEmbutida {
        id: "dd-106-desserializacao",
        severidade: "WARNING",
        mensagem: "bytes de fora são desserializados; o formato permite executar código na leitura",
        cwe: "CWE-502: Deserialization of Untrusted Data",
        familias: &["py"],
        alternativas: &[
            &[m("pickle.loads(")],
            &[m("pickle.load(")],
            &[m("yaml.load(")],
            &[m("marshal.loads(")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-106-desserializacao-php",
        severidade: "WARNING",
        mensagem: "bytes de fora são desserializados; o formato permite executar código na leitura",
        cwe: "CWE-502: Deserialization of Untrusted Data",
        familias: &["php"],
        alternativas: &[&[m("unserialize(")]],
        anterior: "",
    },
    // ---------- hash criptográfico quebrado ----------
    RegraEmbutida {
        id: "dd-107-criptografia-frac",
        severidade: "WARNING",
        mensagem: "hash quebrado (MD5 ou SHA-1) usado para proteger alguma coisa",
        cwe: "CWE-327: Broken or Risky Cryptographic Algorithm",
        familias: &["py"],
        alternativas: &[
            &[m("hashlib.md5(")],
            &[m("hashlib.sha1(")],
            &[mc("hashlib.new(\"md5")],
            &[mc("hashlib.new(\"sha1")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-107-criptografia-frac-js",
        severidade: "WARNING",
        mensagem: "hash quebrado (MD5 ou SHA-1) usado para proteger alguma coisa",
        cwe: "CWE-327: Broken or Risky Cryptographic Algorithm",
        familias: &["js"],
        alternativas: &[&[mc("createhash(\"md5")], &[mc("createhash(\"sha1")]],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-107-criptografia-frac-php",
        severidade: "WARNING",
        mensagem: "hash quebrado (MD5 ou SHA-1) usado para proteger alguma coisa",
        cwe: "CWE-327: Broken or Risky Cryptographic Algorithm",
        familias: &["php"],
        alternativas: &[&[m("md5(")], &[m("sha1(")]],
        anterior: "",
    },
    // ---------- política de origem / checagem calada / falha escondida ----------
    RegraEmbutida {
        id: "dd-108-cors-aberto",
        severidade: "WARNING",
        mensagem: "qualquer origem da internet está autorizada a chamar este serviço",
        cwe: "CWE-942: Permissive Cross-domain Policy",
        familias: &[],
        alternativas: &[
            &[mc("access-control-allow-origin"), mc("*")],
            &[mc("allow_origins"), mc("*")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-109-verificacao-suprimida",
        severidade: "WARNING",
        mensagem:
            "uma checagem automática foi calada nesta linha — o motivo precisa estar escrito aqui",
        cwe: "",
        familias: &[],
        alternativas: &[
            &[mc("#nosemgrep")],
            &[mc("#nosec")],
            &[mc("//nosemgrep")],
            &[mc("eslint-disable")],
            &[mc("tslint:disable")],
            &[mc("phpcs:ignore")],
        ],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-110-falha-escondida",
        severidade: "INFO",
        mensagem: "a exceção é capturada e descartada; o que falhou não chega a lugar nenhum",
        cwe: "CWE-703: Improper Check or Handling of Exceptional Conditions",
        familias: &["py"],
        alternativas: &[&[m("pass")]],
        anterior: "except",
    },
    RegraEmbutida {
        id: "dd-111-debug-ativo",
        severidade: "INFO",
        mensagem: "um modo de depuração ficou ligado no código",
        cwe: "CWE-489: Active Debug Code",
        familias: &["py"],
        alternativas: &[&[mc(".run("), mc("debug=true")]],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-111-debug-ativo-js",
        severidade: "INFO",
        mensagem: "um ponto de parada de depuração ficou no código",
        cwe: "CWE-489: Active Debug Code",
        familias: &["js"],
        alternativas: &[&[m("debugger")]],
        anterior: "",
    },
];

/// Os identificadores das regras embarcadas, na ordem da tabela. É o que a coluna
/// `rules` da corrida mostra quando só o analisador embutido rodou.
pub fn ids_embarcados() -> Vec<&'static str> {
    REGRAS_EMBUTIDAS.iter().map(|r| r.id).collect()
}

/// A família de linguagem pelo sufixo. O dialeto `js` cobre JSX e TypeScript
/// porque é o que as regras daqui distinguem — não há regra exclusiva de
/// TypeScript neste conjunto, medido no dialeto do próprio semgrep.
fn familia(extensao: &str) -> &str {
    match extensao {
        "py" | "pyi" | "pyw" => "py",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "vue" => "js",
        "php" | "phtml" | "php5" | "php7" => "php",
        outra => outra,
    }
}

/// A linha em minúsculas sem os espaços — a outra face da mesma linha, para as
/// marcas que não ligam para formatação.
fn colado(linha: &str) -> String {
    linha
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn eh_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A marca está no texto respeitando as bordas de identificador: uma marca que
/// começa em letra, dígito ou `_` não pode estar colada a outro identificador
/// antes dela, e o mesmo vale depois.
fn contem(texto: &str, marca: &str) -> bool {
    let bytes = texto.as_bytes();
    let alvo = marca.as_bytes();
    if alvo.is_empty() || alvo.len() > bytes.len() {
        return false;
    }
    let inicio_ok = |i: usize| i == 0 || !eh_ident(alvo[0]) || !eh_ident(bytes[i - 1]);
    let fim_ok =
        |j: usize| j >= bytes.len() || !eh_ident(alvo[alvo.len() - 1]) || !eh_ident(bytes[j]);
    let mut i = 0usize;
    while i + alvo.len() <= bytes.len() {
        if &bytes[i..i + alvo.len()] == alvo && inicio_ok(i) && fim_ok(i + alvo.len()) {
            return true;
        }
        i += 1;
    }
    false
}

/// O que uma varredura leu: os arquivos abertos, os que ficaram de fora (binário,
/// grande demais, além do limite) e se o limite de arquivos foi atingido. É isto
/// que o pacote mostra como cobertura, porque "zero achados" sem leitura nenhuma
/// não é uma conclusão.
#[derive(Debug, Clone, Default)]
pub struct Cobertura {
    pub lidos: i64,
    pub pulados: i64,
    pub cortada: bool,
}

/// Um achado antes de virar linha de banco: sem identificador próprio, porque só
/// recebe um quando é gravado.
#[derive(Debug, Clone)]
pub struct Achado {
    pub rule_id: String,
    pub source: String,
    pub severity: String,
    pub file: String,
    pub line_start: i64,
    pub line_end: i64,
    pub message: String,
    pub cwe: Option<String>,
    pub snippet: Option<String>,
}

impl Achado {
    fn para_linha(self, run_id: i64) -> AnalysisFinding {
        AnalysisFinding {
            id: 0,
            run_id,
            rule_id: self.rule_id,
            source: self.source,
            severity: self.severity,
            file: self.file,
            line_start: self.line_start,
            line_end: self.line_end,
            message: self.message,
            cwe: self.cwe,
            snippet: self.snippet,
            fingerprint: String::new(),
        }
    }
}

/// Varre a árvore com as regras embarcadas. A régua de "esta pasta não é código do
/// projeto" e de "isto é texto legível" vem de [`crate::security`] — a mesma em
/// todo o aplicativo, sem uma segunda lista de pastas para esquecer.
fn varredura_embutida(raiz: &Path, cancel: &AtomicBool) -> (Vec<Achado>, Cobertura) {
    let mut achados = Vec::new();
    let mut cobertura = Cobertura::default();
    for caminho in percorrer(raiz, &mut cobertura) {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let Some(bytes) = security::texto_do_arquivo(&caminho) else {
            cobertura.pulados += 1;
            continue;
        };
        cobertura.lidos += 1;
        let texto = String::from_utf8_lossy(&bytes).to_string();
        let relativo = nome_relativo(raiz, &caminho);
        let sufixo = sufixo(&relativo);
        let fam = familia(&sufixo);
        let mut anterior = String::new();
        for (i, linha) in texto.lines().enumerate() {
            if linha.trim().is_empty() {
                continue;
            }
            let min = linha.to_lowercase();
            let apertado = colado(linha);
            for regra in REGRAS_EMBUTIDAS {
                if !regra.familias.is_empty() && !regra.familias.contains(&fam) {
                    continue;
                }
                if !regra.anterior.is_empty() && !contem(&anterior, regra.anterior) {
                    continue;
                }
                let casa = regra.alternativas.iter().any(|alt| {
                    alt.iter()
                        .all(|marca| contem(escolha(marca, &min, &apertado), marca.texto))
                });
                if casa {
                    achados.push(Achado {
                        rule_id: regra.id.to_string(),
                        source: "embutido".to_string(),
                        severity: regra.severidade.to_string(),
                        file: relativo.clone(),
                        line_start: i as i64 + 1,
                        line_end: i as i64 + 1,
                        message: regra.mensagem.to_string(),
                        cwe: (!regra.cwe.is_empty()).then(|| regra.cwe.to_string()),
                        snippet: Some(security::mascarar(linha.trim_end())),
                    });
                }
            }
            anterior = min;
        }
    }
    (dedupe(achados), cobertura)
}

/// A face da linha que a marca consulta: a crua em minúsculas ou a sem espaços.
fn escolha<'a>(marca: &Marca, min: &'a str, apertado: &'a str) -> &'a str {
    if marca.colado {
        apertado
    } else {
        min
    }
}

/// Os arquivos candidatos da árvore. Pastas que não são código do projeto ficam
/// fora antes de qualquer leitura, e o limite de arquivos é conferido a cada
/// entrada — uma árvore de 200 mil arquivos não pode ser lida até o fim.
fn percorrer(raiz: &Path, cobertura: &mut Cobertura) -> Vec<PathBuf> {
    let Ok(raiz_canonica) = raiz.canonicalize() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut pilha = vec![raiz.to_path_buf()];
    // Uma pasta alcançada por dois caminhos (link simbólico dentro da árvore) só
    // precisa ser lida uma vez; sem isto, um link em ciclo trava a varredura.
    let mut pastas_vistas: BTreeSet<PathBuf> = BTreeSet::new();
    pastas_vistas.insert(raiz_canonica.clone());
    while let Some(pasta) = pilha.pop() {
        if out.len() >= LIMITE_ARQUIVOS {
            cobertura.cortada = true;
            break;
        }
        let Ok(leituras) = std::fs::read_dir(&pasta) else {
            continue;
        };
        for entrada in leituras.flatten() {
            let nome = entrada.file_name().to_string_lossy().to_string();
            let caminho = entrada.path();
            let tipo = entrada.file_type();
            // Link simbólico é resolvido antes de qualquer leitura: um link para
            // fora da raiz varrida não é código do projeto, e lê-lo entregaria à
            // análise (e ao pacote) conteúdo que ninguém pediu para varrer.
            if tipo.as_ref().map(|t| t.is_symlink()).unwrap_or(false) {
                let Ok(alvo) = caminho.canonicalize() else {
                    cobertura.pulados += 1;
                    continue;
                };
                if !alvo.starts_with(&raiz_canonica) {
                    cobertura.pulados += 1;
                    continue;
                }
                if alvo.is_dir() {
                    if !security::pular_dir(&nome) && pastas_vistas.insert(alvo.clone()) {
                        pilha.push(alvo);
                    }
                } else if out.len() < LIMITE_ARQUIVOS {
                    out.push(alvo);
                } else {
                    cobertura.cortada = true;
                    return out;
                }
                continue;
            }
            let eh_dir = tipo
                .map(|t| t.is_dir())
                .unwrap_or_else(|_| caminho.is_dir());
            if eh_dir {
                if !security::pular_dir(&nome) {
                    pilha.push(caminho);
                }
                continue;
            }
            if out.len() >= LIMITE_ARQUIVOS {
                cobertura.cortada = true;
                return out;
            }
            out.push(caminho);
        }
    }
    out
}

fn sufixo(nome: &str) -> String {
    nome.rsplit('.')
        .next()
        .filter(|s| s.len() <= 8 && !s.contains('/'))
        .unwrap_or("")
        .to_lowercase()
}

/// O caminho relativo à raiz varrida, sempre com barra: é o que a tela mostra e o
/// que o SARIF põe em `uri`.
fn nome_relativo(raiz: &Path, caminho: &Path) -> String {
    let cru = caminho
        .strip_prefix(raiz)
        .unwrap_or(caminho)
        .to_string_lossy()
        .replace('\\', "/");
    if cru.is_empty() {
        return caminho
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| caminho.display().to_string());
    }
    cru
}

/// O mesmo achado visto duas vezes — por alternativas que se sobrepõem (medido no
/// semgrep com `pattern-either`) ou pelos dois instrumentos — é gravado uma vez.
/// A chave é regra, arquivo e linha; a origem registrada é a de quem viu primeiro,
/// e a ordem final é a do peso, nunca a de um HashMap.
fn dedupe(achados: Vec<Achado>) -> Vec<Achado> {
    let mut vistos: BTreeMap<(String, String, i64), Achado> = BTreeMap::new();
    for a in achados {
        vistos
            .entry((a.rule_id.clone(), a.file.clone(), a.line_start))
            .or_insert(a);
    }
    let mut out: Vec<Achado> = vistos.into_values().collect();
    out.sort_by(|a, b| {
        peso(&a.severity)
            .cmp(&peso(&b.severity))
            .then(a.file.cmp(&b.file))
            .then(a.line_start.cmp(&b.line_start))
            .then(a.rule_id.cmp(&b.rule_id))
    });
    out
}

fn peso(severidade: &str) -> u8 {
    match severidade {
        "ERROR" => 0,
        "WARNING" => 1,
        _ => 2,
    }
}

// ---------- o adaptador semgrep ----------

/// O que o semgrep devolveu, antes de virar achado. O código de saída está aqui e
/// é só um número: medido, uma varredura completa com um arquivo que ela não
/// interpretou volta com código 2 e JSON inteiro, e outra com um erro volta com
/// código 0. O veredito vem dos dados, nunca dele.
///
/// Os estados de "não sei" têm nome próprio, porque a ação é outra: `sem_resposta`
/// é "havia programa e ele não acordou", `invalida` é "respondeu, mas o que
/// devolveu não se leu". A ausência do programa é dita antes, pela sondagem.
#[derive(Default)]
struct Resposta {
    achados: Vec<Achado>,
    lidos: i64,
    pulados: i64,
    erros: i64,
    code: Option<i32>,
    sem_resposta: bool,
    cancelado: bool,
    invalida: bool,
    nota: Option<String>,
    regras: Vec<String>,
}

impl Resposta {
    /// Havia programa, e ele não acordou a tempo.
    fn sem_resposta(nota: String) -> Self {
        Resposta {
            sem_resposta: true,
            nota: Some(nota),
            ..Resposta::default()
        }
    }

    /// A resposta chegou, mas não diz o que foi varrido: não é "zero achados", é
    /// nenhuma prova. Os achados que por acaso vieram continuam, o veredito não.
    fn invalida(nota: String) -> Self {
        Resposta {
            invalida: true,
            nota: Some(nota),
            ..Resposta::default()
        }
    }
}

/// Roda o semgrep sobre `alvo` e lê o JSON que ele escreveu. `raiz` é o diretório
/// de trabalho do filho, e é por ela que os caminhos do JSON chegam relativos ao
/// que foi varrido. `configs` é a lista de configs a usar: pode incluir tanto o
/// arquivo de regras embarcadas quanto `p/default` para cobertura multi-linguagem.
fn varredura_semgrep(
    binario: &str,
    configs: &[String],
    regras: &Path,
    raiz: &Path,
    prazo: Duration,
    cancel: &AtomicBool,
) -> Resposta {
    // Duas varreduras podem rodar ao mesmo tempo em projetos diferentes: o
    // arquivo de saída é de uma corrida, não do processo. Um nome por PID faria uma
    // ler o JSON da outra — veredito trocado por cima do disco.
    // Medido no semgrep 1.182: o caminho do TEMP vale como foi dado, curto
    // (DOUGLA~1.GRO) ou comprido — o que ele não aceita é o prefixo `\\?\`.
    static SEQUENCIA: AtomicU64 = AtomicU64::new(0);
    let seq = SEQUENCIA.fetch_add(1, Ordering::SeqCst);
    let saida = std::env::temp_dir().join(format!(
        "devops-desktop-semgrep-{}-{seq}.json",
        std::process::id()
    ));
    let arquivo_saida = saida.display().to_string();
    let mut args = vec![
        "scan".to_string(),
        "--metrics=off".to_string(),
        "--json-output".to_string(),
        arquivo_saida.clone(),
    ];
    for config in configs {
        args.push("--config".to_string());
        args.push(config.clone());
    }
    args.push(".".to_string());
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let chamada = rodar(binario, &args_ref, Some(raiz), prazo, cancel);
    if chamada.cancelado {
        let _ = std::fs::remove_file(&saida);
        return Resposta {
            cancelado: true,
            nota: Some("a varredura foi interrompida".to_string()),
            ..Resposta::default()
        };
    }
    if chamada.sem_resposta {
        let _ = std::fs::remove_file(&saida);
        return Resposta::sem_resposta(format!(
            "o semgrep não terminou dentro do prazo de {} s — o que ele varreu não é conhecido",
            prazo.as_secs()
        ));
    }
    let conteudo = ler_resultado(&saida);
    let _ = std::fs::remove_file(&saida);
    let Some(conteudo) = conteudo else {
        return Resposta::invalida(format!(
            "o semgrep não escreveu o resultado (código {:?}); disse: {}",
            chamada.code,
            cauda(&chamada.stderr)
        ));
    };
    let Ok(valor) = serde_json::from_str::<Value>(&conteudo) else {
        return Resposta::invalida(format!(
            "o JSON do semgrep não é legível (código {:?})",
            chamada.code
        ));
    };
    let mut resposta = ler_envelope(&valor, raiz, chamada, regras);
    // O relatório nomeia todas as fontes que estavam em vigor. Sem achado, o
    // envelope não nomeia nenhuma, e o arquivo embarcado sozinho esconderia que
    // o registry também foi consultado.
    if configs.iter().any(|c| c == "p/default") && !resposta.regras.iter().any(|r| r == "p/default")
    {
        resposta.regras.push("p/default".to_string());
    }
    resposta
}

/// Lê o JSON que o semgrep escreveu, com um pouco de insistência. O filho devolve
/// 0 antes de os trabalhadores paralelos dele largarem o arquivo do
/// `--json-output`, e é a varredura grande — a que tem resultado bom pra ler —
/// que chega perto de esbarrar nessa janela. A decodificação é tolerante porque o
/// resultado ecoa linhas do código varrido, e arquivo em latin-1 não é UTF-8.
fn ler_resultado(saida: &Path) -> Option<String> {
    for tentativa in 0..10 {
        if let Ok(bytes) = std::fs::read(saida) {
            if !bytes.is_empty() {
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        if tentativa + 1 < 10 {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    None
}

/// Lê o envelope real do semgrep 1.181: `results[]`, `errors[]`, `paths.scanned`,
/// `paths.skipped`. As três listas que sustentam a prova são obrigatórias: um
/// formato que mudou não pode fazer a varredura mentir sobre o que não estava lá,
/// e um campo ausente não é um campo vazio — `results` que não veio virava
/// "nenhum achado", que é justamente a conclusão que libera o pipeline.
fn ler_envelope(valor: &Value, raiz: &Path, chamada: Chamada, regras: &Path) -> Resposta {
    let Some(results) = valor.get("results").and_then(Value::as_array) else {
        return Resposta::invalida("a resposta do semgrep não traz a lista de achados".to_string());
    };
    let Some(errors) = valor.get("errors").and_then(Value::as_array) else {
        return Resposta::invalida("a resposta do semgrep não traz a lista de erros".to_string());
    };
    let paths = valor.get("paths");
    let Some(scanned) = paths
        .and_then(|p| p.get("scanned"))
        .and_then(Value::as_array)
    else {
        return Resposta::invalida(
            "a resposta do semgrep não diz quais arquivos foram lidos".to_string(),
        );
    };
    match paths.and_then(|p| p.get("skipped")) {
        None | Some(Value::Array(_)) => {}
        Some(_) => {
            return Resposta::invalida(
                "a resposta do semgrep diz quais arquivos pulou de um jeito ilegível".to_string(),
            )
        }
    }

    let mut achados = Vec::new();
    let mut vistos: Vec<String> = Vec::new();
    let mut trechos = Trechos::default();
    let mut sem_forma = 0usize;
    for r in results {
        // Um achado sem regra, sem caminho ou sem linha não tem onde ficar: conta
        // como leitura que não se leu, não como achado que não existia.
        let Some(id) = r.get("check_id").and_then(Value::as_str) else {
            sem_forma += 1;
            continue;
        };
        let Some(arquivo) = r.get("path").and_then(Value::as_str) else {
            sem_forma += 1;
            continue;
        };
        let Some(inicio) = r
            .get("start")
            .and_then(|s| s.get("line"))
            .and_then(Value::as_i64)
        else {
            sem_forma += 1;
            continue;
        };
        // `check_id` é o caminho da config pontuado mais o identificador da
        // regra; o que importa é o identificador, que é o nome que os dois
        // instrumentos compartilham.
        let regra = id.rsplit('.').next().unwrap_or(id).to_string();
        if !vistos.contains(&regra) {
            vistos.push(regra.clone());
        }
        let extra = r.get("extra").cloned().unwrap_or(Value::Null);
        let metadata = extra.get("metadata").cloned().unwrap_or(Value::Null);
        let severidade = extra
            .get("severity")
            .and_then(Value::as_str)
            .or_else(|| metadata.get("severity").and_then(Value::as_str))
            .map(|s| s.to_uppercase())
            // Fora dos três conhecidos entra como WARNING: é o meio termo
            // honesto entre descartar um achado e inflar um aviso.
            .filter(|s| SEVERIDADES.contains(&s.as_str()))
            .unwrap_or_else(|| "WARNING".to_string());
        let fim = r
            .get("end")
            .and_then(|s| s.get("line"))
            .and_then(Value::as_i64)
            .filter(|n| *n >= inicio)
            .unwrap_or(inicio);
        achados.push(Achado {
            rule_id: regra,
            source: "semgrep".to_string(),
            severity: severidade,
            file: nome_relativo(raiz, &Path::new(raiz).join(arquivo.replace('\\', "/"))),
            line_start: inicio,
            line_end: fim,
            message: extra
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("a regra não escreveu mensagem")
                .to_string(),
            cwe: cwe_da_metadata(&metadata),
            // O modo livre não devolve o trecho: medido, `extra.lines` vem
            // escrito "requires login". Então é o aplicativo que lê a linha,
            // e é aqui que ela encontra a mascaráção antes de qualquer gravação.
            snippet: trechos.trecho(raiz, arquivo, inicio, fim),
        });
    }
    let erros = errors.len() as i64;
    let mut motivos: Vec<String> = Vec::new();
    if sem_forma > 0 {
        motivos.push(format!(
            "{sem_forma} achado(s) da resposta vieram sem regra, caminho ou linha — a varredura não é completa"
        ));
    }
    if erros > 0 {
        motivos.push(format!(
            "a ferramenta reportou {erros} erro(s) fora dos achados — o que ela não interpretou não foi varrido"
        ));
    }
    Resposta {
        achados: dedupe(achados),
        lidos: scanned.len() as i64,
        pulados: paths
            .and_then(|p| p.get("skipped"))
            .and_then(Value::as_array)
            .map(|a| a.len() as i64)
            .unwrap_or(0),
        erros,
        code: chamada.code,
        invalida: sem_forma > 0,
        nota: (!motivos.is_empty()).then(|| motivos.join(" · ")),
        regras: efetivamente_lidos(vistos, regras),
        ..Default::default()
    }
}

/// As regras que a varredura tinha. Quando o JSON não nomeia nenhuma — varredura
/// sem achados, o caso mais provável — a lista diz "p/default" (as regras padrão
/// do Semgrep que foram usadas).
fn efetivamente_lidos(vistos: Vec<String>, caminho_das_regras: &Path) -> Vec<String> {
    if !vistos.is_empty() {
        return vistos;
    }
    // Tenta ler o arquivo local (analisador embutido); se não existir, retorna
    // "p/default" que é o config remoto usado pelo Semgrep.
    if caminho_das_regras.as_os_str().is_empty() || !caminho_das_regras.exists() {
        return vec!["p/default".to_string()];
    }
    std::fs::read_to_string(caminho_das_regras)
        .map(|t| ids_do_yaml(&t))
        .unwrap_or_else(|_| vec!["p/default".to_string()])
}

/// `cwe` chega como texto ou como lista de textos, dependendo da regra. Os dois
/// são aceitos; o que sai é uma linha só.
fn cwe_da_metadata(metadata: &Value) -> Option<String> {
    match metadata.get("cwe") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Array(itens)) => {
            let junta = itens
                .iter()
                .filter_map(|i| {
                    i.as_str()
                        .or_else(|| i.get("value").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join(", ");
            (!junta.is_empty()).then_some(junta)
        }
        _ => None,
    }
}

/// O fim da reclamação de uma ferramenta. Duzentos caracteres dizem o que foi; o
/// resto é log, e log de ferramenta não é achado. A máscara age antes do corte: um
/// stderr que citou um caminho de config com token não pode ser gravado como veio.
fn cauda(texto: &str) -> String {
    let limpo = security::mascarar(texto.trim());
    let total = limpo.chars().count();
    if total <= 200 {
        return limpo;
    }
    format!("…{}", limpo.chars().skip(total - 200).collect::<String>())
}

/// Busca a linha que o semgrep apontou e a devolve mascarada.
#[derive(Default)]
struct Trechos {
    cache: HashMap<String, Vec<String>>,
    usado: usize,
}

impl Trechos {
    fn trecho(&mut self, raiz: &Path, arquivo: &str, inicio: i64, fim: i64) -> Option<String> {
        if inicio <= 0 {
            return None;
        }
        let chave = arquivo.replace('\\', "/");
        if !self.cache.contains_key(&chave) {
            if self.usado >= LIMITE_CACHE_BYTES {
                return None;
            }
            let bytes = security::texto_do_arquivo(&raiz.join(&chave))?;
            let texto = String::from_utf8_lossy(&bytes).to_string();
            self.usado += texto.len();
            self.cache.insert(
                chave.clone(),
                texto.lines().map(|l| l.to_string()).collect(),
            );
        }
        let linhas = self.cache.get(&chave)?;
        let i = (inicio - 1) as usize;
        if i >= linhas.len() {
            return None;
        }
        let ate = (fim.max(inicio) as usize)
            .min(linhas.len())
            .min(i + LINHAS_DE_TRECHO)
            .max(i + 1);
        Some(
            linhas[i..ate]
                .iter()
                .map(|l| security::mascarar(l.trim_end()))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

// ---------- a varredura ----------

/// A pasta varrida no caminho canônico, que é a régua da contenção. As três
/// respostas possíveis de uma leitura de caminho são coisas diferentes: o canônico,
/// a pasta que **não está lá** — e é isso que recusa o pedido — e a pasta que **não
/// respondeu**, que devolve o motivo e jamais vira "não existe". `prazo` é parâmetro
/// porque essa é a única forma de medir a espera abandonada sem ter um volume
/// adormecido em mãos; a varredura chama com [`git_probe::PRAZO_SONDA`].
fn alvo_da_varredura(raiz: &Path, pedido: &Path, prazo: Duration) -> Result<PathBuf, String> {
    let raiz_real = match git_probe::canon(raiz, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(e) => {
            return Err(format!("não foi possível abrir a pasta do projeto: {e}"))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(motivo),
    };
    let alvo = match git_probe::canon(pedido, prazo) {
        git_probe::Canon::Resolveu(caminho) => caminho,
        git_probe::Canon::Ausente(_) => {
            return Err(format!(
                "a pasta varrida “{}” não existe dentro do projeto",
                pedido.display()
            ))
        }
        git_probe::Canon::SemResposta(motivo) => return Err(motivo),
    };
    if !alvo.starts_with(&raiz_real) {
        return Err("a varredura só roda dentro da pasta cadastrada do projeto".to_string());
    }
    Ok(alvo)
}

/// O resultado de uma varredura é a corrida como ela ficou gravada, lida de volta
/// do banco: as duas portas (a tela e o nó do pipeline) leem esta mesma linha, e o
/// que elas mostram é o que está guardado, não o que a memória do processo ainda
/// tinha. `blocked` é o veredito — a contagem contra o corte — e nunca o código de
/// saída da ferramenta.
///
/// `raiz` é a pasta cadastrada do projeto; a subpasta opcional do pedido é conferida
/// por [`AnalysisInput::conferir`] e contida pelo caminho canônico, a mesma régua dos
/// nós de comando locais — e com o mesmo prazo, porque abrir a pasta é uma espera de
/// disco.
pub fn executar(
    conn: &Connection,
    dados: &Path,
    projeto: &Project,
    raiz: &Path,
    entrada: &AnalysisInput,
    cancel: &AtomicBool,
) -> Result<AnalysisRun, String> {
    executar_com(conn, dados, projeto, raiz, entrada, cancel, localizar)
}

/// O mesmo caminho de [`executar`], com o localizador da ferramenta aberto para
/// quem testa poder fixar a ausência sem depender do que está instalado na máquina.
fn executar_com(
    conn: &Connection,
    dados: &Path,
    projeto: &Project,
    raiz: &Path,
    entrada: &AnalysisInput,
    cancel: &AtomicBool,
    localizador: impl Fn(Option<&str>) -> Ferramenta,
) -> Result<AnalysisRun, String> {
    let inicio = Instant::now();
    let relativa = entrada.conferir()?;
    let pedido = raiz.join(relativa.as_deref().unwrap_or(""));
    let alvo = alvo_da_varredura(raiz, &pedido, git_probe::PRAZO_SONDA)?;
    let instrumento = entrada.analyzer();
    let corte = entrada.severity_gate();
    let prazo = if entrada.timeout_secs.unwrap_or(0) > 0 {
        Duration::from_secs(entrada.timeout_secs.unwrap_or(0) as u64)
    } else {
        PRAZO_PADRAO
    };
    let quer_externo = instrumento != "embutido";
    let explicito = if quer_externo {
        db::setting_value(conn, CHAVE_CONFIG)?.unwrap_or_default()
    } else {
        String::new()
    };
    let mut ferramenta = if quer_externo {
        localizador(Some(&explicito))
    } else {
        Ferramenta {
            estado: "não-usada".to_string(),
            caminho: None,
            versao: None,
            duracao_ms: 0,
            regras: contar_regras(REGRAS_YAML),
            origem: String::new(),
            apontado: String::new(),
            resposta: String::new(),
        }
    };
    let regras = publicar_regras(dados)?;

    let mut aberto = AnalysisRun {
        id: 0,
        project_id: projeto.id,
        project: projeto.name.clone(),
        root: alvo.display().to_string(),
        analyzer: instrumento.clone(),
        tool: nome_do_instrumento(&instrumento, &ferramenta),
        tool_version: ferramenta.versao.clone(),
        rules: String::new(),
        severity_gate: corte.clone(),
        status: "rodando".to_string(),
        started_at: db::now_rfc3339(),
        finished_at: None,
        duration_ms: 0,
        files_scanned: 0,
        files_skipped: 0,
        exit_code: None,
        tool_errors: 0,
        blocked: false,
        counts: AnalysisCounts::default(),
        note: None,
    };
    let run_id = db::abrir_analise(conn, &aberto)?;

    let mut notas: Vec<String> = Vec::new();
    let mut regras_usadas: Vec<String> = Vec::new();
    let mut cobertura = Cobertura::default();
    let mut achados: Vec<Achado> = Vec::new();
    let mut exit_code = None;
    let mut tool_errors = 0;
    // O instrumento externo foi pedido e não entregou prova: não estava lá, não
    // respondeu, ou respondeu de um jeito que não se leu. É diferente de "varreu e
    // não achou nada", e é isso que a nota e o corte precisam dizer.
    let mut externo_sem_prova = false;

    // O caminho apontado não pode esconder a ferramenta que a busca automática
    // achou. A varredura segue com ela, mas a nota guarda que o apontado não serviu.
    if quer_externo && ferramenta.apontado_ignorado() {
        notas.push(format!(
            "o caminho apontado em {CHAVE_CONFIG} não serve; foi usado {} ({})",
            ferramenta.caminho.as_deref().unwrap_or("semgrep"),
            ferramenta.origem
        ));
    }

    // Instalação automática só quando ninguém apontou um caminho e o programa
    // realmente não está aqui. A busca automática já tentou PATH e scripts do
    // Python mesmo com um valor apontado; se nada apareceu, a nota diz o que foi
    // tentado. "Sem-resposta" é programa presente que não acordou, que instalar
    // por cima não conserta.
    if quer_externo && ferramenta.estado == "ausente" && explicito.trim().is_empty() {
        notas.push("o semgrep não está nesta máquina — tentando instalar com pip/pipx".to_string());
        match instalar_semgrep(cancel) {
            Ok(caminho) => {
                notas.push(format!("semgrep instalado em {caminho}"));
                ferramenta = localizador(Some(&caminho));
                aberto.tool = nome_do_instrumento(&instrumento, &ferramenta);
                aberto.tool_version = ferramenta.versao.clone();
            }
            Err(erro) => notas.push(erro),
        }
        if cancel.load(Ordering::SeqCst) {
            return fechar_interrompida(conn, run_id, &aberto, inicio);
        }
    }

    if instrumento != "semgrep" {
        let (embutidos, dele) = varredura_embutida(&alvo, cancel);
        if cancel.load(Ordering::SeqCst) {
            return fechar_interrompida(conn, run_id, &aberto, inicio);
        }
        achados = embutidos;
        cobertura = dele;
        regras_usadas.extend(ids_embarcados().iter().map(|s| s.to_string()));
    }
    if quer_externo {
        if ferramenta.encontrada() {
            // Duas fontes, uma chamada: as 19 regras da casa cobrem o que o
            // registry não conhece do domínio, e `p/default` cobre as linguagens
            // que o YAML embarcado não olha. A primeira execução baixa o
            // registry — daí o prazo padrão maior.
            let configs = vec![regras.display().to_string(), "p/default".to_string()];
            let resposta = varredura_semgrep(
                ferramenta.caminho.as_deref().unwrap_or("semgrep"),
                &configs,
                &regras,
                &alvo,
                prazo,
                cancel,
            );
            if resposta.cancelado {
                return fechar_interrompida(conn, run_id, &aberto, inicio);
            }
            exit_code = resposta.code;
            tool_errors = resposta.erros;
            // Os dois instrumentos leem a mesma árvore: somar os arquivos lidos
            // contaria o mesmo arquivo duas vezes em `ambos`. O maior é a leitura
            // de fato comprovada por algum deles; a nota de cobertura parcial
            // continua vindo de cada resposta, não deste número.
            cobertura.lidos = cobertura.lidos.max(resposta.lidos);
            cobertura.pulados = cobertura.pulados.max(resposta.pulados);
            achados.extend(resposta.achados);
            regras_usadas.extend(resposta.regras);
            if let Some(nota) = resposta.nota {
                notas.push(nota);
            }
            externo_sem_prova = resposta.sem_resposta || resposta.invalida;
        } else {
            externo_sem_prova = true;
            notas.push(match ferramenta.estado.as_str() {
                "sem-resposta" => "o semgrep existe mas não respondeu à sondagem de versão".to_string(),
                _ if explicito.trim().is_empty() => format!(
                    "o semgrep não estava disponível nesta máquina e a instalação automática não o trouxe — instale-o ou aponte o caminho em {CHAVE_CONFIG}"
                ),
                _ => format!(
                    "o caminho apontado em {CHAVE_CONFIG} não serve — instale-o ou aponte o caminho certo"
                ),
            });
        }
    }

    let achados = dedupe(achados);
    let guardados = if achados.len() > LIMITE_ACHADOS {
        notas.push(format!(
            "a varredura achou {} achados e {} foram guardados",
            achados.len(),
            LIMITE_ACHADOS
        ));
        &achados[..LIMITE_ACHADOS]
    } else {
        &achados[..]
    };
    let linhas: Vec<AnalysisFinding> = guardados
        .iter()
        .cloned()
        .map(|a| a.para_linha(run_id))
        .collect();
    db::gravar_achados(conn, run_id, &linhas)?;
    let counts = db::contar_achados(conn, run_id)?;

    if cobertura.cortada {
        notas.push(format!(
            "a varredura parou em {LIMITE_ARQUIVOS} arquivos — a cobertura é parcial"
        ));
    }
    let sem_leitura = cobertura.lidos == 0;
    if sem_leitura {
        notas.push(
            "a varredura não leu arquivo nenhum — não há o que afirmar sobre o código".to_string(),
        );
    }
    // Um corte é uma promessa de leitura completa. "Não consegui ler" não é "está
    // limpo": quando o instrumento pedido não entregou prova, ou quando não houve
    // leitura nenhuma, a corrida é dita falha — os achados que existem continuam
    // gravados, e é a nota que diz o que falta.
    let sem_prova = sem_leitura || externo_sem_prova;
    // Cobertura parcial não é ausência de prova — é prova de menos. Com um corte
    // declarado, a promessa vale para a árvore inteira, então ela barra; sem corte,
    // o nó informa e a nota diz o que ficou de fora.
    let cobertura_parcial =
        cobertura.cortada || tool_errors > 0 || externo_sem_prova || sem_leitura;
    aberto.id = run_id;
    aberto.status = if sem_prova {
        "falhou".to_string()
    } else {
        "concluida".to_string()
    };
    aberto.finished_at = Some(db::now_rfc3339());
    aberto.duration_ms = duracao_ms(inicio);
    aberto.files_scanned = cobertura.lidos;
    aberto.files_skipped = cobertura.pulados;
    aberto.exit_code = exit_code.map(i64::from);
    aberto.tool_errors = tool_errors;
    aberto.blocked = barra_o_corte(&corte, &counts) || (!corte.is_empty() && cobertura_parcial);
    aberto.counts = counts;
    aberto.rules = regras_usadas.join(", ");
    aberto.note = (!notas.is_empty()).then(|| notas.join(" · "));
    db::fechar_analise(conn, run_id, &aberto)?;

    db::obter_analise(conn, run_id)
}

/// Fecha a corrida como interrompida. Uma varredura cortada no meio não é uma
/// varredura limpa, e é assim que ela tem de aparecer no histórico: com o corte
/// declarado, a promessa não cumprida barra; é o nó que devolve o cancelamento,
/// não um sucesso com a nota no meio.
fn fechar_interrompida(
    conn: &Connection,
    run_id: i64,
    aberto: &AnalysisRun,
    inicio: Instant,
) -> Result<AnalysisRun, String> {
    let mut fechado = aberto.clone();
    fechado.id = run_id;
    fechado.status = "interrompida".to_string();
    fechado.finished_at = Some(db::now_rfc3339());
    fechado.duration_ms = duracao_ms(inicio);
    fechado.blocked = !fechado.severity_gate.is_empty();
    fechado.note = Some("a varredura foi interrompida antes do fim".to_string());
    db::fechar_analise(conn, run_id, &fechado)?;
    db::obter_analise(conn, run_id)
}

/// O corte é uma frase sobre o futuro: "com qualquer coisa deste nível para cima,
/// a publicação para". Vazio é "nada barra".
pub fn barra_o_corte(corte: &str, counts: &AnalysisCounts) -> bool {
    !corte.is_empty() && achados_no_corte(corte, counts) > 0
}

/// Quantos achados estão no corte declarado ou acima dele. É outro número que o
/// total: dizer "barram 1.358 achados" quando o corte é ERROR e os erros são 676
/// seria inventar o resto.
fn achados_no_corte(corte: &str, counts: &AnalysisCounts) -> i64 {
    match corte {
        "ERROR" => counts.error,
        "WARNING" => counts.error + counts.warning,
        _ => counts.total(),
    }
}

fn nome_do_instrumento(instrumento: &str, ferramenta: &Ferramenta) -> String {
    match (instrumento, ferramenta.encontrada()) {
        ("semgrep", true) => "semgrep".to_string(),
        ("semgrep", false) => "semgrep ausente".to_string(),
        ("ambos", true) => "embutido + semgrep".to_string(),
        ("ambos", false) => "embutido + semgrep ausente".to_string(),
        _ => "embutido".to_string(),
    }
}

/// O que a tela mostra sobre a ferramenta, sem varrer nada.
pub fn sonda_ferramenta(conn: &Connection) -> Result<Ferramenta, String> {
    let explicito = db::setting_value(conn, CHAVE_CONFIG)?.unwrap_or_default();
    Ok(localizar(Some(&explicito)))
}

// ---------- o pacote para uma IA decidir ----------

/// O pacote que uma IA lê: `devops-desktop/analise@1`.
///
/// É determinístico — a mesma corrida sempre produz o mesmo texto, na mesma ordem
/// — e diz o que **não** foi lido, porque um modelo que vê só os achados conclui
/// "limpo" onde a verdade é "a ferramenta não estava aqui". Os trechos já
/// passaram por [`crate::security::mascarar`] na gravação, então nenhum valor de
/// credencial sai daqui, e `codigoDeSaidaEProva` diz em uma linha que o número da
/// ferramenta não é um veredito.
pub fn pacote(conn: &Connection, run_id: i64, limite: Option<usize>) -> Result<Value, String> {
    let run = db::obter_analise(conn, run_id)?;
    let limite = limite.unwrap_or(LIMITE_DO_PACOTE).clamp(1, LIMITE_ACHADOS);
    let achados = db::listar_achados(conn, run_id, None, limite as i64 + 1)?;
    let truncado = achados.len() > limite;
    let mostrados = if truncado {
        &achados[..limite]
    } else {
        &achados[..]
    };
    let veredito = classificar(&run);
    let decisoes = decisoes(&run, &veredito);
    let comparacao = db::comparar_analises(conn, run_id)?;
    let exemplos_corrigidos: Vec<Value> = comparacao
        .corrigidos_itens
        .iter()
        .take(20)
        .map(|a| {
            json!({
                "regra": a.rule_id,
                "severidade": a.severity,
                "arquivo": a.file,
                "linha": a.line_start,
                "mensagem": a.message,
            })
        })
        .collect();

    Ok(json!({
        "esquema": "devops-desktop/analise@1",
        "geradoEm": db::now_rfc3339(),
        "projeto": {
            "id": run.project_id,
            "nome": run.project,
            "pasta": run.root,
        },
        "execucao": {
            "id": run.id,
            "instrumento": run.analyzer,
            "ferramenta": run.tool,
            "versao": run.tool_version,
            "regras": run.rules.split(", ").filter(|s| !s.is_empty()).collect::<Vec<_>>(),
            "corte": run.severity_gate,
            "status": run.status,
            "inicio": run.started_at,
            "fim": run.finished_at,
            "duracaoMs": run.duration_ms,
            "codigoDeSaidaDaFerramenta": run.exit_code,
            "codigoDeSaidaEProva": false,
            "errosDaFerramenta": run.tool_errors,
        },
        "cobertura": {
            "arquivosLidos": run.files_scanned,
            "arquivosPulados": run.files_skipped,
            "achadosGuardados": run.counts.total(),
        },
        "veredito": veredito,
        "decisoes": decisoes,
        // A comparação é o que transforma uma lista de achados em decisão: o que
        // esta corrida trouxe, o que continua e o que sumiu. Sem corrida anterior
        // comparável, `base` é nulo — e isso não é "tudo novo".
        "comparacao": {
            "base": comparacao.baseline.as_ref().map(|b| json!({
                "id": b.id,
                "inicio": b.started_at,
                "ferramenta": b.tool,
                "versao": b.tool_version,
            })),
            "novos": comparacao.novos,
            "persistentes": comparacao.persistentes,
            "corrigidos": comparacao.corrigidos,
            "corrigidosConclusivos": comparacao.corrigidos_conclusivos,
            "corrigidosExemplos": exemplos_corrigidos,
        },
        "porRegra": agrupar_por_regra(mostrados),
        "achados": mostrados.iter().map(|a| json!({
            "impressao": a.fingerprint,
            "regra": a.rule_id,
            "origem": a.source,
            "severidade": a.severity,
            "arquivo": a.file,
            "linha": a.line_start,
            "linhaAte": a.line_end,
            "mensagem": a.message,
            "cwe": a.cwe,
            "trechoMascarado": a.snippet,
        })).collect::<Vec<_>>(),
        "truncado": truncado,
        "nota": run.note,
        "observacao": "nenhum valor de credencial está aqui: os trechos passam pela mascaráção de security.rs antes de serem gravados",
    }))
}

/// A classificação da corrida, em palavras das quais uma máquina escolhe ação. É
/// derivada do que foi gravado, nunca do código de saída da ferramenta.
fn classificar(run: &AnalysisRun) -> Value {
    let total = run.counts.total();
    let rotulo = if run.status == "interrompida" {
        "varredura-interrompida"
    } else if run.files_scanned == 0 {
        "cobertura-nula"
    } else if run.status == "falhou" || run.tool.contains("ausente") {
        // O instrumento pedido não entregou prova. Os números que existem são os
        // dele, e a ausência do resto não pode ser lida como "sem achados".
        "leitura-incompleta"
    } else if run.blocked {
        "barra"
    } else if run.tool_errors > 0 {
        "revalidar-varredura"
    } else if total == 0 {
        "sem-achados"
    } else {
        "achados-abaixo-do-corte"
    };
    json!({
        "classificacao": rotulo,
        "bloqueia": run.blocked,
        "corte": run.severity_gate,
        "contagem": {
            "error": run.counts.error,
            "warning": run.counts.warning,
            "info": run.counts.info,
            "total": total,
            "noCorte": achados_no_corte(&run.severity_gate, &run.counts),
        },
    })
}

/// As ações que os números sustentam, cada uma apontando para o campo do próprio
/// pacote que a justifica. Quem recebe isto escolhe; nada aqui escolheu sozinho.
fn decisoes(run: &AnalysisRun, veredito: &Value) -> Vec<Value> {
    let rotulo = veredito
        .get("classificacao")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut out = Vec::new();
    if run.blocked {
        out.push(json!({
            "acao": "parar-a-publicacao",
            "motivo": format!(
                "{} achado(s) no corte {} ou acima",
                achados_no_corte(&run.severity_gate, &run.counts),
                if run.severity_gate.is_empty() { "nenhum" } else { &run.severity_gate }
            ),
            "evidencia": "veredito.contagem.noCorte",
        }));
    }
    match rotulo {
        "cobertura-nula" => out.push(json!({
            "acao": "revisar-o-pedido",
            "motivo": "a varredura não abriu arquivo nenhum; não há conclusão possível sobre o código",
            "evidencia": "cobertura.arquivosLidos",
        })),
        "varredura-interrompida" => out.push(json!({
            "acao": "rodar-de-novo",
            "motivo": "a varredura foi cortada antes do fim",
            "evidencia": "execucao.status",
        })),
        "leitura-incompleta" => out.push(json!({
            "acao": "refazer-a-varredura",
            "motivo": "o instrumento pedido não entregou prova; o que ele não leu não pode ser dado como limpo",
            "evidencia": "execucao.status",
        })),
        _ => {}
    }
    if run.tool_errors > 0 {
        out.push(json!({
            "acao": "revalidar-varredura",
            "motivo": format!("a ferramenta reportou {} erro(s) de interpretação fora dos achados; parte da árvore não foi varrida", run.tool_errors),
            "evidencia": "execucao.errosDaFerramenta",
        }));
    }
    if run.tool.contains("ausente") {
        out.push(json!({
            "acao": "instalar-ou-apontar-a-ferramenta",
            "motivo": "o instrumento externo pedido não estava disponível nesta máquina",
            "evidencia": "execucao.ferramenta",
        }));
    }
    if run.files_skipped > 0 {
        out.push(json!({
            "acao": "conferir-a-cobertura",
            "motivo": format!("{} arquivo(s) ficaram de fora (binários ou acima do limite de tamanho)", run.files_skipped),
            "evidencia": "cobertura.arquivosPulados",
        }));
    }
    if out.is_empty() {
        out.push(json!({
            "acao": "prosseguir",
            "motivo": "nada no corte declarado, com leitura efetiva e sem erro de ferramenta",
            "evidencia": "veredito.contagem",
        }));
    }
    out
}

/// Os achados resumidos por regra: o que é preciso para priorizar sem ler todas as
/// linhas.
fn agrupar_por_regra(achados: &[AnalysisFinding]) -> Vec<Value> {
    let mut contar: BTreeMap<&str, (usize, Vec<&str>, &str)> = BTreeMap::new();
    for a in achados {
        let entrada =
            contar
                .entry(a.rule_id.as_str())
                .or_insert((0, Vec::new(), a.severity.as_str()));
        entrada.0 += 1;
        if !entrada.1.contains(&a.file.as_str()) {
            entrada.1.push(a.file.as_str());
        }
    }
    contar
        .into_iter()
        .map(|(regra, (quantidade, arquivos, severidade))| {
            json!({
                "regra": regra,
                "severidade": severidade,
                "quantidade": quantidade,
                "arquivosDiferentes": arquivos.len(),
                "exemplos": arquivos.iter().take(3).copied().collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// SARIF 2.1.0 da mesma corrida, para abrir o resultado em outra ferramenta. É
/// gerado das linhas gravadas, então vale para os dois instrumentos e não depende
/// de a ferramenta externa estar presente.
pub fn sarif(conn: &Connection, run_id: i64) -> Result<Value, String> {
    let run = db::obter_analise(conn, run_id)?;
    let achados = db::listar_achados(conn, run_id, None, LIMITE_ACHADOS as i64)?;
    let comparacao = db::comparar_analises(conn, run_id)?;
    let base = match comparacao.baseline.as_ref() {
        Some(b) => Some(db::impressoes_da_corrida(conn, b.id)?),
        None => None,
    };

    // A ordem das regras é a ordem do `ruleIndex` dos resultados. `BTreeMap`
    // garante que duas exportações da mesma corrida tenham o mesmo índice.
    let mut por_regra: BTreeMap<&str, (u8, &str, Option<&str>)> = BTreeMap::new();
    for a in &achados {
        let detalhe = REGRAS_EMBUTIDAS.iter().find(|r| r.id == a.rule_id);
        let entrada = por_regra.entry(a.rule_id.as_str()).or_insert((
            peso(&a.severity),
            detalhe.map(|r| r.mensagem).unwrap_or(a.message.as_str()),
            detalhe
                .map(|r| r.cwe)
                .filter(|c| !c.is_empty())
                .or(a.cwe.as_deref()),
        ));
        entrada.0 = entrada.0.min(peso(&a.severity));
        if entrada.1.is_empty() {
            entrada.1 = a.message.as_str();
        }
        if entrada.2.is_none() {
            entrada.2 = a.cwe.as_deref();
        }
    }
    let indice: BTreeMap<&str, usize> = por_regra
        .keys()
        .enumerate()
        .map(|(i, regra)| (*regra, i))
        .collect();
    let regras: Vec<Value> = por_regra
        .iter()
        .map(|(regra, (nivel, descricao, cwe))| {
            let nivel = *nivel;
            let descricao = *descricao;
            let cwe = *cwe;
            let link = cwe.and_then(link_do_cwe);
            json!({
                "id": regra,
                "name": regra,
                "shortDescription": { "text": descricao },
                "fullDescription": { "text": descricao },
                "helpUri": link,
                "defaultConfiguration": {
                    "level": nivel_da_severidade_por_peso(nivel),
                },
                "properties": {
                    "tags": ["security", cwe.unwrap_or("análise")],
                    "security-severity": peso_do_security_severity(nivel),
                },
            })
        })
        .collect();
    let results: Vec<Value> = achados
        .iter()
        .map(|a| {
            let mut resultado = json!({
                "ruleId": a.rule_id,
                "ruleIndex": indice.get(a.rule_id.as_str()).copied().unwrap_or(0),
                "level": nivel_da_severidade(&a.severity),
                "message": { "text": a.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": a.file.replace('\\', "/") },
                        "region": { "startLine": a.line_start, "endLine": a.line_end },
                    }
                }],
                "partialFingerprints": {
                    "devopsFingerprint/v1": a.fingerprint,
                },
                "properties": {
                    "source": a.source,
                    "cwe": a.cwe,
                },
            });
            if let Some(base) = &base {
                let mapa = resultado.as_object_mut().expect("resultado é objeto");
                mapa.insert(
                    "baselineState".to_string(),
                    Value::String(
                        if base.contains(&a.fingerprint) {
                            "unchanged"
                        } else {
                            "new"
                        }
                        .to_string(),
                    ),
                );
            }
            resultado
        })
        .collect();
    let mut invocacao = json!({
        "executionSuccessful": run.status == "concluida",
        "startTimeUtc": run.started_at,
        "endTimeUtc": run.finished_at,
        "exitCode": run.exit_code,
        "properties": {
            "status": run.status,
            "blocked": run.blocked,
            "filesScanned": run.files_scanned,
            "filesSkipped": run.files_skipped,
            "toolErrors": run.tool_errors,
            "new": comparacao.novos,
            "persisting": comparacao.persistentes,
            "fixed": comparacao.corrigidos,
        },
    });
    if let Some(nota) = &run.note {
        invocacao["toolExecutionNotifications"] = json!([{
            "level": "warning",
            "message": { "text": nota },
        }]);
    }
    Ok(json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "devops-desktop",
                    "fullName": "DevOps Desktop — análise de código",
                    "version": run.tool_version.clone().unwrap_or_else(|| "desconhecida".to_string()),
                    "informationUri": "https://github.com/DouglasGDev/devops-desktop",
                    "rules": regras,
                }
            },
            "invocations": [invocacao],
            "results": results,
            "columnKind": "utf16CodeUnits",
            "properties": {
                "project": run.project,
                "analyzer": run.analyzer,
                "tool": run.tool,
                "severityGate": run.severity_gate,
                "baselineRunId": comparacao.baseline.as_ref().map(|b| b.id),
            },
        }],
    }))
}

fn nivel_da_severidade_por_peso(peso: u8) -> &'static str {
    match peso {
        0 => "error",
        1 => "warning",
        _ => "note",
    }
}

/// O que o GitHub code scanning lê para ordenar alertas. É uma nota no formato do
/// CVSS que acompanha a severidade da regra, não uma medição do achado.
fn peso_do_security_severity(peso: u8) -> &'static str {
    match peso {
        0 => "8.0",
        1 => "5.0",
        _ => "2.0",
    }
}

/// O número de um `CWE-78: …` vira o link oficial da fraqueza. Sem número, sem
/// link — inventar uma URL seria pior do que não ter ajuda.
fn link_do_cwe(cwe: &str) -> Option<String> {
    let resto = cwe.strip_prefix("CWE-")?;
    let numero: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
    if numero.is_empty() {
        return None;
    }
    Some(format!(
        "https://cwe.mitre.org/data/definitions/{numero}.html"
    ))
}

fn nivel_da_severidade(severidade: &str) -> &'static str {
    match severidade {
        "ERROR" => "error",
        "WARNING" => "warning",
        _ => "note",
    }
}

/// As extensões que cada formato aceita. O nome é de quem exporta, o conteúdo é do
/// formato: um `.html` escrito com JSON dentro seria aberto por alguém esperando
/// outra coisa, e aí o arquivo entregue deixa de ser prova de nada.
fn extensoes_do_formato(formato: &str) -> &'static [&'static str] {
    match formato {
        "pacote" => &["json"],
        "sarif" => &["sarif", "json"],
        _ => &[],
    }
}

/// Grava o pacote ou o SARIF **no arquivo que a pessoa escolheu** e devolve o
/// caminho escrito. O arquivo existe para ser entregue a alguém — uma IA, uma
/// revisão, outra ferramenta — e entregar é um lugar que essa pessoa consiga abrir:
/// o aplicativo não escolhe a pasta por ela, e um caminho que não serve não escreve
/// arquivo nenhum.
pub fn exportar(
    conn: &Connection,
    destino: &Path,
    run_id: i64,
    formato: &str,
    limite: Option<usize>,
) -> Result<String, String> {
    let valor = match formato {
        "pacote" => pacote(conn, run_id, limite)?,
        "sarif" => sarif(conn, run_id)?,
        outro => {
            return Err(format!(
                "a exportação escreve pacote ou sarif, não “{outro}”"
            ))
        }
    };
    if destino.as_os_str().is_empty() || !destino.is_absolute() {
        return Err(format!(
            "“{}” não é um arquivo com caminho completo — escolha a pasta onde a exportação vai ficar",
            destino.display()
        ));
    }
    let sufixo = destino
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let aceitas = extensoes_do_formato(formato);
    if !aceitas.contains(&sufixo.as_str()) {
        return Err(format!(
            "um arquivo de {formato} termina em {}; “{}” não termina",
            aceitas
                .iter()
                .map(|e| format!(".{e}"))
                .collect::<Vec<_>>()
                .join(" ou "),
            destino.display()
        ));
    }
    let texto = serde_json::to_string_pretty(&valor)
        .map_err(|e| format!("falha ao escrever a exportação: {e}"))?;
    std::fs::write(destino, texto)
        .map_err(|e| format!("não foi possível gravar {}: {e}", destino.display()))?;
    Ok(git_probe::simple(destino).display().to_string())
}

// ---------- cancelamento ----------

/// As varreduras em andamento, por projeto: a porta do botão "interromper" da
/// tela. Um nó `analise` do pipeline usa a flag da própria corrida e não passa
/// por aqui.
#[derive(Default)]
pub struct Estado {
    ativas: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl Estado {
    pub fn novo() -> Self {
        Self::default()
    }

    /// A flag de quem vai varrer este projeto. Se já há uma varredura dele em
    /// andamento o pedido é recusado: as duas competem pelo disco e o histórico
    /// não diria qual das duas escreveu o quê.
    pub fn iniciar(&self, project_id: i64) -> Result<Arc<AtomicBool>, String> {
        let mut guard = self
            .ativas
            .lock()
            .map_err(|_| "registrador de varreduras indisponível".to_string())?;
        if guard.contains_key(&project_id) {
            return Err("já há uma varredura deste projeto em andamento".to_string());
        }
        let flag = Arc::new(AtomicBool::new(false));
        guard.insert(project_id, flag.clone());
        Ok(flag)
    }

    pub fn terminar(&self, project_id: i64) {
        if let Ok(mut guard) = self.ativas.lock() {
            guard.remove(&project_id);
        }
    }

    /// Levanta a flag de quem está rodando e diz se havia alguém rodando.
    pub fn interromper(&self, project_id: i64) -> bool {
        let Ok(guard) = self.ativas.lock() else {
            return false;
        };
        match guard.get(&project_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    pub fn em_andamento(&self) -> Vec<i64> {
        self.ativas
            .lock()
            .map(|g| {
                let mut ids: Vec<i64> = g.keys().copied().collect();
                ids.sort_unstable();
                ids
            })
            .unwrap_or_default()
    }
}

/// A pasta canônica do projeto — a mesma régua dos nós de comando: é por ela que a
/// varredura não sai do que está cadastrado.
pub fn raiz_do_projeto(projeto: &Project) -> Result<PathBuf, String> {
    git_probe::canonical_dir(&projeto.local_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uma pasta de teste única. O nome leva o prefixo e um contador para duas
    /// execuções do mesmo teste não lerem a árvore uma da outra.
    fn pasta_de_teste(prefixo: &str) -> PathBuf {
        static NUMERO: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NUMERO.fetch_add(1, Ordering::SeqCst);
        let caminho =
            std::env::temp_dir().join(format!("dd-analise-{prefixo}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&caminho);
        std::fs::create_dir_all(&caminho).unwrap();
        caminho
    }

    fn escrever(pasta: &Path, nome: &str, conteudo: &str) {
        let caminho = pasta.join(nome);
        if let Some(p) = caminho.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(caminho, conteudo).unwrap();
    }

    fn arvore_de_teste() -> PathBuf {
        let pasta = pasta_de_teste("casa");
        escrever(
            &pasta,
            "ruim/c.py",
            "import os, subprocess, hashlib, pickle, ssl, yaml, requests\n\
             def f(cmd, url, cur):\n\
             \x20   subprocess.run(\"x\", shell=True)\n\
             \x20   os.system(\"ls\")\n\
             \x20   cur.execute(f\"SELECT {cmd}\")\n\
             \x20   cur.execute(\"SELECT \" + cmd)\n\
             \x20   pickle.loads(b\"x\")\n\
             \x20   yaml.load(\"x\")\n\
             \x20   hashlib.md5(b\"x\")\n\
             \x20   requests.get(url, verify=False)\n\
             \x20   ssl._create_unverified_context()\n\
             \x20   eval(cmd)\n\
             \x20   compile(cmd, \"<s>\", \"exec\")\n\
             \x20   try:\n\
             \x20       g()\n\
             \x20   except Exception:\n\
             \x20       pass\n\
             \n\
             app.run(host=\"0.0.0.0\", debug=True)\n\
             CORS = [\"*\"]  # allow_origins = [\"*\"]\n",
        );
        escrever(
            &pasta,
            "ruim/r.js",
            "el.innerHTML = html;\n\
             document.write(x);\n\
             db.query(`SELECT ${v}`);\n\
             tls.connect({host: h, rejectUnauthorized: false});\n\
             const g = new Function(\"return 1\");\n\
             eval(code);\n\
             crypto.createHash(\"md5\");\n\
             debugger;\n\
             // eslint-disable-next-line\n",
        );
        escrever(
            &pasta,
            "ruim/s.php",
            "<?php\nsystem(\"ls \" . $in);\nunserialize($_GET[\"o\"]);\nreturn md5(\"s\");\nheader('Access-Control-Allow-Origin: *');\n",
        );
        escrever(&pasta, "ruim/n.txt", "# nosemgrep: dd-101-shell-true\n");
        escrever(
            &pasta,
            "limpo/bom.py",
            "import subprocess\ndef f(cur, nome):\n    subprocess.run([\"ls\", nome], check=True)\n\
             \x20   cur.execute(\"SELECT n FROM t WHERE n = %s\", (nome,))\n\
             \x20   return hashlib.sha256(nome.encode()).hexdigest()\n",
        );
        escrever(
            &pasta,
            "limpo/bom.js",
            "const {Client} = require(\"pg\");\nfunction pinta(el, texto) { el.textContent = texto; }\n\
             async function q(sql, valores) { return client.query(sql, valores); }\n",
        );
        pasta
    }

    #[test]
    fn as_duas_listas_de_regras_tem_os_mesmos_ids_na_mesma_ordem() {
        // Os dois instrumentos têm de falar a mesma língua, ou `ambos` produz duas
        // listas que ninguém soma.
        let embutidas: Vec<String> = ids_embarcados()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(embutidas, ids_do_yaml(REGRAS_YAML));
        assert_eq!(contar_regras(REGRAS_YAML), REGRAS_EMBUTIDAS.len());
        assert_eq!(REGRAS_EMBUTIDAS.len(), 19);
    }

    #[test]
    fn cada_regra_embutida_ca_no_codigo_que_deve_caer() {
        let pasta = arvore_de_teste();
        let (achados, cobertura) = varredura_embutida(&pasta, &AtomicBool::new(false));
        let vistas: Vec<&str> = achados.iter().map(|a| a.rule_id.as_str()).collect();
        for regra in REGRAS_EMBUTIDAS {
            assert!(
                vistas.contains(&regra.id),
                "a regra {} não disparou",
                regra.id
            );
        }
        assert!(
            cobertura.lidos >= 5,
            "poucos arquivos lidos: {}",
            cobertura.lidos
        );
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn o_codigo_bom_nao_acusa_nada() {
        // Uma regra que grita em tudo não é uma regra.
        let pasta = pasta_de_teste("limpo");
        escrever(
            &pasta,
            "bom.py",
            "import subprocess\ndef f(cur, nome):\n    subprocess.run([\"ls\", nome], check=True)\n\
             \x20   cur.execute(\"SELECT n FROM t WHERE n = %s\", (nome,))\n\
             \x20   return hashlib.sha256(nome.encode()).hexdigest()\n\
             \x20   print(nome)\n",
        );
        escrever(
            &pasta,
            "bom.js",
            "const {Client} = require(\"pg\");\nfunction pinta(el, texto) { el.textContent = texto; }\n\
             async function q(sql, valores) { return client.query(sql, valores); }\n",
        );
        escrever(
            &pasta,
            "bom.php",
            "<?php\n$hesita = password_hash($s, PASSWORD_DEFAULT);\n",
        );
        let (achados, cobertura) = varredura_embutida(&pasta, &AtomicBool::new(false));
        assert_eq!(cobertura.lidos, 3);
        assert!(
            achados.is_empty(),
            "{:?}",
            achados
                .iter()
                .map(|a| (&a.file, a.line_start, &a.rule_id))
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn marca_de_identificador_respeita_a_borda() {
        assert!(contem("return md5(", "md5("));
        assert!(!contem("return sem_md5(", "md5("));
        assert!(!contem("password = x", "pass"));
        assert!(contem("        pass", "pass"));
        assert!(contem("x = re.compile(", "compile("));
        assert!(!contem("x = re.compile(", "eval("));
    }

    #[test]
    fn a_marca_colada_ignora_a_formatacao() {
        assert!(contem(&colado("verify = False"), "verify=false"));
        assert!(contem(&colado("el.innerHTML = html"), ".innerhtml="));
        assert!(!contem(&colado("el.textContent = texto"), ".innerhtml="));
        // `eslint-disable-next-line` mantém a marca: hífen não é caractere de
        // identificador para a borda, senão a marca acabaria em `-` e sumiria.
        assert!(contem(
            &colado("// eslint-disable-next-line"),
            "eslint-disable"
        ));
    }

    #[test]
    fn a_varredura_para_quando_a_flag_levanta() {
        let pasta = pasta_de_teste("cancela");
        for i in 0..40 {
            escrever(&pasta, &format!("a{i}.py"), "x = os.system('ls')\n");
        }
        let flag = AtomicBool::new(true);
        let (achados, cobertura) = varredura_embutida(&pasta, &flag);
        assert!(achados.is_empty(), "rodou depois de cancelada");
        assert_eq!(cobertura.lidos, 0);
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn as_pastas_de_dependencia_ficam_fora_da_varredura() {
        let pasta = pasta_de_teste("barulho");
        escrever(&pasta, "app.py", "os.system('ls')\n");
        escrever(&pasta, "node_modules/pacote/app.py", "os.system('ls')\n");
        escrever(&pasta, ".git/hooks/app.py", "os.system('ls')\n");
        let (achados, cobertura) = varredura_embutida(&pasta, &AtomicBool::new(false));
        assert_eq!(
            cobertura.lidos,
            1,
            "{:?}",
            achados.iter().map(|a| &a.file).collect::<Vec<_>>()
        );
        assert!(achados.iter().all(|a| a.file == "app.py"));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn o_corte_diz_quando_barra() {
        let contagem = AnalysisCounts {
            error: 0,
            warning: 2,
            info: 5,
        };
        assert!(!barra_o_corte("", &contagem));
        assert!(!barra_o_corte("ERROR", &contagem));
        assert!(barra_o_corte("WARNING", &contagem));
        assert!(barra_o_corte("INFO", &contagem));
        assert!(barra_o_corte(
            "ERROR",
            &AnalysisCounts {
                error: 1,
                warning: 0,
                info: 0
            }
        ));
    }

    #[test]
    fn o_mesmo_achado_dos_dois_instrumentos_e_gravado_uma_vez() {
        let mesmo = |fonte: &str| Achado {
            rule_id: "dd-105-avaliacao-de-codigo".to_string(),
            source: fonte.to_string(),
            severity: "ERROR".to_string(),
            file: "a.py".to_string(),
            line_start: 3,
            line_end: 3,
            message: "texto é executado como código".to_string(),
            cwe: None,
            snippet: None,
        };
        let juntos = dedupe(vec![mesmo("embutido"), mesmo("semgrep"), mesmo("embutido")]);
        assert_eq!(juntos.len(), 1);
        // Quem viu primeiro dá o nome da origem, e a ordem não depende de HashMap.
        assert_eq!(juntos[0].source, "embutido");
    }

    #[test]
    fn a_ordem_do_pacote_e_deterministica_e_pesa_o_que_barra_primeiro() {
        let feito = |id: &str, sev: &str, arquivo: &str, linha: i64| Achado {
            rule_id: id.to_string(),
            source: "embutido".to_string(),
            severity: sev.to_string(),
            file: arquivo.to_string(),
            line_start: linha,
            line_end: linha,
            message: String::new(),
            cwe: None,
            snippet: None,
        };
        let ordem: Vec<(String, i64)> = dedupe(vec![
            feito("dd-111-debug-ativo", "INFO", "z.py", 1),
            feito("dd-105-avaliacao-de-codigo", "ERROR", "b.py", 9),
            feito("dd-105-avaliacao-de-codigo", "ERROR", "b.py", 2),
            feito("dd-101-shell-true", "ERROR", "a.py", 4),
        ])
        .into_iter()
        .map(|a| (a.file, a.line_start))
        .collect();
        assert_eq!(
            ordem,
            vec![
                ("a.py".to_string(), 4),
                ("b.py".to_string(), 2),
                ("b.py".to_string(), 9),
                ("z.py".to_string(), 1)
            ]
        );
    }

    /// O envelope real de uma varredura, com um erro de interpretação e código 2.
    fn envelope_real() -> Value {
        json!({
            "version": "1.181.0",
            "results": [{
                "check_id": "regras.dd-105-avaliacao-de-codigo",
                "path": "a.py",
                "start": { "line": 3, "col": 1, "offset": 20 },
                "end": { "line": 3, "col": 9, "offset": 28 },
                "extra": {
                    "message": "texto é executado como código",
                    "metadata": { "cwe": ["CWE-94: Code Injection"] },
                    "severity": "ERROR",
                    "fingerprint": "requires login",
                    "lines": "requires login",
                    "validation_state": "NO_VALIDATOR",
                    "engine_kind": "OSS"
                }
            }],
            "errors": [{ "message": "Bad message syntax" }],
            "paths": { "scanned": ["a.py", "b.py", "c.py"], "skipped": ["grande.bin"] }
        })
    }

    fn chamada_de(code: Option<i32>) -> Chamada {
        Chamada {
            stdout: String::new(),
            stderr: String::new(),
            code,
            sem_resposta: false,
            cancelado: false,
        }
    }

    #[test]
    fn o_codigo_de_saida_da_ferramenta_nunca_e_o_veredito() {
        // Medido: varredura completa com um arquivo mal interpretado volta código 2
        // e JSON inteiro; outra, sem achados, volta código 0. As duas têm de virar
        // o mesmo tipo de conclusão — o que pesa são os dados.
        let pasta = pasta_de_teste("envelope");
        escrever(&pasta, "a.py", "import os\n\nos.system(cmd)\n");
        let resposta = ler_envelope(
            &envelope_real(),
            &pasta,
            chamada_de(Some(2)),
            Path::new("regras.yaml"),
        );
        assert_eq!(resposta.achados.len(), 1);
        assert_eq!(resposta.achados[0].rule_id, "dd-105-avaliacao-de-codigo");
        assert_eq!(resposta.achados[0].severity, "ERROR");
        assert_eq!(
            resposta.achados[0].cwe.as_deref(),
            Some("CWE-94: Code Injection")
        );
        assert_eq!(resposta.lidos, 3);
        assert_eq!(resposta.pulados, 1);
        assert_eq!(resposta.erros, 1);
        assert_eq!(resposta.code, Some(2));
        assert!(!resposta.sem_resposta && !resposta.invalida);
        let nota = resposta.nota.clone().unwrap_or_default();
        assert!(nota.contains("1 erro(s)"), "{nota}");
        // O trecho veio do arquivo lido por nós, não do JSON — que em modo livre
        // devolve "requires login" no lugar da linha.
        assert_eq!(
            resposta.achados[0].snippet.as_deref(),
            Some("os.system(cmd)")
        );
        let _ = std::fs::remove_dir_all(&pasta);
    }

    /// Envelope gravado de uma varredura real do semgrep 1.181.0 nesta máquina, em
    /// 2026-10-21: um arquivo rastreado pelo Git, duas linhas de comando, `errors`
    /// vazio e **sem** `paths.skipped` — porque nada foi pulado. É a forma que o
    /// leitor tem de continuar aceitando: um campo obrigatório que a ferramenta não
    /// escreve tornaria toda varredura limpa inválida.
    fn envelope_medido() -> Value {
        let achado = |linha: i64, ate: i64, offset: i64, offset_ate: i64| {
            json!({
                "check_id": "E.ssh-master.src-tauri.regras.dd-101-shell-true",
                "path": "a.py",
                "start": { "line": linha, "col": 5, "offset": offset },
                "end": { "line": ate, "col": 19, "offset": offset_ate },
                "extra": {
                    "message": "um comando é montado e entregue a um shell",
                    "metadata": { "cwe": "CWE-78: OS Command Injection" },
                    "severity": "ERROR",
                    "fingerprint": "requires login",
                    "lines": "requires login",
                    "validation_state": "NO_VALIDATOR",
                    "engine_kind": "OSS"
                }
            })
        };
        json!({
            "version": "1.181.0",
            "results": [achado(5, 5, 45, 59), achado(6, 6, 64, 103)],
            "errors": [],
            "paths": { "scanned": ["a.py"] },
            "time": { "rules": [], "targets": [], "total_bytes": 0 },
            "engine_requested": "OSS",
            "skipped_rules": [],
            "profiling_results": []
        })
    }

    #[test]
    fn o_envelope_real_medido_e_lido_com_prova() {
        let casa = pasta_de_teste("envelope-medido");
        escrever(
            &casa,
            "a.py",
            "import os\nimport subprocess\n\ndef f(cmd):\n    os.system(cmd)\n    \
             subprocess.run(\"ls \" + cmd, shell=True)\n",
        );
        let resposta = ler_envelope(
            &envelope_medido(),
            &casa,
            chamada_de(Some(0)),
            &caminho_das_regras(),
        );
        assert!(
            !resposta.invalida && !resposta.sem_resposta,
            "{:?}",
            resposta.nota
        );
        assert_eq!(resposta.lidos, 1);
        assert_eq!(
            resposta.pulados, 0,
            "sem `skipped` no JSON não há arquivo pulado"
        );
        assert_eq!(resposta.erros, 0);
        assert_eq!(resposta.nota, None, "varredura lida não tem o que reclamar");
        assert_eq!(resposta.code, Some(0));
        // o `check_id` é o caminho da config pontuado: o nome que os dois
        // instrumentos compartilham é o da regra
        assert_eq!(resposta.achados[0].rule_id, "dd-101-shell-true");
        assert_eq!(resposta.achados.len(), 2);
        // `cwe` veio como texto, não como lista
        assert_eq!(
            resposta.achados[0].cwe.as_deref(),
            Some("CWE-78: OS Command Injection")
        );
        // o trecho é a linha do arquivo, com a indentação que ela tem
        assert_eq!(
            resposta.achados[0].snippet.as_deref(),
            Some("    os.system(cmd)")
        );
        assert_eq!(resposta.regras, vec!["dd-101-shell-true".to_string()]);
        let _ = std::fs::remove_dir_all(&casa);
    }

    /// O arquivo de regras embarcado, pelo caminho real do crate. É o que a
    /// varredura materializa, e o que os testes de leitura conferem.
    fn caminho_das_regras() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("regras")
            .join(NOME_DAS_REGRAS)
    }

    #[test]
    fn um_envelope_sem_achados_declara_as_regras_do_arquivo() {
        let pasta = pasta_de_teste("vazio");
        escrever(&pasta, "a.py", "x = 1\n");
        let valor = json!({ "results": [], "errors": [], "paths": { "scanned": ["a.py"] } });
        let resposta = ler_envelope(&valor, &pasta, chamada_de(Some(0)), &caminho_das_regras());
        assert!(resposta.achados.is_empty());
        assert_eq!(resposta.lidos, 1);
        assert_eq!(resposta.regras.len(), 19, "{:?}", resposta.regras.len());
        assert!(resposta.nota.is_none());
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn um_envelope_sem_achados_com_semgrep_puro_declara_p_default() {
        let pasta = pasta_de_teste("vazio-p-default");
        escrever(&pasta, "a.py", "x = 1\n");
        let valor = json!({ "results": [], "errors": [], "paths": { "scanned": ["a.py"] } });
        // Caminho vazio: o Semgrep usa apenas p/default, sem arquivo local.
        let resposta = ler_envelope(&valor, &pasta, chamada_de(Some(0)), Path::new(""));
        assert!(resposta.achados.is_empty());
        assert_eq!(resposta.regras, vec!["p/default".to_string()]);
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn um_resultado_fora_de_utf8_e_lido_do_mesmo_modo() {
        let pasta = pasta_de_teste("ler-resultado");
        let caminho = pasta.join("saida.json");
        // "café" em latin-1: o 0xE9 sozinho não forma UTF-8, e o achado ecoa a
        // linha do arquivo varrido, não uma cópia limpa.
        std::fs::write(&caminho, b"{\"results\": [{\"linha\": \"caf\xE9\"}]}").expect("escrever");
        let lido = ler_resultado(&caminho).expect("o byte solto não descarta o resultado");
        assert!(lido.contains("caf"));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn um_envelope_incompleto_nao_inventa_numero() {
        // "lista de achados vazia" sem as outras duas listas não é "varreu e não
        // achou nada": é um formato que não lemos. Os zeros que sobram não são
        // prova, e era com eles que a corrida anterior se concluía.
        let resposta = ler_envelope(
            &json!({ "results": [] }),
            Path::new("."),
            chamada_de(None),
            Path::new("../regras/dd-seguranca.yaml"),
        );
        assert!(
            resposta.invalida,
            "envelope sem erros nem caminhos lidos não conclui"
        );
        assert_eq!(resposta.lidos, 0);
        assert_eq!(resposta.pulados, 0);
        assert_eq!(resposta.erros, 0);
        let nota = resposta.nota.clone().unwrap_or_default();
        assert!(nota.contains("lista de erros"), "{nota}");

        // o outro lado: o campo de achados que não veio virava "nenhum achado"
        let sem_achados = ler_envelope(
            &json!({ "errors": [], "paths": { "scanned": ["a.py"] } }),
            Path::new("."),
            chamada_de(Some(0)),
            Path::new("../regras/dd-seguranca.yaml"),
        );
        assert!(sem_achados.invalida);
        assert!(sem_achados
            .nota
            .clone()
            .unwrap_or_default()
            .contains("lista de achados"));

        // um achado sem linha não desaparece da conta: é leitura que não se leu
        let sem_forma = ler_envelope(
            &json!({
                "results": [{ "check_id": "p.regra", "path": "a.py", "extra": { "message": "m" } }],
                "errors": [],
                "paths": { "scanned": ["a.py"] }
            }),
            Path::new("."),
            chamada_de(Some(0)),
            Path::new("../regras/dd-seguranca.yaml"),
        );
        assert!(sem_forma.achados.is_empty());
        assert!(sem_forma.invalida);
        assert!(sem_forma
            .nota
            .unwrap_or_default()
            .contains("sem regra, caminho ou linha"));
    }

    #[test]
    fn a_cauda_corta_em_caractere_e_esconde_o_valor_que_passa_por_ela() {
        // O caso medido: 201 bytes com o acento exatamente na borda do corte. A
        // régua antiga contava bytes e o slice caía no meio do `é` — pânico no meio
        // da varredura, e a varredura é o que dava o veredito.
        let na_borda = format!("é{}", "a".repeat(199));
        assert_eq!(na_borda.len(), 201, "o caso é de bytes, não de caracteres");
        assert_eq!(na_borda.chars().count(), 200);
        assert_eq!(
            cauda(&na_borda),
            na_borda,
            "200 caracteres estão dentro do limite"
        );

        let bruto = format!("{}{}é", "x".repeat(100), "y".repeat(199));
        let corte = cauda(&bruto);
        assert_eq!(corte.chars().count(), 201, "a cauda cabe em 200 caracteres");
        assert!(corte.starts_with('…') && corte.ends_with('é'), "{corte}");

        // um stderr que citou a linha de comando com o token não é gravado como veio
        let com_segredo =
            cauda("falhou ao ler C:\\x\\config com password = \"FICTICIO_VALOR_SOLTO_123\"");
        assert!(com_segredo.contains("password = "), "{com_segredo}");
        assert!(
            !com_segredo.contains("FICTICIO_VALOR_SOLTO_123"),
            "{com_segredo}"
        );
    }

    #[test]
    fn uma_severidade_desconhecida_nao_desaparece_nem_vira_erro() {
        let valor = json!({
            "results": [{
                "check_id": "p.regra-alheia",
                "path": "a.py",
                "start": { "line": 1 },
                "extra": { "message": "x", "severity": "CRITICAL" }
            }],
            "errors": [],
            "paths": { "scanned": ["a.py"] }
        });
        let resposta = ler_envelope(
            &valor,
            Path::new("."),
            chamada_de(Some(0)),
            Path::new("r.yaml"),
        );
        assert_eq!(resposta.achados[0].severity, "WARNING");
    }

    #[test]
    fn o_trecho_gravado_nunca_tem_o_valor_de_um_segredo() {
        let valor = "FICTICIO_TOKEN_OPACO_123456789";
        let chave = "FICTICIO_CHAVE_DE_API_123456789";
        let pasta = pasta_de_teste("mascara");
        // O segredo está na linha que a regra aponta (embutido) e dentro da janela
        // de três linhas do achado (semgrep): é o trecho que ia para o banco.
        escrever(
            &pasta,
            "a.py",
            &format!(
                "import os\nos.system(\"curl -H 'Authorization: Bearer {valor}'\")\napi_key = \"{chave}\"\n"
            ),
        );

        let (achados, _) = varredura_embutida(&pasta, &AtomicBool::new(false));
        let apontados: Vec<&Achado> = achados
            .iter()
            .filter(|a| a.snippet.as_deref().unwrap_or("").contains("Bearer"))
            .collect();
        assert!(
            !apontados.is_empty(),
            "a varredura não achou a linha do comando"
        );
        for a in &apontados {
            let texto = a.snippet.clone().unwrap_or_default();
            assert!(
                !texto.contains(valor),
                "o valor do cabeçalho vazou: {texto}"
            );
            assert!(texto.contains("Bearer «valor mascarado»"), "{texto}");
        }

        // O mesmo arquivo pelo caminho do semgrep: o trecho é lido por nós, linha a
        // linha, e a linha do segredo pode estar dentro da janela sem ser a linha
        // apontada.
        let valor_json = json!({
            "results": [{
                "check_id": "regras.dd-101-shell-true",
                "path": "a.py",
                "start": { "line": 2 },
                "end": { "line": 3 },
                "extra": { "message": "m", "severity": "ERROR" }
            }],
            "errors": [],
            "paths": { "scanned": ["a.py"] }
        });
        let resposta = ler_envelope(
            &valor_json,
            &pasta,
            chamada_de(Some(0)),
            Path::new("r.yaml"),
        );
        let texto = resposta.achados[0].snippet.clone().unwrap_or_default();
        assert!(!texto.contains(valor) && !texto.contains(chave), "{texto}");
        assert!(texto.contains("«valor mascarado»"), "{texto}");
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn o_trecho_do_achado_e_a_linha_apontada() {
        let pasta = pasta_de_teste("trecho");
        escrever(&pasta, "a.py", "x = 1\ny = 2\neval(cmd)\nz = 3\n");
        let valor = json!({
            "results": [{
                "check_id": "regras.dd-105-avaliacao-de-codigo",
                "path": "a.py",
                "start": { "line": 3 },
                "end": { "line": 3 },
                "extra": { "message": "m", "severity": "ERROR" }
            }],
            "errors": [],
            "paths": { "scanned": ["a.py"] }
        });
        let resposta = ler_envelope(&valor, &pasta, chamada_de(Some(0)), Path::new("r.yaml"));
        assert_eq!(resposta.achados[0].snippet.as_deref(), Some("eval(cmd)"));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn a_versao_e_lida_do_que_a_ferramenta_escreve() {
        assert_eq!(
            versao_da_resposta("1.181.0", "").as_deref(),
            Some("1.181.0")
        );
        assert_eq!(
            versao_da_resposta("semgrep 1.181.0\n", "").as_deref(),
            Some("1.181.0")
        );
        assert_eq!(
            versao_da_resposta("", "Traceback: nada de versão aqui").as_deref(),
            None
        );
    }

    #[test]
    fn as_regras_materializadas_sao_as_do_aplicativo() {
        let dados = pasta_de_teste("regras");
        let arquivo = publicar_regras(&dados).unwrap();
        assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), REGRAS_YAML);
        // Uma regra antiga esquecida no disco é reescrita, não respeitada.
        escrever(
            &dados,
            "regras/dd-seguranca.yaml",
            "rules:\n  - id: antiga\n",
        );
        publicar_regras(&dados).unwrap();
        assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), REGRAS_YAML);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn o_estado_recusa_duas_varreduras_do_mesmo_projeto() {
        let estado = Estado::novo();
        let primeira = estado.iniciar(7).unwrap();
        assert!(estado.iniciar(7).is_err());
        assert!(estado.iniciar(8).is_ok());
        assert_eq!(estado.em_andamento(), vec![7, 8]);
        assert!(estado.interromper(7));
        assert!(primeira.load(Ordering::SeqCst));
        assert!(!estado.interromper(99));
        estado.terminar(7);
        assert!(estado.iniciar(7).is_ok());
    }

    fn corrida(campos: &str) -> AnalysisRun {
        let counts = match campos {
            "vazia" => AnalysisCounts::default(),
            "cheia" => AnalysisCounts {
                error: 2,
                warning: 0,
                info: 0,
            },
            _ => AnalysisCounts::default(),
        };
        AnalysisRun {
            id: 1,
            project_id: 1,
            project: "p".to_string(),
            root: "r".to_string(),
            analyzer: "embutido".to_string(),
            tool: "embutido".to_string(),
            tool_version: None,
            rules: "dd-101-shell-true".to_string(),
            severity_gate: "ERROR".to_string(),
            status: "concluida".to_string(),
            started_at: "a".to_string(),
            finished_at: Some("b".to_string()),
            duration_ms: 10,
            files_scanned: if campos == "sem-leitura" { 0 } else { 12 },
            files_skipped: 0,
            exit_code: None,
            tool_errors: if campos == "erros" { 3 } else { 0 },
            blocked: campos == "cheia",
            counts,
            note: None,
        }
    }

    #[test]
    fn a_classificacao_diz_o_que_os_numeros_dizem() {
        let base = corrida("vazia");
        assert_eq!(
            classificar(&base)["classificacao"].as_str(),
            Some("sem-achados")
        );
        assert_eq!(
            decisoes(&base, &classificar(&base))[0]["acao"].as_str(),
            Some("prosseguir")
        );
        assert_eq!(
            classificar(&corrida("sem-leitura"))["classificacao"].as_str(),
            Some("cobertura-nula")
        );
        assert_eq!(
            classificar(&corrida("cheia"))["classificacao"].as_str(),
            Some("barra")
        );
        assert_eq!(
            decisoes(&corrida("cheia"), &classificar(&corrida("cheia")))[0]["acao"].as_str(),
            Some("parar-a-publicacao")
        );
        let reclamou = corrida("erros");
        assert_eq!(
            classificar(&reclamou)["classificacao"].as_str(),
            Some("revalidar-varredura")
        );
        let acoes: Vec<String> = decisoes(&reclamou, &classificar(&reclamou))
            .iter()
            .map(|d| d["acao"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            acoes.contains(&"revalidar-varredura".to_string()),
            "{acoes:?}"
        );

        let mut interrompida = base.clone();
        interrompida.status = "interrompida".to_string();
        assert_eq!(
            classificar(&interrompida)["classificacao"].as_str(),
            Some("varredura-interrompida")
        );

        // A varredura que falhou não é "sem achados": os números que ela trouxe são
        // dela, e o resto continua sem prova.
        let falhou = AnalysisRun {
            status: "falhou".to_string(),
            ..base.clone()
        };
        assert_eq!(
            classificar(&falhou)["classificacao"].as_str(),
            Some("leitura-incompleta")
        );
        let acoes: Vec<String> = decisoes(&falhou, &classificar(&falhou))
            .iter()
            .map(|d| d["acao"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            acoes.contains(&"refazer-a-varredura".to_string()),
            "{acoes:?}"
        );
        assert!(!acoes.contains(&"prosseguir".to_string()), "{acoes:?}");

        let ferramenta_fora = AnalysisRun {
            tool: "semgrep ausente".to_string(),
            ..base
        };
        assert_eq!(
            classificar(&ferramenta_fora)["classificacao"].as_str(),
            Some("leitura-incompleta")
        );
        assert!(decisoes(&ferramenta_fora, &classificar(&ferramenta_fora))
            .iter()
            .any(|d| d["acao"].as_str() == Some("instalar-ou-apontar-a-ferramenta")));
    }

    #[test]
    fn o_que_barra_e_a_contagem_do_corte_nao_a_contagem_total() {
        // A corrida medida: 1.358 achados, 676 deles ERROR, corte ERROR. Dizer
        // "barram 1.358" inventaria 682 avisos que ninguém pediu para barrar.
        let base = corrida("cheia");
        let mut conta = base.clone();
        conta.counts = AnalysisCounts {
            error: 2,
            warning: 7,
            info: 3,
        };
        let veredito = classificar(&conta);
        assert_eq!(veredito["contagem"]["noCorte"].as_i64(), Some(2));
        assert_eq!(veredito["contagem"]["total"].as_i64(), Some(12));
        let decisao = &decisoes(&conta, &veredito)[0];
        assert_eq!(decisao["acao"].as_str(), Some("parar-a-publicacao"));
        assert_eq!(
            decisao["motivo"].as_str(),
            Some("2 achado(s) no corte ERROR ou acima"),
            "{decisao}"
        );
        assert_eq!(
            decisao["evidencia"].as_str(),
            Some("veredito.contagem.noCorte")
        );

        // aviso acima de um corte que só barra erro continua barrando por erro
        let so_aviso = AnalysisRun {
            counts: AnalysisCounts {
                error: 0,
                warning: 4,
                info: 0,
            },
            blocked: false,
            ..base
        };
        assert_eq!(
            classificar(&so_aviso)["classificacao"].as_str(),
            Some("achados-abaixo-do-corte")
        );
    }

    #[test]
    fn o_grupo_por_regra_soma_o_que_os_achados_sao() {
        let feito = |regra: &str, arquivo: &str| AnalysisFinding {
            id: 0,
            run_id: 1,
            rule_id: regra.to_string(),
            source: "embutido".to_string(),
            severity: "ERROR".to_string(),
            file: arquivo.to_string(),
            line_start: 1,
            line_end: 1,
            message: "m".to_string(),
            cwe: None,
            snippet: None,
            fingerprint: String::new(),
        };
        let grupo = agrupar_por_regra(&[
            feito("dd-101-shell-true", "a.py"),
            feito("dd-101-shell-true", "b.py"),
            feito("dd-105-avaliacao-de-codigo", "a.py"),
        ]);
        assert_eq!(grupo[0]["regra"].as_str(), Some("dd-101-shell-true"));
        assert_eq!(grupo[0]["quantidade"].as_i64(), Some(2));
        assert_eq!(grupo[0]["arquivosDiferentes"].as_i64(), Some(2));
        assert_eq!(grupo[1]["quantidade"].as_i64(), Some(1));
    }

    #[test]
    fn o_sarif_tem_a_forma_que_a_especificacao_espera() {
        let achados = vec![AnalysisFinding {
            id: 1,
            run_id: 9,
            rule_id: "dd-101-shell-true".to_string(),
            source: "embutido".to_string(),
            severity: "ERROR".to_string(),
            file: "src\\a.py".to_string(),
            line_start: 4,
            line_end: 4,
            message: "m".to_string(),
            cwe: Some("CWE-78".to_string()),
            snippet: Some("os.system(x)".to_string()),
            fingerprint: String::new(),
        }];
        let grupo = agrupar_por_regra(&achados);
        let resultados: Vec<Value> = achados
            .iter()
            .map(|a| {
                json!({
                    "ruleId": a.rule_id,
                    "level": nivel_da_severidade(&a.severity),
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": a.file.replace('\\', "/") },
                            "region": { "startLine": a.line_start },
                        }
                    }],
                })
            })
            .collect();
        assert_eq!(grupo[0]["severidade"].as_str(), Some("ERROR"));
        assert_eq!(resultados[0]["level"].as_str(), Some("error"));
        assert_eq!(
            resultados[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str(),
            Some("src/a.py")
        );
        assert_eq!(nivel_da_severidade("WARNING"), "warning");
        assert_eq!(nivel_da_severidade("INFO"), "note");
    }

    #[test]
    fn o_caminho_explicito_soe_existe_quando_existe() {
        let aqui = std::env::current_exe().unwrap();
        let nome = aqui.to_str().unwrap();
        assert_eq!(resolver_explicito_com(nome, no_path).as_deref(), Some(nome));
        assert!(resolver_explicito_com("C:\\não\\há\\nada\\aqui.exe", no_path).is_none());
        assert!(resolver_explicito_com("", no_path).is_none());
    }

    #[test]
    fn uma_ferramenta_ausente_e_dita_ausente_e_nao_sem_resposta() {
        // O nome não existe: `ausente`. É a diferença entre "instale" e "espere".
        // Os buscadores vazios isolam a resposta do que está instalado na máquina.
        let ferramenta = localizar_com(
            Some("um-programa-que-nao-existe-em-lugar-nenhum"),
            |_| None,
            |_| None,
        );
        assert_eq!(ferramenta.estado, "ausente");
        assert!(ferramenta.caminho.is_none());
        assert_eq!(ferramenta.regras, 19);
        assert_eq!(
            ferramenta.apontado,
            "um-programa-que-nao-existe-em-lugar-nenhum"
        );
    }

    #[test]
    fn a_busca_automatica_vale_quando_o_caminho_apontado_nao_serve() {
        // O caso real: a tela guardou uma pasta, e o semgrep está nos scripts do
        // Python. O apontado não pode esconder a ferramenta que existe.
        let (binario, origem) = procurar_binario(
            Some(r"E:\projeto\dist\"),
            |_| None,
            |nome| {
                (nome == "pysemgrep.exe").then(|| r"C:\Python\Scripts\pysemgrep.exe".to_string())
            },
        );
        assert_eq!(binario.as_deref(), Some(r"C:\Python\Scripts\pysemgrep.exe"));
        assert_eq!(origem, "python-scripts");

        // Sem candidato em lugar nenhum, a resposta continua sendo ausente e sem
        // origem inventada; o valor apontado é o que a tela mostra.
        let (nenhum, vazia) = procurar_binario(Some(r"E:\projeto\dist\"), |_| None, |_| None);
        assert!(nenhum.is_none());
        assert!(vazia.is_empty());
    }

    #[test]
    fn a_raiz_do_projeto_existe_ou_diz_que_nao() {
        let projeto = Project {
            id: 1,
            name: "n".to_string(),
            local_path: "uma-pasta-que-nao-existe-aqui".to_string(),
            is_git_repo: false,
            git_branch: None,
            git_remote: None,
            default_server_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(raiz_do_projeto(&projeto).is_err());
    }

    // ---------- a varredura inteira, pela porta da tela ----------

    fn banco() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn
    }

    fn projeto_de_teste(conn: &Connection, pasta: &Path) -> Project {
        db::insert_project(
            conn,
            "casa",
            &pasta.display().to_string(),
            false,
            None,
            None,
        )
        .unwrap()
    }

    fn pedido(project_id: i64, instrumento: &str, corte: &str) -> AnalysisInput {
        AnalysisInput {
            project_id,
            analyzer: Some(instrumento.to_string()),
            severity_gate: Some(corte.to_string()),
            path: None,
            timeout_secs: Some(0),
        }
    }

    #[test]
    fn uma_varredura_de_verdade_grava_a_corrida_e_os_achados() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", "ERROR"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(corrida.status, "concluida");
        assert_eq!(corrida.analyzer, "embutido");
        assert_eq!(
            corrida.project, "casa",
            "a corrida diz de quem é sem segunda consulta"
        );
        assert!(
            corrida.blocked,
            "o código plantado tem erro e o corte é ERROR"
        );
        assert!(corrida.counts.error > 0);
        assert!(!corrida.rules.is_empty());
        assert!(
            corrida.tool_version.is_none(),
            "o embutido não tem versão de ferramenta"
        );
        // a contagem dita pelo veredito é a contagem da tabela, não a promessa de
        // quem escreveu os achados
        let lidos = db::listar_achados(&conn, corrida.id, Some("ERROR"), 500).unwrap();
        assert_eq!(lidos.len() as i64, corrida.counts.error);
        assert!(lidos.iter().all(|a| a.source == "embutido"));
        assert_eq!(
            db::contar_achados(&conn, corrida.id).unwrap().total(),
            corrida.counts.total()
        );
        let de_volta = db::obter_analise(&conn, corrida.id).unwrap();
        assert_eq!(de_volta.counts, corrida.counts);
        assert_eq!(de_volta.root, corrida.root);
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn a_raiz_gravada_e_a_mesma_rgua_da_contencao_sem_o_prefixo_verbatim() {
        let casa = pasta_de_teste("raiz-gravada");
        escrever(
            &casa,
            "bom.py",
            "import subprocess\ndef f(cmd):\n    subprocess.run(cmd, check=True)\n",
        );
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-raiz-gravada");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", "ERROR"),
            &AtomicBool::new(false),
        )
        .unwrap();
        // No Windows `canonicalize()` devolve `\\?\C:\...`, que nenhum outro programa
        // abre; a contenção mede nesse caminho, a tela e o pacote mostram o outro.
        assert!(!corrida.root.starts_with(r"\\?\"), "{}", corrida.root);
        assert_eq!(corrida.root, raiz.display().to_string());
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn uma_casa_limpa_passa_e_diz_que_passou() {
        let casa = pasta_de_teste("limpa");
        escrever(
            &casa,
            "bom.py",
            "import subprocess\ndef f(nome):\n    subprocess.run([\"ls\", nome], check=True)\n\
             \x20   return hashlib.sha256(nome.encode()).hexdigest()\n",
        );
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-limpa");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", "ERROR"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(corrida.status, "concluida");
        assert!(!corrida.blocked);
        assert_eq!(corrida.counts.total(), 0);
        assert_eq!(corrida.files_scanned, 1);
        let pacote = self::pacote(&conn, corrida.id, None).unwrap();
        assert_eq!(pacote["veredito"]["classificacao"], "sem-achados");
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    /// Aponta um caminho de ferramenta que não existe, para a corrida falar do
    /// instrumento externo sem depender do que está instalado nesta máquina.
    fn ferramenta_apontada(conn: &Connection, caminho: &str) {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![CHAVE_CONFIG, caminho, db::now_rfc3339()],
        )
        .unwrap();
    }

    /// A ausência fixada em teste: nem PATH nem scripts do Python desta máquina
    /// entram na resposta.
    fn localizador_ausente(_: Option<&str>) -> Ferramenta {
        Ferramenta {
            estado: "ausente".to_string(),
            caminho: None,
            versao: None,
            duracao_ms: 0,
            regras: contar_regras(REGRAS_YAML),
            origem: String::new(),
            apontado: String::new(),
            resposta: String::new(),
        }
    }

    #[test]
    fn o_instrumento_pedido_que_nao_rodou_nao_conclui_a_corrida() {
        let casa = arvore_de_teste();
        let conn = banco();
        ferramenta_apontada(&conn, r"C:\nao\há\semgrep.exe");
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-ausente");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar_com(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "ambos", "ERROR"),
            &AtomicBool::new(false),
            localizador_ausente,
        )
        .unwrap();
        // Medido: a corrida #17 foi pedida como `ambos`, o semgrep não estava no
        // PATH e ela voltou `concluida` com a contagem do embutido — o nó leu
        // "sem nada acima do corte" e publicou. Um instrumento que não rodou não é
        // um instrumento que não achou nada.
        assert_eq!(corrida.status, "falhou", "{:?}", corrida.note);
        assert!(corrida.tool.contains("semgrep ausente"), "{}", corrida.tool);
        assert!(
            corrida.blocked,
            "corte declarado sem a leitura prometida barra"
        );
        let nota = corrida.note.clone().unwrap_or_default();
        assert!(nota.contains("instale-o ou aponte o caminho"), "{nota}");
        // o que foi lido continua gravado: é a nota que diz o que falta, não o
        // sumiço do que existe
        assert!(corrida.counts.error > 0);
        assert_eq!(
            db::listar_achados(&conn, corrida.id, Some("ERROR"), 500)
                .unwrap()
                .len() as i64,
            corrida.counts.error
        );
        let pacote = self::pacote(&conn, corrida.id, None).unwrap();
        assert_eq!(pacote["veredito"]["classificacao"], "leitura-incompleta");
        let acoes: Vec<String> = pacote["decisoes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["acao"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(
            acoes.contains(&"parar-a-publicacao".to_string()),
            "{acoes:?}"
        );
        assert!(
            acoes.contains(&"instalar-ou-apontar-a-ferramenta".to_string()),
            "{acoes:?}"
        );
        assert!(!acoes.contains(&"prosseguir".to_string()), "{acoes:?}");

        // sem corte declarado não há promessa de leitura a descumprir: a corrida é
        // dita falha pelo que aconteceu, mas nada barra por contagem
        let sem_corte = executar_com(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "ambos", ""),
            &AtomicBool::new(false),
            localizador_ausente,
        )
        .unwrap();
        assert_eq!(sem_corte.status, "falhou");
        assert!(!sem_corte.blocked, "nada foi pedido para barrar");
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn uma_casa_onde_nada_foi_lido_nao_e_uma_casa_limpa() {
        let casa = pasta_de_teste("nada-lido");
        // binário: o arquivo existe, a varredura passa por ele e não abre nada.
        std::fs::write(casa.join("dados.bin"), [0u8; 512]).unwrap();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-nada-lido");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", "ERROR"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(corrida.files_scanned, 0);
        assert_eq!(corrida.status, "falhou", "{:?}", corrida.note);
        assert!(
            corrida.blocked,
            "zero leitura com corte ERROR não é zero achados"
        );
        let nota = corrida.note.clone().unwrap_or_default();
        assert!(nota.contains("não leu arquivo nenhum"), "{nota}");
        let pacote = self::pacote(&conn, corrida.id, None).unwrap();
        assert_eq!(pacote["veredito"]["classificacao"], "cobertura-nula");
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn a_varredura_nao_sai_da_pasta_do_projeto() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-fora");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let mut dobrando = pedido(projeto.id, "embutido", "");
        dobrando.path = Some("../de-lado".to_string());
        let err = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &dobrando,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(err.contains("caminho relativo"), "{err}");
        // uma pasta que não existe no projeto também não varre nada
        let mut inexistente = pedido(projeto.id, "embutido", "");
        inexistente.path = Some("nao-existe".to_string());
        assert!(executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &inexistente,
            &AtomicBool::new(false)
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn a_varredura_para_na_porta_quando_a_pasta_nao_acorda() {
        let casa = pasta_de_teste("porta");
        std::fs::create_dir_all(casa.join("src")).expect("criar a subpasta");

        // Pasta que está lá: a régua canônica responde, e vem sem o prefixo verbatim.
        let alvo = alvo_da_varredura(&casa, &casa.join("src"), git_probe::PRAZO_SONDA)
            .expect("a pasta respondeu");
        assert!(!alvo.to_string_lossy().starts_with(r"\\?\"), "{alvo:?}");

        // Prazo menor que qualquer resposta de disco: o que a porta devolve é o
        // motivo da espera abandonada — e nunca a frase de "não existe", que
        // culparia o pedido quando quem não acordou foi o volume.
        let sem_resposta = alvo_da_varredura(&casa, &casa.join("src"), Duration::ZERO)
            .expect_err("0 ms é prazo, não resposta");
        assert!(
            sem_resposta.contains("não respondeu em 0 ms"),
            "{sem_resposta}"
        );
        assert!(!sem_resposta.contains("não existe"), "{sem_resposta}");

        // Subpasta que não está lá, com prazo de verdade: essa sim é ausência dita.
        let falta = alvo_da_varredura(&casa, &casa.join("nao-existe"), git_probe::PRAZO_SONDA)
            .expect_err("a subpasta não existe");
        assert!(falta.contains("não existe dentro do projeto"), "{falta}");
        let _ = std::fs::remove_dir_all(&casa);
    }

    #[test]
    fn exportar_escreve_o_mesmo_texto_que_a_tela_leu() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("exporta");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", "WARNING"),
            &AtomicBool::new(false),
        )
        .unwrap();
        let arquivo = dados.join("analise-exportada.json");
        let caminho = exportar(&conn, &arquivo, corrida.id, "pacote", None).unwrap();
        assert!(caminho.ends_with("analise-exportada.json"), "{caminho}");
        assert_eq!(Path::new(&caminho), arquivo.as_path());
        // o lugar que o aplicativo decidia antes não é mais escrito por ninguém
        assert!(!dados.join("analises").exists());
        let valor: Value =
            serde_json::from_str(&std::fs::read_to_string(&caminho).unwrap()).unwrap();
        assert_eq!(valor["esquema"], "devops-desktop/analise@1");
        assert_eq!(valor["execucao"]["id"], corrida.id);
        assert_eq!(valor["execucao"]["codigoDeSaidaEProva"], false);
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn o_sarif_exportado_sai_das_linhas_gravadas() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("exporta-sarif");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", ""),
            &AtomicBool::new(false),
        )
        .unwrap();
        // o nome escolhido é o nome escrito, e um sarif também aceita .json
        let arquivo = dados.join("varredura.sarif");
        let caminho = exportar(&conn, &arquivo, corrida.id, "sarif", None).unwrap();
        assert!(caminho.ends_with("varredura.sarif"), "{caminho}");
        let valor: Value =
            serde_json::from_str(&std::fs::read_to_string(&caminho).unwrap()).unwrap();
        assert_eq!(valor["version"], "2.1.0");
        assert_eq!(
            valor["runs"][0]["results"].as_array().unwrap().len(),
            corrida.counts.total() as usize
        );
        // um formato que o aplicativo não escreve não escreve arquivo nenhum
        assert!(exportar(&conn, &dados, corrida.id, "html", None).is_err());
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
    }

    #[test]
    fn um_caminho_que_nao_serve_nao_escreve_arquivo_nenhum() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("exporta-recusa");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let corrida = executar(
            &conn,
            &dados,
            &projeto,
            &raiz,
            &pedido(projeto.id, "embutido", ""),
            &AtomicBool::new(false),
        )
        .unwrap();
        // relativo: ninguém sabe em qual pasta cairia
        let relativo = PathBuf::from("analise.json");
        let erro = exportar(&conn, &relativo, corrida.id, "pacote", None).unwrap_err();
        assert!(erro.contains("caminho completo"), "{erro}");
        assert!(!relativo.exists());
        // a extensão do formato ao lado do nome escolhido
        let errado = dados.join("analise.html");
        let erro = exportar(&conn, &errado, corrida.id, "pacote", None).unwrap_err();
        assert!(erro.contains("termina em .json"), "{erro}");
        assert!(!errado.exists());
        // o nome é do pacote; o do sarif tem as duas faces
        let sarif_json = dados.join("para-a-ci.json");
        assert!(exportar(&conn, &sarif_json, corrida.id, "sarif", None).is_ok());
        // um lugar que não existe devolve o motivo do disco, não um caminho bonito
        const INEXISTENTE: &str = "dd-exporta-sem-pasta-9z4k";
        let falta = std::env::temp_dir().join(INEXISTENTE).join("analise.json");
        let erro = exportar(&conn, &falta, corrida.id, "pacote", None).unwrap_err();
        assert!(erro.contains("não foi possível gravar"), "{erro}");
        assert!(erro.len() > "não foi possível gravar ".len(), "{erro}");
        let _ = std::fs::remove_dir_all(&dados);
        let _ = std::fs::remove_dir_all(&casa);
    }

    #[test]
    fn a_lista_da_tela_e_a_tabela_que_varre() {
        let lista = regras_embarcadas();
        let ids: Vec<&str> = lista.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ids_embarcados());
        assert_eq!(lista.len(), 19);
        assert!(lista.iter().all(|r| !r.mensagem.trim().is_empty()));
        assert!(lista
            .iter()
            .all(|r| SEVERIDADES.contains(&r.severidade.as_str())));
    }

    #[test]
    fn a_conferencia_diz_os_tres_estados_da_ferramenta() {
        let chamada = |stdout: &str, stderr: &str, code: Option<i32>, sem_resposta: bool| Chamada {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            code,
            sem_resposta,
            cancelado: false,
        };
        assert_eq!(
            estado_da_conferencia(&chamada("", "", Some(0), false)),
            "validadas"
        );
        assert_eq!(
            estado_da_conferencia(&chamada("", "configuration error", Some(7), false)),
            "inválidas"
        );
        // quem não respondeu não é "inválidas" — e muito menos "validadas"
        assert_eq!(
            estado_da_conferencia(&chamada("", "", None, true)),
            "sem-resposta"
        );
        assert_eq!(
            estado_da_conferencia(&chamada("", "", Some(0), true)),
            "sem-resposta"
        );
    }

    fn sonda(
        estado: &str,
        apontado: &str,
        caminho: Option<&str>,
        versao: Option<&str>,
    ) -> Ferramenta {
        Ferramenta {
            estado: estado.to_string(),
            caminho: caminho.map(str::to_string),
            versao: versao.map(str::to_string),
            duracao_ms: 2609,
            regras: 19,
            origem: if caminho.is_some() {
                "configuração".to_string()
            } else {
                String::new()
            },
            apontado: apontado.to_string(),
            resposta: "semgrep 1.181.0".to_string(),
        }
    }

    #[test]
    fn a_sonda_grava_o_valor_apontado_quando_a_ferramenta_nao_esta_aqui() {
        // O caso que alguém aponta em Configurações e quer conferir depois no
        // Histórico: `caminho` é nulo e `origem` fica vazio, então sem o valor
        // apontado a linha não diria o que foi tentado.
        let linha = linha_da_sonda(&sonda("ausente", r"C:\pasta\semgrep.exe", None, None));
        assert_eq!(linha.nivel, "warning");
        assert!(linha.operacao.contains("ausente"), "{}", linha.operacao);
        assert!(
            linha.detalhe.contains(r#"“C:\pasta\semgrep.exe”"#),
            "{}",
            linha.detalhe
        );
        assert!(
            linha.detalhe.contains("nenhum binário resolvido"),
            "{}",
            linha.detalhe
        );
        assert!(
            !linha.detalhe.contains("origem"),
            "sem binário não há origem a dizer: {}",
            linha.detalhe
        );
        assert!(linha.detalhe.contains("embutido"), "{}", linha.detalhe);

        let nada = linha_da_sonda(&sonda("ausente", "", None, None));
        assert!(nada.detalhe.contains("nada apontado"), "{}", nada.detalhe);

        // sem resposta não é ausente, e nunca é "encontrado"
        let muda = linha_da_sonda(&sonda(
            "sem-resposta",
            "semgrep",
            Some("/usr/bin/semgrep"),
            None,
        ));
        assert_eq!(muda.nivel, "error");
        assert!(muda.operacao.contains("sem resposta"), "{}", muda.operacao);
        assert!(
            muda.detalhe.contains("/usr/bin/semgrep"),
            "{}",
            muda.detalhe
        );

        let achada = linha_da_sonda(&sonda(
            "encontrado",
            "semgrep",
            Some("/usr/bin/semgrep"),
            Some("1.181.0"),
        ));
        assert_eq!(achada.nivel, "info");
        assert!(
            achada.operacao.contains("19 regra(s) embarcada(s)"),
            "{}",
            achada.operacao
        );
        assert!(
            achada.detalhe.contains("origem configuração"),
            "{}",
            achada.detalhe
        );
        assert!(achada.detalhe.contains("1.181.0"), "{}", achada.detalhe);
        assert!(achada.detalhe.contains("2609 ms"), "{}", achada.detalhe);

        // O apontado não serviu, mas a busca automática achou: a linha diz as duas
        // coisas, sem transformar o achado em "configurado".
        let mut alternativa = sonda(
            "encontrado",
            r"E:\projeto\dist\",
            Some(r"C:\Python\Scripts\pysemgrep.exe"),
            Some("1.182.1"),
        );
        alternativa.origem = "python-scripts".to_string();
        let fallback = linha_da_sonda(&alternativa);
        assert!(
            fallback.detalhe.contains("(não serve)"),
            "{}",
            fallback.detalhe
        );
        assert!(
            fallback.detalhe.contains("origem python-scripts"),
            "{}",
            fallback.detalhe
        );
        assert!(
            fallback.detalhe.contains("pysemgrep.exe"),
            "{}",
            fallback.detalhe
        );
    }

    #[test]
    fn a_conferencia_gravada_diz_o_estado_e_nunca_presume_ok() {
        let validadas = linha_da_conferencia(&Validacao {
            estado: "validadas".to_string(),
            regras: 19,
            codigo: Some(0),
            mensagem: "Configuration is valid".to_string(),
        });
        assert_eq!(validadas.nivel, "info");
        assert!(
            validadas.operacao.contains("validadas"),
            "{}",
            validadas.operacao
        );
        assert!(
            validadas.detalhe.contains("código 0"),
            "{}",
            validadas.detalhe
        );

        let sem_ferramenta = linha_da_conferencia(&Validacao {
            estado: "sem-ferramenta".to_string(),
            regras: 19,
            codigo: None,
            mensagem: "nenhuma conferência externa".to_string(),
        });
        assert_eq!(
            sem_ferramenta.nivel, "warning",
            "ausência de ferramenta não é verde"
        );
        assert!(
            sem_ferramenta.detalhe.contains("sem código de saída"),
            "{}",
            sem_ferramenta.detalhe
        );

        for estado in ["inválidas", "sem-resposta"] {
            let l = linha_da_conferencia(&Validacao {
                estado: estado.to_string(),
                regras: 19,
                codigo: Some(7),
                mensagem: String::new(),
            });
            assert_eq!(l.nivel, "error", "{estado}");
        }
    }

    // ---------- o SARIF e o pacote com identidade ----------

    fn achado_sarif(regra: &str, arquivo: &str, linha: i64, trecho: &str) -> AnalysisFinding {
        AnalysisFinding {
            id: 0,
            run_id: 0,
            rule_id: regra.to_string(),
            source: "embutido".to_string(),
            severity: if regra.starts_with("dd-101") {
                "ERROR"
            } else {
                "WARNING"
            }
            .to_string(),
            file: arquivo.to_string(),
            line_start: linha,
            line_end: linha,
            message: formato_da_regra(regra),
            cwe: regra
                .starts_with("dd-101")
                .then(|| "CWE-78: OS Command Injection".to_string()),
            snippet: Some(trecho.to_string()),
            fingerprint: String::new(),
        }
    }

    fn formato_da_regra(regra: &str) -> String {
        REGRAS_EMBUTIDAS
            .iter()
            .find(|r| r.id == regra)
            .map(|r| r.mensagem.to_string())
            .unwrap_or_else(|| format!("mensagem de {regra}"))
    }

    fn corrida_para_exportar(
        conn: &Connection,
        project_id: i64,
        inicio: &str,
        achados: &[AnalysisFinding],
    ) -> i64 {
        conn.execute(
            "INSERT INTO analysis_runs (project_id, analyzer, tool, tool_version, root, \
             severity_gate, status, started_at, finished_at, files_scanned, counts) \
             VALUES (?1, 'embutido', 'embutido', NULL, 'C:/projeto', 'ERROR', 'concluida', \
             ?2, ?2, 10, ?3)",
            rusqlite::params![
                project_id,
                inicio,
                serde_json::to_string(&AnalysisCounts {
                    error: achados.iter().filter(|a| a.severity == "ERROR").count() as i64,
                    warning: achados.iter().filter(|a| a.severity == "WARNING").count() as i64,
                    info: 0,
                })
                .unwrap()
            ],
        )
        .unwrap();
        let run_id = conn.last_insert_rowid();
        db::gravar_achados(conn, run_id, achados).unwrap();
        run_id
    }

    #[test]
    fn o_sarif_leva_indice_de_regra_impressao_estado_de_base_e_invocacao() {
        let conn = banco();
        let projeto = projeto_de_teste(&conn, Path::new("C:/projeto"));
        let primeira = corrida_para_exportar(
            &conn,
            projeto.id,
            "2026-01-01T10:00:00Z",
            &[achado_sarif(
                "dd-101-shell-true",
                "src/a.py",
                3,
                "os.system(x)",
            )],
        );
        let primeiro = sarif(&conn, primeira).unwrap();
        let driver = &primeiro["runs"][0]["tool"]["driver"];
        assert_eq!(driver["version"].as_str(), Some("desconhecida"));
        assert_eq!(driver["rules"][0]["id"].as_str(), Some("dd-101-shell-true"));
        assert_eq!(
            driver["rules"][0]["helpUri"].as_str(),
            Some("https://cwe.mitre.org/data/definitions/78.html")
        );
        assert_eq!(
            primeiro["runs"][0]["results"][0]["ruleIndex"].as_i64(),
            Some(0)
        );
        assert!(
            primeiro["runs"][0]["results"][0]["partialFingerprints"]["devopsFingerprint/v1"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
        assert_eq!(
            primeiro["runs"][0]["invocations"][0]["executionSuccessful"].as_bool(),
            Some(true)
        );
        assert!(
            primeiro["runs"][0]["results"][0]
                .get("baselineState")
                .is_none(),
            "sem base não se inventa estado"
        );

        let segunda = corrida_para_exportar(
            &conn,
            projeto.id,
            "2026-01-02T10:00:00Z",
            &[
                achado_sarif("dd-101-shell-true", "src/a.py", 8, "os.system(x)"),
                achado_sarif("dd-105-avaliacao-de-codigo", "src/b.py", 2, "eval(y)"),
            ],
        );
        let segundo = sarif(&conn, segunda).unwrap();
        let resultados = segundo["runs"][0]["results"].as_array().unwrap();
        let estado = |regra: &str| {
            resultados
                .iter()
                .find(|r| r["ruleId"] == regra)
                .and_then(|r| r["baselineState"].as_str())
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(estado("dd-101-shell-true"), "unchanged");
        assert_eq!(estado("dd-105-avaliacao-de-codigo"), "new");
        assert_eq!(
            segundo["runs"][0]["properties"]["baselineRunId"].as_i64(),
            Some(primeira)
        );
        assert_eq!(
            segundo["runs"][0]["invocations"][0]["properties"]["fixed"].as_i64(),
            Some(0)
        );

        let pacote = pacote(&conn, segunda, None).unwrap();
        assert_eq!(pacote["comparacao"]["novos"].as_i64(), Some(1));
        assert_eq!(pacote["comparacao"]["persistentes"].as_i64(), Some(1));
        assert!(pacote["achados"][0]["impressao"]
            .as_str()
            .is_some_and(|m| !m.is_empty()));
    }
}
