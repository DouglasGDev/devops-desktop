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

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::models::{AnalysisCounts, AnalysisFinding, AnalysisInput, AnalysisRun, Project, SEVERIDADES};
use crate::{db, git_probe, proc, security, sshexec};

/// As regras embarcadas no aplicativo. Elas são copiadas para os dados do
/// aplicativo na primeira varredura porque o semgrep lê regras do disco: não há
/// como entregar-lhe um texto pela linha de comando.
pub const REGRAS_YAML: &str = include_str!("../regras/dd-seguranca.yaml");
pub const NOME_DAS_REGRAS: &str = "dd-seguranca.yaml";

/// Onde a tela grava o caminho de um semgrep fora do PATH — o caso real desta
/// máquina, com a ferramenta dentro de um ambiente virtual de teste.
pub const CHAVE_CONFIG: &str = "analise.semgrep.caminho";

/// Prazo de uma varredura quando o pedido não diz nada. Medido: 1181 arquivos com
/// as regras embarcadas levam ~7 s; 180 s é margem para um projeto grande sem
/// deixar a janela presa sem fim.
pub const PRAZO_PADRAO: Duration = Duration::from_secs(180);

/// Prazo da sondagem de versão. `semgrep --version` frio levou 2,2 s medido — o
/// prazo de 5 s das sondas de ferramenta é apertado demais para um Python que
/// precisa acordar, então a análise tem o seu.
const PRAZO_SONDA: Duration = Duration::from_secs(20);

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
    /// O que a ferramenta respondeu na sondagem de versão, em uma linha.
    pub resposta: String,
}

impl Ferramenta {
    pub fn encontrada(&self) -> bool {
        self.estado == "encontrado"
    }
}

/// Os nomes pelos quais o pacote se instala. No Windows o `pip` põe um `.exe` no
/// PATH; em POSIX o launcher é um script sem extensão.
fn nomes_da_ferramenta() -> &'static [&'static str] {
    if cfg!(windows) {
        &["semgrep.exe"]
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

/// Resolve o caminho que alguém apontou na configuração. Um nome com barra é um
/// caminho e tem de existir como arquivo; um nome solto é um programa e só o PATH
/// responde por ele.
fn resolver_explicito(bruto: &str) -> Option<String> {
    let nome = bruto.trim().trim_matches('"');
    if nome.is_empty() {
        return None;
    }
    if nome.contains('/') || nome.contains('\\') {
        let caminho = crate::ssh_config::expand_tilde(nome);
        return caminho.is_file().then(|| caminho.display().to_string());
    }
    no_path(nome)
}

/// Localiza o semgrep e pergunta a versão dele. Nada aqui é presunção: o programa
/// é executado com prazo, e quem não responde é dito sem resposta.
pub fn localizar(caminho_explicito: Option<&str>) -> Ferramenta {
    let inicio = Instant::now();
    let regras = contar_regras(REGRAS_YAML);
    let mut origem = String::new();
    let mut binario = caminho_explicito
        .and_then(resolver_explicito)
        .inspect(|_| origem = "configuração".to_string());
    if binario.is_none() {
        for nome in nomes_da_ferramenta() {
            if let Some(achado) = no_path(nome) {
                binario = Some(achado);
                origem = "path".to_string();
                break;
            }
        }
    }
    let Some(binario) = binario else {
        return Ferramenta {
            estado: "ausente".to_string(),
            caminho: None,
            versao: None,
            duracao_ms: duracao_ms(inicio),
            regras,
            origem,
            resposta: String::new(),
        };
    };
    let chamada = rodar(&binario, &["--version"], None, PRAZO_SONDA, &AtomicBool::new(false));
    if chamada.sem_resposta || chamada.code.is_none() && chamada.stderr.is_empty() {
        return Ferramenta {
            estado: "sem-resposta".to_string(),
            caminho: Some(binario),
            versao: None,
            duracao_ms: duracao_ms(inicio),
            regras,
            origem,
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
        resposta: cauda(&chamada.stdout),
    }
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
                .map(|c| if c.is_ascii_digit() || c == '.' { c } else { ' ' })
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
        &["scan", "--metrics=off", "--validate", "--config", caminho.as_ref()],
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
    Marca { texto, colado: false }
}

const fn mc(texto: &'static str) -> Marca {
    Marca { texto, colado: true }
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
        mensagem: "um comando é montado e entregue a um shell; se qualquer parte vier de fora, é injeção",
        cwe: "CWE-78: OS Command Injection",
        familias: &["py"],
        alternativas: &[&[m("subprocess."), mc("shell=true")], &[m("os.system(")], &[m("os.popen(")]],
        anterior: "",
    },
    RegraEmbutida {
        id: "dd-101-shell-true-php",
        severidade: "ERROR",
        mensagem: "entrada é levada a um comando de sistema",
        cwe: "CWE-78: OS Command Injection",
        familias: &["php"],
        alternativas: &[&[m("system(")], &[m("shell_exec(")], &[m("passthru(")], &[m("exec(")]],
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
        mensagem: "uma checagem automática foi calada nesta linha — o motivo precisa estar escrito aqui",
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
    let fim_ok = |j: usize| j >= bytes.len() || !eh_ident(alvo[alvo.len() - 1]) || !eh_ident(bytes[j]);
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
                let casa = regra
                    .alternativas
                    .iter()
                    .any(|alt| alt.iter().all(|marca| contem(escolha(marca, &min, &apertado), marca.texto)));
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
    let mut out = Vec::new();
    let mut pilha = vec![raiz.to_path_buf()];
    while let Some(pasta) = pilha.pop() {
        if out.len() >= LIMITE_ARQUIVOS {
            cobertura.cortada = true;
            break;
        }
        let Ok(leituras) = std::fs::read_dir(&pasta) else { continue };
        for entrada in leituras.flatten() {
            let nome = entrada.file_name().to_string_lossy().to_string();
            let caminho = entrada.path();
            let eh_dir = entrada
                .file_type()
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
    nome
        .rsplit('.')
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
struct Resposta {
    achados: Vec<Achado>,
    lidos: i64,
    pulados: i64,
    erros: i64,
    code: Option<i32>,
    sem_resposta: bool,
    cancelado: bool,
    ausente: bool,
    nota: Option<String>,
    regras: Vec<String>,
}

impl Resposta {
    fn falha(nota: String) -> Self {
        Resposta {
            achados: Vec::new(),
            lidos: 0,
            pulados: 0,
            erros: 0,
            code: None,
            sem_resposta: false,
            cancelado: false,
            ausente: true,
            nota: Some(nota),
            regras: Vec::new(),
        }
    }
}

/// Roda o semgrep sobre `alvo` e lê o JSON que ele escreveu. `raiz` é o diretório
/// de trabalho do filho, e é por ela que os caminhos do JSON chegam relativos ao
/// que foi varrido.
fn varredura_semgrep(
    binario: &str,
    regras: &Path,
    raiz: &Path,
    prazo: Duration,
    cancel: &AtomicBool,
) -> Resposta {
    let saida = std::env::temp_dir().join(format!("devops-desktop-semgrep-{}.json", std::process::id()));
    let arquivo_saida = saida.display().to_string();
    let args = [
        "scan",
        "--metrics=off",
        "--json-output",
        &arquivo_saida,
        "--config",
        &regras.display().to_string(),
        ".",
    ];
    let chamada = rodar(binario, &args, Some(raiz), prazo, cancel);
    if chamada.cancelado {
        let _ = std::fs::remove_file(&saida);
        return Resposta {
            cancelado: true,
            nota: Some("a varredura foi interrompida".to_string()),
            ..Resposta::falha(String::new())
        };
    }
    if chamada.sem_resposta {
        let _ = std::fs::remove_file(&saida);
        return Resposta {
            sem_resposta: true,
            nota: Some(format!(
                "o semgrep não terminou dentro do prazo de {} s — o que ele varreu não é conhecido",
                prazo.as_secs()
            )),
            ..Resposta::falha(String::new())
        };
    }
    let conteudo = std::fs::read_to_string(&saida);
    let _ = std::fs::remove_file(&saida);
    let Ok(conteudo) = conteudo else {
        return Resposta::falha(format!(
            "o semgrep não escreveu o resultado (código {:?}); disse: {}",
            chamada.code,
            cauda(&chamada.stderr)
        ));
    };
    let Ok(valor) = serde_json::from_str::<Value>(&conteudo) else {
        return Resposta::falha(format!(
            "o JSON do semgrep não é legível (código {:?})",
            chamada.code
        ));
    };
    ler_envelope(&valor, raiz, chamada, regras)
}

/// Lê o envelope real do semgrep 1.181: `results[]`, `errors[]`, `paths.scanned`,
/// `paths.skipped`. Cada campo é opcional — um formato que muda não pode fazer a
/// varredura mentir sobre o que não estava lá.
fn ler_envelope(valor: &Value, raiz: &Path, chamada: Chamada, regras: &Path) -> Resposta {
    let mut achados = Vec::new();
    let mut vistos: Vec<String> = Vec::new();
    let mut trechos = Trechos::default();
    if let Some(lista) = valor.get("results").and_then(Value::as_array) {
        for r in lista {
            let Some(id) = r.get("check_id").and_then(Value::as_str) else {
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
            let arquivo = r.get("path").and_then(Value::as_str).unwrap_or("");
            let inicio = r.get("start").and_then(|s| s.get("line")).and_then(Value::as_i64).unwrap_or(0);
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
    }
    let erros = contador_de_lista(valor.get("errors"));
    let nota = (erros > 0).then(|| {
        format!(
            "a ferramenta reportou {erros} erro(s) fora dos achados — o que ela não interpretou não foi varrido"
        )
    });
    Resposta {
        achados: dedupe(achados),
        lidos: contador_de_lista(valor.get("paths").and_then(|p| p.get("scanned"))),
        pulados: contador_de_lista(valor.get("paths").and_then(|p| p.get("skipped"))),
        erros,
        code: chamada.code,
        sem_resposta: false,
        cancelado: false,
        ausente: false,
        nota,
        regras: efetivamente_lidos(vistos, regras),
    }
}

/// As regras que a varredura tinha. Quando o JSON não nomeia nenhuma — varredura
/// sem achados, o caso mais provável — a lista vem do arquivo embarcado, para a
/// corrida nunca dizer "zero regras".
fn efetivamente_lidos(vistos: Vec<String>, caminho_das_regras: &Path) -> Vec<String> {
    if !vistos.is_empty() {
        return vistos;
    }
    std::fs::read_to_string(caminho_das_regras)
        .map(|t| ids_do_yaml(&t))
        .unwrap_or_else(|_| ids_embarcados().into_iter().map(|s| s.to_string()).collect())
}

/// `cwe` chega como texto ou como lista de textos, dependendo da regra. Os dois
/// são aceitos; o que sai é uma linha só.
fn cwe_da_metadata(metadata: &Value) -> Option<String> {
    match metadata.get("cwe") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Array(itens)) => {
            let junta = itens
                .iter()
                .filter_map(|i| i.as_str().or_else(|| i.get("value").and_then(Value::as_str)))
                .collect::<Vec<_>>()
                .join(", ");
            (!junta.is_empty()).then_some(junta)
        }
        _ => None,
    }
}

fn contador_de_lista(valor: Option<&Value>) -> i64 {
    valor.and_then(Value::as_array).map(|a| a.len() as i64).unwrap_or(0)
}

/// O fim da reclamação de uma ferramenta. Duzentos caracteres dizem o que foi; o
/// resto é log, e log de ferramenta não é achado.
fn cauda(texto: &str) -> String {
    let limpo = texto.trim();
    if limpo.len() <= 200 {
        return limpo.to_string();
    }
    let mut corte = 200;
    while !limpo.is_char_boundary(corte) {
        corte -= 1;
    }
    format!("…{}", &limpo[limpo.len() - corte..])
}

/// Busca a linha que o semgrep apontou e a devolve mascaráda.
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
            self.cache.insert(chave.clone(), texto.lines().map(|l| l.to_string()).collect());
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

/// O resultado de uma varredura é a corrida como ela ficou gravada, lida de volta
/// do banco: as duas portas (a tela e o nó do pipeline) leem esta mesma linha, e o
/// que elas mostram é o que está guardado, não o que a memória do processo ainda
/// tinha. `blocked` é o veredito — a contagem contra o corte — e nunca o código de
/// saída da ferramenta.
///
/// `raiz` é a pasta canônica do projeto; a subpasta opcional do pedido é conferida
/// por [`AnalysisInput::conferir`] e contida por `canonicalize`, a mesma régua dos
/// nós de comando locais.
pub fn executar(
    conn: &Connection,
    dados: &Path,
    projeto: &Project,
    raiz: &Path,
    entrada: &AnalysisInput,
    cancel: &AtomicBool,
) -> Result<AnalysisRun, String> {
    let inicio = Instant::now();
    let relativa = entrada.conferir()?;
    let pedido = raiz.join(relativa.as_deref().unwrap_or(""));
    let raiz_real = raiz
        .canonicalize()
        .map_err(|e| format!("não foi possível abrir a pasta do projeto: {e}"))?;
    let alvo = pedido
        .canonicalize()
        .map_err(|_| format!("a pasta varrida “{}” não existe dentro do projeto", pedido.display()))?;
    if !alvo.starts_with(&raiz_real) {
        return Err("a varredura só roda dentro da pasta cadastrada do projeto".to_string());
    }
    // A contenção foi medida nos caminhos canônicos; o que vai para a tela e para o
    // pacote é o caminho que outro programa reconhece.
    let alvo = git_probe::simple(&alvo);
    let instrumento = entrada.analyzer();
    let corte = entrada.severity_gate();
    let prazo = if entrada.timeout_secs.unwrap_or(0) > 0 {
        Duration::from_secs(entrada.timeout_secs.unwrap_or(0) as u64)
    } else {
        PRAZO_PADRAO
    };
    let quer_externo = instrumento != "embutido";
    let ferramenta = if quer_externo {
        let explicito = db::setting_value(conn, CHAVE_CONFIG)?.unwrap_or_default();
        localizar(Some(&explicito))
    } else {
        Ferramenta {
            estado: "não-usada".to_string(),
            caminho: None,
            versao: None,
            duracao_ms: 0,
            regras: contar_regras(REGRAS_YAML),
            origem: String::new(),
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
    // O instrumento externo foi pedido e não esteve disponível: é diferente de
    // "não foi pedido", e é isso que a nota e o corte precisam dizer.
    let mut externo_indisponivel = false;

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
            let resposta = varredura_semgrep(
                ferramenta.caminho.as_deref().unwrap_or("semgrep"),
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
            cobertura.lidos += resposta.lidos;
            cobertura.pulados += resposta.pulados;
            achados.extend(resposta.achados);
            regras_usadas.extend(resposta.regras);
            if let Some(nota) = resposta.nota {
                notas.push(nota);
            }
            externo_indisponivel = resposta.ausente || resposta.sem_resposta;
            if externo_indisponivel {
                notas.push(match ferramenta.estado.as_str() {
                    "sem-resposta" => "o semgrep existe mas não respondeu à sondagem de versão".to_string(),
                    _ => format!(
                        "o semgrep não estava disponível nesta máquina — instale-o ou aponte o caminho em {CHAVE_CONFIG}"
                    ),
                });
            }
        } else {
            externo_indisponivel = true;
            notas.push(match ferramenta.estado.as_str() {
                "sem-resposta" => "o semgrep existe mas não respondeu à sondagem de versão".to_string(),
                _ => format!(
                    "o semgrep não estava disponível nesta máquina — instale-o ou aponte o caminho em {CHAVE_CONFIG}"
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
        notas.push("a varredura não leu arquivo nenhum — não há o que afirmar sobre o código".to_string());
    }
    // Um corte é uma promessa de leitura completa. Com o instrumento pedido
    // ausente, a promessa não pode ser cumprida por metade dela: o varrido é
    // dito, mas a corrida não passa por limpa.
    aberto.id = run_id;
    aberto.status = if sem_leitura || (externo_indisponivel && instrumento == "semgrep") {
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
    aberto.blocked = barra_o_corte(&corte, &counts)
        || ((sem_leitura || externo_indisponivel) && !corte.is_empty());
    aberto.counts = counts;
    aberto.rules = regras_usadas.join(", ");
    aberto.note = (!notas.is_empty()).then(|| notas.join(" · "));
    db::fechar_analise(conn, run_id, &aberto)?;

    db::obter_analise(conn, run_id)
}

/// Fecha a corrida como interrompida. Uma varredura cortada no meio não é uma
/// varredura limpa, e é assim que ela tem de aparecer no histórico.
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
    fechado.note = Some("a varredura foi interrompida antes do fim".to_string());
    db::fechar_analise(conn, run_id, &fechado)?;
    db::obter_analise(conn, run_id)
}

/// O corte é uma frase sobre o futuro: "com qualquer coisa deste nível para cima,
/// a publicação para". Vazio é "nada barra".
pub fn barra_o_corte(corte: &str, counts: &AnalysisCounts) -> bool {
    match corte {
        "ERROR" => counts.error > 0,
        "WARNING" => counts.error + counts.warning > 0,
        "INFO" => counts.total() > 0,
        _ => false,
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
    let mostrados = if truncado { &achados[..limite] } else { &achados[..] };
    let veredito = classificar(&run);
    let decisoes = decisoes(&run, &veredito);

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
        "porRegra": agrupar_por_regra(mostrados),
        "achados": mostrados.iter().map(|a| json!({
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
    } else if run.blocked {
        "barra"
    } else if run.tool_errors > 0 || run.tool.contains("ausente") {
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
    match rotulo {
        "barra" => out.push(json!({
            "acao": "parar-a-publicacao",
            "motivo": format!("{} achado(s) no corte {} ou acima", run.counts.total(), run.severity_gate),
            "evidencia": "veredito.contagem",
        })),
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
        _ => {}
    }
    if run.tool_errors > 0 {
        out.push(json!({
            "acao": "revalidar-varredura",
            "motivo": format!("a ferramenta não interpretou {} arquivo(s); o que ela não leu não foi varrido", run.tool_errors),
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
        let entrada = contar
            .entry(a.rule_id.as_str())
            .or_insert((0, Vec::new(), a.severity.as_str()));
        entrada.0 += 1;
        if !entrada.1.contains(&a.file.as_str()) {
            entrada.1.push(a.file.as_str());
        }
    }
    contar
        .into_iter()
        .map(|(regra, (quantidade, arquivos, severidade))| json!({
            "regra": regra,
            "severidade": severidade,
            "quantidade": quantidade,
            "arquivosDiferentes": arquivos.len(),
            "exemplos": arquivos.iter().take(3).copied().collect::<Vec<_>>(),
        }))
        .collect()
}

/// SARIF 2.1.0 da mesma corrida, para abrir o resultado em outra ferramenta. É
/// gerado das linhas gravadas, então vale para os dois instrumentos e não depende
/// de a ferramenta externa estar presente.
pub fn sarif(conn: &Connection, run_id: i64) -> Result<Value, String> {
    let run = db::obter_analise(conn, run_id)?;
    let achados = db::listar_achados(conn, run_id, None, LIMITE_ACHADOS as i64)?;
    let regras: Vec<Value> = agrupar_por_regra(&achados)
        .into_iter()
        .map(|r| {
            json!({
                "id": r["regra"],
                "name": r["regra"],
                "shortDescription": { "text": r["regra"] },
                "defaultConfiguration": {
                    "level": nivel_da_severidade(r["severidade"].as_str().unwrap_or("INFO")),
                },
            })
        })
        .collect();
    let results: Vec<Value> = achados
        .iter()
        .map(|a| {
            json!({
                "ruleId": a.rule_id,
                "level": nivel_da_severidade(&a.severity),
                "message": { "text": a.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": a.file.replace('\\', "/") },
                        "region": { "startLine": a.line_start, "endLine": a.line_end },
                    }
                }],
            })
        })
        .collect();
    Ok(json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "devops-desktop",
                    "version": run.tool_version,
                    "informationUri": "https://github.com/DouglasGDev/devops-desktop",
                    "rules": regras,
                }
            },
            "results": results,
            "columnKind": "utf16CodeUnits",
        }],
    }))
}

fn nivel_da_severidade(severidade: &str) -> &'static str {
    match severidade {
        "ERROR" => "error",
        "WARNING" => "warning",
        _ => "note",
    }
}

/// Grava o pacote ou o SARIF nos dados do aplicativo e devolve o caminho escrito.
/// O arquivo existe para ser entregue a alguém — uma IA, uma revisão, outra
/// ferramenta — sem que o aplicativo precise de um servidor para isso.
pub fn exportar(
    conn: &Connection,
    dados: &Path,
    run_id: i64,
    formato: &str,
    limite: Option<usize>,
) -> Result<String, String> {
    let valor = match formato {
        "pacote" => pacote(conn, run_id, limite)?,
        "sarif" => sarif(conn, run_id)?,
        outro => return Err(format!("a exportação escreve pacote ou sarif, não “{outro}”")),
    };
    let pasta = dados.join("analises");
    std::fs::create_dir_all(&pasta)
        .map_err(|e| format!("não foi possível criar a pasta das exportações: {e}"))?;
    let destino = pasta.join(format!("corrida-{run_id}.{}", if formato == "sarif" { "sarif" } else { "json" }));
    let texto = serde_json::to_string_pretty(&valor)
        .map_err(|e| format!("falha ao escrever a exportação: {e}"))?;
    std::fs::write(&destino, texto)
        .map_err(|e| format!("não foi possível gravar {}: {e}", destino.display()))?;
    Ok(destino.display().to_string())
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
        let Ok(guard) = self.ativas.lock() else { return false };
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
        let caminho = std::env::temp_dir().join(format!("dd-analise-{prefixo}-{}-{n}", std::process::id()));
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
    fn as_duas_listas_de_regras_tem_os_mesmos_ids_na_mesma_ordem() {        // Os dois instrumentos têm de falar a mesma língua, ou `ambos` produz duas
        // listas que ninguém soma.
        let embutidas: Vec<String> = ids_embarcados().into_iter().map(|s| s.to_string()).collect();
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
            assert!(vistas.contains(&regra.id), "a regra {} não disparou", regra.id);
        }
        assert!(cobertura.lidos >= 5, "poucos arquivos lidos: {}", cobertura.lidos);
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
        escrever(&pasta, "bom.php", "<?php\n$hesita = password_hash($s, PASSWORD_DEFAULT);\n");
        let (achados, cobertura) = varredura_embutida(&pasta, &AtomicBool::new(false));
        assert_eq!(cobertura.lidos, 3);
        assert!(achados.is_empty(), "{:?}", achados.iter().map(|a| (&a.file, a.line_start, &a.rule_id)).collect::<Vec<_>>());
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
        assert!(contem(&colado("// eslint-disable-next-line"), "eslint-disable"));
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
        assert_eq!(cobertura.lidos, 1, "{:?}", achados.iter().map(|a| &a.file).collect::<Vec<_>>());
        assert!(achados.iter().all(|a| a.file == "app.py"));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn o_corte_diz_quando_barra() {
        let contagem = AnalysisCounts { error: 0, warning: 2, info: 5 };
        assert!(!barra_o_corte("", &contagem));
        assert!(!barra_o_corte("ERROR", &contagem));
        assert!(barra_o_corte("WARNING", &contagem));
        assert!(barra_o_corte("INFO", &contagem));
        assert!(barra_o_corte(
            "ERROR",
            &AnalysisCounts { error: 1, warning: 0, info: 0 }
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
        Chamada { stdout: String::new(), stderr: String::new(), code, sem_resposta: false, cancelado: false }
    }

    #[test]
    fn o_codigo_de_saida_da_ferramenta_nunca_e_o_veredito() {
        // Medido: varredura completa com um arquivo mal interpretado volta código 2
        // e JSON inteiro; outra, sem achados, volta código 0. As duas têm de virar
        // o mesmo tipo de conclusão — o que pesa são os dados.
        let pasta = pasta_de_teste("envelope");
        escrever(&pasta, "a.py", "import os\n\nos.system(cmd)\n");
        let resposta = ler_envelope(&envelope_real(), &pasta, chamada_de(Some(2)), Path::new("regras.yaml"));
        assert_eq!(resposta.achados.len(), 1);
        assert_eq!(resposta.achados[0].rule_id, "dd-105-avaliacao-de-codigo");
        assert_eq!(resposta.achados[0].severity, "ERROR");
        assert_eq!(resposta.achados[0].cwe.as_deref(), Some("CWE-94: Code Injection"));
        assert_eq!(resposta.lidos, 3);
        assert_eq!(resposta.pulados, 1);
        assert_eq!(resposta.erros, 1);
        assert_eq!(resposta.code, Some(2));
        assert!(!resposta.ausente && !resposta.sem_resposta);
        let nota = resposta.nota.clone().unwrap_or_default();
        assert!(nota.contains("1 erro(s)"), "{nota}");
        // O trecho veio do arquivo lido por nós, não do JSON — que em modo livre
        // devolve "requires login" no lugar da linha.
        assert_eq!(resposta.achados[0].snippet.as_deref(), Some("os.system(cmd)"));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    /// O arquivo de regras embarcado, pelo caminho real do crate. É o que a
    /// varredura materializa, e o que os testes de leitura conferem.
    fn caminho_das_regras() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("regras").join(NOME_DAS_REGRAS)
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
    fn um_envelope_incompleto_nao_inventa_numero() {
        let resposta = ler_envelope(
            &json!({ "results": [] }),
            Path::new("."),
            chamada_de(None),
            Path::new("../regras/dd-seguranca.yaml"),
        );
        assert_eq!(resposta.lidos, 0);
        assert_eq!(resposta.pulados, 0);
        assert_eq!(resposta.erros, 0);
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
        let resposta = ler_envelope(&valor, Path::new("."), chamada_de(Some(0)), Path::new("r.yaml"));
        assert_eq!(resposta.achados[0].severity, "WARNING");
    }

    #[test]
    fn o_trecho_gravado_nunca_tem_o_valor_de_um_segredo() {
        let pasta = pasta_de_teste("mascara");
        escrever(&pasta, "a.py", "API_TOKEN = 'abc123SECRET'\nos.system('ls')\n");
        let (achados, _) = varredura_embutida(&pasta, &AtomicBool::new(false));
        assert!(!achados.is_empty(), "a varredura não achou a linha");
        for a in &achados {
            let texto = a.snippet.clone().unwrap_or_default();
            assert!(!texto.contains("abc123SECRET"), "o valor de uma atribuição vazou: {texto}");
        }
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
        assert_eq!(versao_da_resposta("1.181.0", "").as_deref(), Some("1.181.0"));
        assert_eq!(versao_da_resposta("semgrep 1.181.0\n", "").as_deref(), Some("1.181.0"));
        assert_eq!(versao_da_resposta("", "Traceback: nada de versão aqui").as_deref(), None);
    }

    #[test]
    fn as_regras_materializadas_sao_as_do_aplicativo() {
        let dados = pasta_de_teste("regras");
        let arquivo = publicar_regras(&dados).unwrap();
        assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), REGRAS_YAML);
        // Uma regra antiga esquecida no disco é reescrita, não respeitada.
        escrever(&dados, "regras/dd-seguranca.yaml", "rules:\n  - id: antiga\n");
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
            "cheia" => AnalysisCounts { error: 2, warning: 0, info: 0 },
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
        assert_eq!(classificar(&base)["classificacao"].as_str(), Some("sem-achados"));
        assert_eq!(
            decisoes(&base, &classificar(&base))[0]["acao"].as_str(),
            Some("prosseguir")
        );
        assert_eq!(classificar(&corrida("sem-leitura"))["classificacao"].as_str(), Some("cobertura-nula"));
        assert_eq!(classificar(&corrida("cheia"))["classificacao"].as_str(), Some("barra"));
        assert_eq!(
            decisoes(&corrida("cheia"), &classificar(&corrida("cheia")))[0]["acao"].as_str(),
            Some("parar-a-publicacao")
        );
        let reclamou = corrida("erros");
        assert_eq!(classificar(&reclamou)["classificacao"].as_str(), Some("revalidar-varredura"));
        let acoes: Vec<String> = decisoes(&reclamou, &classificar(&reclamou))
            .iter()
            .map(|d| d["acao"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(acoes.contains(&"revalidar-varredura".to_string()), "{acoes:?}");

        let mut interrompida = base.clone();
        interrompida.status = "interrompida".to_string();
        assert_eq!(classificar(&interrompida)["classificacao"].as_str(), Some("varredura-interrompida"));

        let ferramenta_fora = AnalysisRun { tool: "semgrep ausente".to_string(), ..base };
        assert!(decisoes(&ferramenta_fora, &classificar(&ferramenta_fora))
            .iter()
            .any(|d| d["acao"].as_str() == Some("instalar-ou-apontar-a-ferramenta")));
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
        }];
        let grupo = agrupar_por_regra(&achados);
        let resultados: Vec<Value> = achados
            .iter()
            .map(|a| json!({
                "ruleId": a.rule_id,
                "level": nivel_da_severidade(&a.severity),
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": a.file.replace('\\', "/") },
                        "region": { "startLine": a.line_start },
                    }
                }],
            }))
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
        assert_eq!(resolver_explicito(nome).as_deref(), Some(nome));
        assert!(resolver_explicito("C:\\não\\há\\nada\\aqui.exe").is_none());
        assert!(resolver_explicito("").is_none());
    }

    #[test]
    fn uma_ferramenta_ausente_e_dita_ausente_e_nao_sem_resposta() {
        // O nome não existe: `ausente`. É a diferença entre "instale" e "espere".
        let ferramenta = localizar(Some("um-programa-que-nao-existe-em-lugar-nenhum"));
        assert_eq!(ferramenta.estado, "ausente");
        assert!(ferramenta.caminho.is_none());
        assert_eq!(ferramenta.regras, 19);
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
        db::insert_project(conn, "casa", &pasta.display().to_string(), false, None, None).unwrap()
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
        assert_eq!(corrida.project, "casa", "a corrida diz de quem é sem segunda consulta");
        assert!(corrida.blocked, "o código plantado tem erro e o corte é ERROR");
        assert!(corrida.counts.error > 0);
        assert!(!corrida.rules.is_empty());
        assert!(corrida.tool_version.is_none(), "o embutido não tem versão de ferramenta");
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

    #[test]
    fn a_varredura_nao_sai_da_pasta_do_projeto() {
        let casa = arvore_de_teste();
        let conn = banco();
        let projeto = projeto_de_teste(&conn, &casa);
        let dados = pasta_de_teste("dados-fora");
        let raiz = raiz_do_projeto(&projeto).unwrap();
        let mut dobrando = pedido(projeto.id, "embutido", "");
        dobrando.path = Some("../de-lado".to_string());
        let err = executar(&conn, &dados, &projeto, &raiz, &dobrando, &AtomicBool::new(false))
            .unwrap_err();
        assert!(err.contains("caminho relativo"), "{err}");
        // uma pasta que não existe no projeto também não varre nada
        let mut inexistente = pedido(projeto.id, "embutido", "");
        inexistente.path = Some("nao-existe".to_string());
        assert!(executar(&conn, &dados, &projeto, &raiz, &inexistente, &AtomicBool::new(false)).is_err());
        let _ = std::fs::remove_dir_all(&casa);
        let _ = std::fs::remove_dir_all(&dados);
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
        let caminho = exportar(&conn, &dados, corrida.id, "pacote", None).unwrap();
        assert!(caminho.ends_with(&format!("corrida-{}.json", corrida.id)), "{caminho}");
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
        let caminho = exportar(&conn, &dados, corrida.id, "sarif", None).unwrap();
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
        assert_eq!(estado_da_conferencia(&chamada("", "", Some(0), false)), "validadas");
        assert_eq!(
            estado_da_conferencia(&chamada("", "configuration error", Some(7), false)),
            "inválidas"
        );
        // quem não respondeu não é "inválidas" — e muito menos "validadas"
        assert_eq!(estado_da_conferencia(&chamada("", "", None, true)), "sem-resposta");
        assert_eq!(estado_da_conferencia(&chamada("", "", Some(0), true)), "sem-resposta");
    }
}
