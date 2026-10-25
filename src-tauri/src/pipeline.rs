//! O contrato central de execução de pipelines (plano CI/CD, etapa 2).
//!
//! Não há um segundo motor aqui. Cada operação termina no [`crate::deploy`], que é
//! quem roda as quinze etapas, fala com o servidor e grava o histórico. O que este
//! módulo acrescenta é a porta única pela qual qualquer módulo pede uma execução e
//! acompanha o resultado, mais a procedência que toda execução passa a carregar:
//! definição, versão, ambiente, destino e commit de origem.
//!
//! Acompanhar logs e eventos continua pelos canais que o motor já emite
//! (`deploy-step` e `deploy-log`); duplicar esse canal aqui seria criar uma segunda
//! fonte de verdade sobre o que está acontecendo. O que o motor escreve nesses
//! canais **sobrevive** em `deploy_step_logs`, tentativa por tentativa, e a porta de
//! leitura deste módulo é a única forma de reler aquilo depois — inclusive o bloco
//! de uma tentativa que a repetição do nó não apagou. Do mesmo jeito, *pedir uma
//! execução*, *interromper* e *consultar o estado* já têm dono no app —
//! `deploy_start`, `deploy_cancel`, `deploy_run` — e o contrato não reemporta essas
//! três portas. O que ele acrescenta é o que faltava para o pipeline ser uma coisa
//! só com história: a definição versionada, a conferência dela sem executar, a
//! procedência de cada execução e a nova tentativa só quando ela é segura.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use rusqlite::Connection;
use serde::Serialize;
use tauri::AppHandle;

use crate::{
    audit, auth,
    db::{self, DbHandle},
    deploy,
    models::{
        DefinitionRevision, DeployProfile, DeployRun, DeployRunStep, PipelineNode,
        PipelineNodeInput, StepLogLine, MAX_NODES, NODE_ANALISADORES, NODE_EXECUTORS, NODE_GIT_OPS,
        SEVERIDADES,
    },
};

/// Primeira etapa que toca o destino: 9 cria o backup/ponto de recuperação no
/// destino, 10 envia, 11 em diante mudam o que está lá. Antes da 9 nada aconteceu
/// fora da pasta do projeto, então repetir é seguro.
const PRIMEIRA_ETAPA_NO_DESTINO: i64 = 9;

// ---------- o grafo desenhado na tela ----------

/// O essencial de um nó para conferir a forma do grafo. Vem da tela (antes de
/// salvar) ou do banco (na hora de validar ou de executar), e é por isso que a
/// conferência existe uma única vez: o que a tela mostra é o que o motor roda, e
/// as duas pontas passam pelo mesmo checker.
#[derive(Debug, Clone)]
pub struct NoParaChecar {
    pub chave: String,
    pub rotulo: String,
    pub kind: String,
    pub etapa: Option<i64>,
    pub executor: String,
    /// A operação de um nó `git`. Vazio nos outros tipos, e é por isso que a
    /// conferência do grafo exige que ele venha preenchido quando o tipo é `git`:
    /// sem operação o nó seria um desenho no quadro que ninguém executa.
    pub operacao: String,
    /// O instrumento de um nó `analise`, vazio nos outros tipos pela mesma razão
    /// do campo acima: sem instrumento o nó varreria nada e aprovaria tudo.
    pub instrumento: String,
    /// O corte de severidade do nó `analise`. Vazio é uma escolha legítima — o nó
    /// varre e informa sem barrar — mas um valor inventado barraria para sempre.
    pub corte: String,
    pub dependencias: Vec<String>,
    pub paralelo: bool,
}

impl From<&PipelineNode> for NoParaChecar {
    fn from(no: &PipelineNode) -> Self {
        Self {
            chave: no.node_key.clone(),
            rotulo: format!("nó “{}”", no.node_key),
            kind: no.kind.clone(),
            etapa: no.engine_step,
            executor: no.executor.clone(),
            operacao: no.git_op.clone().unwrap_or_default(),
            instrumento: no.analyzer.clone(),
            corte: no.severity_gate.clone(),
            dependencias: no.depends_on.clone(),
            paralelo: no.parallel,
        }
    }
}

impl From<&PipelineNodeInput> for NoParaChecar {
    fn from(no: &PipelineNodeInput) -> Self {
        Self {
            chave: no.node_key.clone(),
            rotulo: format!("nó “{}”", no.node_key.trim()),
            kind: no.kind(),
            etapa: no.engine_step,
            executor: no.executor(),
            operacao: no.git_op(),
            instrumento: no.analyzer(),
            corte: no.severity_gate(),
            dependencias: no.depends_on(),
            paralelo: no.parallel.unwrap_or(false),
        }
    }
}

/// O grafo de uma definição, conferido com o que ela tem fora do desenho: um nó que
/// roda no servidor exige um servidor cadastrado. É a porta que a tela usa ao salvar
/// e que `validate` usa antes de executar, então um grafo inválido não chega ao motor.
pub fn checar_grafo(nos: &[NoParaChecar], tem_servidor: bool) -> Result<String, String> {
    if nos.is_empty() {
        return Ok("sem grafo · as quinze etapas do motor, em fila".to_string());
    }
    for no in nos {
        if no.executor == "ssh" && !tem_servidor {
            return Err(format!(
                "{} roda no servidor por SSH, e esta definição publica nesta máquina — escolha um servidor ou troque o executor para local",
                no.rotulo
            ));
        }
    }
    checar_forma(nos)
}

/// Os nós de que um nó depende, direta ou indiretamente. Caminho sobre as arestas
/// ao contrário, com memória, porque um grafo de quarenta nós pode ser conferido
/// inteiro sem custar nada.
fn ancestrais(no: &NoParaChecar, todos: &HashMap<&str, &NoParaChecar>) -> HashSet<String> {
    let mut vistos = HashSet::new();
    let mut fila: Vec<&str> = no.dependencias.iter().map(|d| d.as_str()).collect();
    while let Some(chave) = fila.pop() {
        if !vistos.insert(chave.to_string()) {
            continue;
        }
        if let Some(antes) = todos.get(chave) {
            fila.extend(antes.dependencias.iter().map(|d| d.as_str()));
        }
    }
    vistos
}

/// A forma do grafo: identificadores únicos, arestas que apontam para nós que
/// existem, nenhum ciclo, paralelismo onde o motor aguenta, e cada etapa do motor
/// alcançada por quem produz o estado que ela lê.
///
/// O último item é o que impede um desenho bonito de ser uma corrida quebrada: um
/// nó "copiar os arquivos" sem um nó "reunir os arquivos" antes não tem o que
/// copiar, e o motor falharia no meio da execução dizendo isso.
pub fn checar_forma(nos: &[NoParaChecar]) -> Result<String, String> {
    if nos.is_empty() {
        return Err("um grafo precisa de pelo menos um nó — sem grafo a definição roda as quinze etapas em fila".to_string());
    }
    if nos.len() > MAX_NODES {
        return Err(format!("o grafo aceita no máximo {MAX_NODES} nós"));
    }
    let todos: HashMap<&str, &NoParaChecar> = nos.iter().map(|n| (n.chave.trim(), n)).collect();
    if todos.len() != nos.len() {
        let mut vistas: HashSet<&str> = HashSet::new();
        let repetida = nos
            .iter()
            .map(|n| n.chave.trim())
            .find(|c| !vistas.insert(*c))
            .unwrap_or("");
        return Err(format!(
            "dois nós compartilham o identificador “{repetida}”"
        ));
    }
    for no in nos {
        if no.chave.trim().is_empty() {
            return Err("há nó sem identificador".to_string());
        }
        for dep in &no.dependencias {
            if dep.trim() == no.chave.trim() {
                return Err(format!("{} depende dele mesmo", no.rotulo));
            }
            if !todos.contains_key(dep.trim()) {
                return Err(format!(
                    "{} depende de “{}”, que não existe neste grafo",
                    no.rotulo,
                    dep.trim()
                ));
            }
        }
        if no.paralelo && no.kind != "comando" {
            return Err(format!(
                "{} está marcado para rodar em paralelo, e só um nó de comando pode dividir ramal com um irmão — as etapas do motor produzem estado para as seguintes",
                no.rotulo
            ));
        }
    }
    if let Some(sobrou) = achar_ciclo(nos, &todos) {
        return Err(format!(
            "o grafo tem um ciclo: {} nunca terminam de esperar um pelo outro",
            sobrou
                .iter()
                .map(|c| format!("“{c}”"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut etapas = 0usize;
    let mut comandos = 0usize;
    let mut aprovacoes = 0usize;
    let mut git = 0usize;
    let mut analises = 0usize;
    for no in nos {
        match no.kind.as_str() {
            "motor" => {
                etapas += 1;
                let Some(numero) = no.etapa else {
                    return Err(format!("{} não aponta para nenhuma etapa do motor", no.rotulo));
                };
                if !(1..=15).contains(&numero) {
                    return Err(format!("{} aponta para a etapa {numero}, que o motor não tem", no.rotulo));
                }
                let antes: HashSet<i64> = ancestrais(no, &todos)
                    .iter()
                    .filter_map(|c| todos.get(c.as_str()))
                    .filter(|n| n.kind == "motor")
                    .filter_map(|n| n.etapa)
                    .collect();
                let faltam: Vec<String> = deploy::prequisitos(numero)
                    .iter()
                    .filter(|p| !antes.contains(p))
                    .map(|p| format!("etapa {p} ({} · {})", deploy::step_name(*p), motivo_da_etapa(*p)))
                    .collect();
                if !faltam.is_empty() {
                    return Err(format!(
                        "{} roda a etapa {numero}, mas nenhuma dependência dele produz o que ela lê: {}",
                        no.rotulo,
                        faltam.join(", ")
                    ));
                }
            }
            "comando" => {
                comandos += 1;
                if !NODE_EXECUTORS.contains(&no.executor.as_str()) {
                    return Err(format!(
                        "{} usa o executor “{}”, que o motor não tem",
                        no.rotulo, no.executor
                    ));
                }
            }
            "aprovacao" => aprovacoes += 1,
            "git" => {
                git += 1;
                if !NODE_GIT_OPS.contains(&no.operacao.as_str()) {
                    return Err(format!(
                        "{} é um nó git sem operação que o motor rode: use {}",
                        no.rotulo,
                        NODE_GIT_OPS.join(", ")
                    ));
                }
            }
            "analise" => {
                analises += 1;
                if !NODE_ANALISADORES.contains(&no.instrumento.as_str()) {
                    return Err(format!(
                        "{} é um nó de análise sem instrumento que o motor rode: use {}",
                        no.rotulo,
                        NODE_ANALISADORES.join(", ")
                    ));
                }
                if !(no.corte.is_empty() || SEVERIDADES.contains(&no.corte.as_str())) {
                    return Err(format!(
                        "{} corta por “{}”, e o corte dele aceita {}, vazio incluso — sem corte o nó informa, não barra",
                        no.rotulo,
                        no.corte,
                        SEVERIDADES.join(", ")
                    ));
                }
                // A varredura lê a pasta do projeto nesta máquina; não existe
                // análise remota por SSH, então um nó assim seria um desenho falso.
                if no.executor != "local" {
                    return Err(format!(
                        "{} analisa o código desta máquina, e o executor dele é local por definição",
                        no.rotulo
                    ));
                }
            }
            outro => {
                return Err(format!(
                    "{} é do tipo “{outro}”, e o motor só conhece motor, comando, aprovacao, git e analise",
                    no.rotulo
                ))
            }
        }
    }
    Ok(format!(
        "grafo de {} nó(s) · {etapas} etapa(s) do motor, {comandos} de comando, {aprovacoes} de aprovação, {git} de git, {analises} de análise",
        nos.len()
    ))
}

/// O que a etapa produz, dito como o erro de dependência precisa dizer.
fn motivo_da_etapa(position: i64) -> &'static str {
    match position {
        3 => "lê o ramo e o commit de origem",
        4 => "decide quais arquivos vão mudar",
        8 => "cria a release no destino",
        _ => "produz estado para as etapas seguintes",
    }
}

/// Ciclo é o que impede o grafo de ter uma ordem. A procura é Kahn: todo nó sem
/// dependência sai do grafo e libera quem depende dele; se sobrar nó, sobrou ciclo,
/// e os identificadores que sobraram são a prova dele.
fn achar_ciclo(nos: &[NoParaChecar], todos: &HashMap<&str, &NoParaChecar>) -> Option<Vec<String>> {
    let mut indepesas: HashMap<&str, usize> = nos
        .iter()
        .map(|n| {
            (
                n.chave.trim(),
                n.dependencias
                    .iter()
                    .filter(|d| todos.contains_key(d.trim()))
                    .count(),
            )
        })
        .collect();
    let mut fila: Vec<&str> = indepesas
        .iter()
        .filter(|(_, q)| **q == 0)
        .map(|(c, _)| *c)
        .collect();
    let mut removidos = 0usize;
    while let Some(chave) = fila.pop() {
        removidos += 1;
        for no in nos
            .iter()
            .filter(|n| n.dependencias.iter().any(|d| d.trim() == chave))
        {
            if let Some(q) = indepesas.get_mut(no.chave.trim()) {
                *q -= 1;
                if *q == 0 {
                    fila.push(no.chave.trim());
                }
            }
        }
    }
    (removidos != nos.len()).then(|| {
        nos.iter()
            .filter(|n| indepesas.get(n.chave.trim()).copied().unwrap_or(0) > 0)
            .map(|n| n.chave.clone())
            .collect()
    })
}

/// Uma definição como o contrato a entrega: o modelo salvo e a última execução dele.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionSummary {
    pub definition: DeployProfile,
    pub last_run_id: Option<i64>,
    pub last_run_status: Option<String>,
    pub last_run_at: Option<String>,
}

/// Se vale a pena oferecer uma nova tentativa, e por que não vale.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    pub allowed: bool,
    pub reason: Option<String>,
}

/// Listar definições com a última execução de cada uma. As execuções vêm de uma
/// única consulta, não de uma por definição.
pub fn definitions(
    db: &DbHandle,
    project_id: Option<i64>,
) -> Result<Vec<DefinitionSummary>, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    let perfis = db::list_deploy_profiles(&conn, project_id)?;
    let execucoes = db::list_runs(&conn, project_id, 200)?;
    Ok(perfis
        .into_iter()
        .map(|perfil| {
            let ultima = execucoes.iter().find(|r| r.profile_id == perfil.id);
            DefinitionSummary {
                last_run_id: ultima.map(|r| r.id),
                last_run_status: ultima.map(|r| r.status.clone()),
                last_run_at: ultima.map(|r| r.started_at.clone()),
                definition: perfil,
            }
        })
        .collect())
}

/// As versões salvas de uma definição, com o retrato do que ela era em cada uma.
pub fn revisions(db: &DbHandle, definition_id: i64) -> Result<Vec<DefinitionRevision>, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    db::list_revisions(&conn, definition_id)
}

/// O grafo salvo de uma definição, na ordem em que a tela o desenha.
pub fn nodes(db: &DbHandle, definition_id: i64) -> Result<Vec<PipelineNode>, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    db::list_pipeline_nodes(&conn, definition_id)
}

/// Gravar o grafo é gravar a definição: sobe a versão, o retrato novo entra no
/// histórico, e nada executa. Cada nó é conferido sozinho e o grafo inteiro é
/// conferido junto — a validação não é um favor à tela, é a garantia de que o
/// desenho que você vê é o caminho que o motor percorre.
pub fn save_nodes(
    db: &DbHandle,
    definition_id: i64,
    nos: &[PipelineNodeInput],
) -> Result<Vec<PipelineNode>, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    let perfil = db::get_deploy_profile(&conn, definition_id)?;
    for no in nos {
        no.validate()?;
    }
    let lista: Vec<NoParaChecar> = nos.iter().map(NoParaChecar::from).collect();
    checar_grafo(&lista, perfil.server_id.is_some())?;
    let gravados = db::replace_pipeline_nodes(&conn, definition_id, nos)?;
    let _ = audit::log(
        &conn,
        "info",
        "deploy",
        "Grafo do pipeline salvo",
        None,
        Some(&perfil.name),
        None,
        Some(&format!(
            "definição #{} · v{} · {} nó(s){}",
            definition_id,
            perfil.version + 1,
            gravados.len(),
            if gravados.is_empty() {
                " · volta à fila de quinze etapas"
            } else {
                ""
            }
        )),
    );
    Ok(gravados)
}

/// Conferir a definição sem executar nada: as mesmas checagens da etapa 1 do motor,
/// através da mesma função. Os valores das variáveis não entram aqui — eles só
/// existem na hora da execução, e o motor continua cobrando a ausência deles.
pub fn validate(db: &DbHandle, definition_id: i64) -> Result<String, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    let perfil = db::get_deploy_profile(&conn, definition_id)?;
    let projeto = db::get_project(&conn, perfil.project_id)?;
    let servidor = match perfil.server_id {
        Some(id) => Some(db::get_server(&conn, id)?),
        None => None,
    };
    let raiz = Path::new(&projeto.local_path);
    if !raiz.is_dir() {
        return Err(format!(
            "a pasta local do projeto não existe mais: {}",
            projeto.local_path
        ));
    }
    deploy::checar_definicao(&perfil, raiz, servidor.as_ref())?;
    let nos = db::list_pipeline_nodes(&conn, definition_id)?;
    let lista: Vec<NoParaChecar> = nos.iter().map(NoParaChecar::from).collect();
    let grafo = checar_grafo(&lista, servidor.is_some())?;
    Ok(format!(
        "definição “{}” v{} · {} · destino {} · {} · {}",
        perfil.name,
        perfil.version,
        perfil.strategy,
        perfil.remote_path.trim(),
        if servidor.is_some() {
            "servidor SSH"
        } else {
            "nesta máquina"
        },
        grafo
    ))
}

/// Estado atual de uma execução, inclusive enquanto ela roda.
fn run_state(db: &DbHandle, run_id: i64) -> Result<DeployRun, String> {
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    db::get_run(&conn, run_id)
}

/// A política de nova tentativa. `allowed = false` nunca é um "não" mudo: vem com
/// o que precisa ser conferido antes de publicar de novo.
pub fn retry_policy(db: &DbHandle, run_id: i64) -> Result<RetryPolicy, String> {
    let estado = run_state(db, run_id)?;
    Ok(match politica(&estado) {
        Ok(()) => RetryPolicy {
            allowed: true,
            reason: None,
        },
        Err(reason) => RetryPolicy {
            allowed: false,
            reason: Some(reason),
        },
    })
}

/// A regra, separada da leitura do banco para poder ser testada sozinha.
pub fn politica(execucao: &DeployRun) -> Result<(), String> {
    if execucao.status == "running" {
        return Err(format!(
            "a execução #{} ainda está marcada como em andamento — confirme o destino antes de publicar de novo",
            execucao.id
        ));
    }
    if execucao.steps.is_empty() {
        return Err(format!(
            "a execução #{} não tem nenhuma etapa registrada — não é possível saber o que ela chegou a fazer no destino",
            execucao.id
        ));
    }
    // Uma execução que terminou bem já é um "publicar de novo" pedido pelo usuário,
    // não uma tentativa sobre resultado desconhecido.
    if execucao.status == "success" {
        return Ok(());
    }
    if execucao.steps.iter().any(tocou_o_destino) {
        return Err(format!(
            "a execução #{} chegou a mexer no destino ({}), então o resultado lá é desconhecido — confira o destino antes de repetir",
            execucao.id,
            descricao_da_etapa(execucao)
        ));
    }
    Ok(())
}

/// Se a linha da execução registra trabalho no destino. Numa execução linear é a
/// posição a partir da etapa 9; numa execução de grafo a posição é a linha do
/// desenho, e quem diz o que aconteceu lá é a marca que a própria etapa registrou.
fn tocou_o_destino(etapa: &DeployRunStep) -> bool {
    matches!(etapa.status.as_str(), "running" | "done" | "failed")
        && (etapa.touched_destination
            || (etapa.node_key.is_none() && etapa.position >= PRIMEIRA_ETAPA_NO_DESTINO))
}

fn descricao_da_etapa(execucao: &DeployRun) -> String {
    execucao
        .steps
        .iter()
        .filter(|s| tocou_o_destino(s))
        .map(|s| match &s.node_key {
            Some(chave) => format!("nó “{chave}”: {} · {}", s.name, s.status),
            None => format!("etapa {}: {} · {}", s.position, s.name, s.status),
        })
        .next()
        .unwrap_or_else(|| "etapa de destino".to_string())
}

/// Nova tentativa: reexecuta a mesma definição, só quando a política libera. A
/// autorização explícita de segredos **não** herda da execução anterior — ela é
/// pedida de novo, porque é uma decisão sobre conteúdo, não sobre o pipeline.
pub fn retry(
    app: AppHandle,
    state: &deploy::DeployState,
    db: DbHandle,
    broker: auth::AuthBroker,
    run_id: i64,
    secret_values: HashMap<String, String>,
) -> Result<i64, String> {
    let origem = run_state(&db, run_id)?;
    politica(&origem)?;
    let alca = db.clone();
    let novo = deploy::start(
        app,
        state,
        db,
        broker,
        origem.profile_id,
        secret_values,
        false,
    )?;
    if let Ok(conn) = alca.lock() {
        let _ = audit::log(
            &conn,
            "info",
            "deploy",
            "Nova tentativa de pipeline",
            origem.server_name.as_deref(),
            Some(&origem.project_name),
            None,
            Some(&format!(
                "execução #{run_id} → #{novo} · definição #{}",
                origem.profile_id
            )),
        );
    }
    Ok(novo)
}

/// Quantas linhas de uma etapa a leitura devolve quando ninguém pede um tope.
const LOGS_POR_LEITURA: i64 = 500;

/// As linhas de saída que uma etapa deixou, por ordem de tentativa. É a porta da
/// tela e a do que for consultar depois — a gravação continua sendo do motor, no
/// mesmo ponto em que ele emite o evento, então o que se lê aqui é o que a janela
/// mostrou na hora. `query` filtra por trecho do texto e `limit` é um tope **por
/// bloco de tentativa**: um bloco cheio não esconde o da tentativa seguinte, e quem
/// pede o bloco inteiro recebe também a linha que declara o corte. Pedir menos do
/// que o bloco tem continua sendo um tope, não uma promessa de que aquilo era o fim.
pub fn step_logs(
    db: &DbHandle,
    run_id: i64,
    position: i64,
    query: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<StepLogLine>, String> {
    // O teto da gravação mais a linha única que declara o corte: um tope parado
    // exatamente no teto cortaria justamente o aviso de que houve corte.
    let teto = deploy::TETO_LINHAS_POR_TENTATIVA + 1;
    let pedido = limit.unwrap_or(LOGS_POR_LEITURA as u32) as i64;
    if pedido <= 0 {
        return Err("a leitura do log de uma etapa precisa pedir ao menos uma linha".to_string());
    }
    let conn = db
        .lock()
        .map_err(|_| "o banco local está indisponível".to_string())?;
    db::read_step_logs(
        &conn,
        run_id,
        position,
        None,
        query.as_deref(),
        pedido.min(teto),
    )
}

/// Execução órfã: o aplicativo foi fechado enquanto um pipeline rodava. O estado
/// `'running'` não significa mais nada — não há processo nenhum — e deixá-lo assim
/// faria a tela mostrar um deploy que nunca termina. É marcado como interrompido,
/// com o motivo, e nada é reexecutado por conta própria.
pub fn recover_open_runs(conn: &Connection) -> usize {
    let agora = db::now_rfc3339();
    let execucoes = conn
        .execute(
            "UPDATE deploy_runs SET status = 'cancelled', finished_at = ?1, \
             detail = 'interrompida — o aplicativo foi fechado antes de terminar' \
             WHERE status = 'running'",
            [&agora],
        )
        .unwrap_or(0);
    let etapas = conn
        .execute(
            "UPDATE deploy_run_steps SET status = 'failed', message = 'interrompida — o aplicativo foi fechado', \
             finished_at = ?1 WHERE status = 'running'",
            [&agora],
        )
        .unwrap_or(0);
    if execucoes > 0 {
        let _ = audit::log(
            conn,
            "warning",
            "deploy",
            "Execuções interrompidas identificadas",
            None,
            None,
            None,
            Some(&format!(
                "{execucoes} execução(ões) e {etapas} etapa(s) marcadas como interrompidas na abertura do aplicativo"
            )),
        );
    }
    execucoes
}

/// As varreduras de código que ficaram em 'rodando' porque o aplicativo fechou no
/// meio delas. Entram na mesma abertura: são estado de execução, como o deploy, e
/// uma análise que nunca terminou não pode aparecer como se tivesse passado.
pub fn recover_open_analyses(conn: &Connection) -> usize {
    db::recuperar_analises_abertas(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execucao(status: &str, etapas: Vec<(i64, &str)>) -> DeployRun {
        let steps = etapas
            .iter()
            .map(|(position, st)| linha(*position, st, None, false))
            .collect();
        run_com(status, steps)
    }

    /// Uma linha do histórico como ela é salva. Os campos do grafo entram aqui
    /// porque a política de nova tentativa lê exatamente eles.
    fn linha(position: i64, status: &str, node_key: Option<&str>, touched: bool) -> DeployRunStep {
        DeployRunStep {
            position,
            name: node_key
                .map(|c| format!("nó {c}"))
                .unwrap_or_else(|| format!("etapa {position}")),
            status: status.into(),
            command: None,
            exit_code: None,
            message: None,
            started_at: None,
            finished_at: None,
            node_key: node_key.map(str::to_string),
            touched_destination: touched,
        }
    }

    fn run_com(status: &str, steps: Vec<DeployRunStep>) -> DeployRun {
        DeployRun {
            id: 7,
            profile_id: 3,
            project_name: "site".into(),
            server_name: Some("web".into()),
            strategy: "sftp".into(),
            status: status.into(),
            started_at: "2026-10-10T00:00:00Z".into(),
            finished_at: None,
            release_dir: None,
            previous_release: None,
            previous_image: None,
            detail: None,
            definition_version: Some(2),
            destination: Some("/var/www/site".into()),
            environment: Some("production".into()),
            commit_sha: Some("abc".into()),
            steps,
        }
    }

    /// Execução de grafo: as linhas são a ordem do desenho, e o que diz se o destino
    /// foi tocado é a marca que o próprio nó registrou.
    fn execucao_de_nos(status: &str, nos: Vec<(&str, &str, bool)>) -> DeployRun {
        let steps = nos
            .iter()
            .enumerate()
            .map(|(i, (chave, st, toca))| linha(i as i64 + 1, st, Some(chave), *toca))
            .collect();
        run_com(status, steps)
    }

    #[test]
    fn repetir_e_libere_depois_do_destino_nao_ter_sido_tocado() {
        // falhou na etapa de testes (6): nada saiu da pasta do projeto
        let antes = execucao("failed", vec![(1, "done"), (2, "done"), (6, "failed")]);
        assert!(politica(&antes).is_ok());
    }

    #[test]
    fn nao_repete_quando_o_destino_pode_ter_mudado() {
        let depois = execucao("failed", vec![(1, "done"), (9, "done"), (10, "failed")]);
        let err = politica(&depois).unwrap_err();
        assert!(err.contains("resultado lá é desconhecido"), "{err}");
        assert!(err.contains("etapa 9"), "{err}");
    }

    #[test]
    fn execucao_em_andamento_ou_sem_etapas_nunca_repete() {
        let viva = execucao("running", vec![(1, "done")]);
        assert!(politica(&viva).unwrap_err().contains("em andamento"));
        let muda = execucao("failed", vec![]);
        assert!(politica(&muda)
            .unwrap_err()
            .contains("nenhuma etapa registrada"));
    }

    #[test]
    fn sucesso_pode_ser_publicado_de_novo() {
        let bom = execucao("success", vec![(1, "done"), (10, "done"), (15, "done")]);
        assert!(politica(&bom).is_ok());
    }

    #[test]
    fn rollback_que_chegou_a_etapa_de_destino_nao_e_repetivel() {
        // o pipeline de reversão pula 3..=10 e roda a 11: marcou 9 e 10 como
        // `skipped`, então a regra não é "existe linha", é "a etapa trabalhou"
        let volta = execucao(
            "failed",
            vec![(1, "done"), (9, "skipped"), (10, "skipped"), (11, "failed")],
        );
        assert!(politica(&volta).unwrap_err().contains("etapa 11"));
    }

    #[test]
    fn grafo_que_so_testou_e_repetivel_embora_os_nos_sejam_a_linha_do_desenho() {
        // posições 1, 2 e 3 seriam "antes do destino" na fila; num grafo elas não
        // significam nada, e quem responde é a marca que o nó registrou
        let antes = execucao_de_nos(
            "failed",
            vec![
                ("testes", "done", false),
                ("build", "done", false),
                ("lint", "failed", false),
            ],
        );
        assert!(politica(&antes).is_ok());
    }

    #[test]
    fn grafo_que_tocou_o_destino_nao_repete_sem_conferir() {
        let depois = execucao_de_nos(
            "failed",
            vec![
                ("coleta", "done", false),
                ("envio", "done", true),
                ("ativa", "failed", true),
            ],
        );
        let err = politica(&depois).unwrap_err();
        assert!(err.contains("resultado lá é desconhecido"), "{err}");
        assert!(err.contains("nó “envio”"), "{err}");
    }

    /// Banco no esquema atual, com as tabelas mínimas para uma execução existir.
    fn conn_com_execucao(status: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, local_path, is_git_repo, created_at, updated_at) \
             VALUES (1, 'site', 'C:/projetos/site', 0, 'agora', 'agora')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deploy_profiles (id, project_id, name, server_id, strategy, remote_path, \
             created_at, updated_at) VALUES (1, 1, 'produção', NULL, 'script', 'C:/publicados', 'agora', 'agora')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deploy_runs (id, profile_id, status, started_at) VALUES (1, 1, ?1, 'agora')",
            [status],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deploy_run_steps (run_id, position, name, status) VALUES (1, 10, 'Copiar os arquivos', 'running')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn execucao_orfa_vira_interrompida_na_abertura() {
        let conn = conn_com_execucao("running");
        assert_eq!(recover_open_runs(&conn), 1);

        let estado = db::get_run(&conn, 1).unwrap();
        assert_eq!(estado.status, "cancelled");
        assert!(estado.detail.unwrap().contains("aplicativo foi fechado"));
        assert_eq!(estado.steps[0].status, "failed");
        // a segunda abertura não encontra mais nada: nada é reexecutado por conta própria
        assert_eq!(recover_open_runs(&conn), 0);
    }

    #[test]
    fn execucao_ja_terminada_nao_e_tocada_pela_varredura() {
        let conn = conn_com_execucao("failed");
        assert_eq!(recover_open_runs(&conn), 0);
        assert_eq!(db::get_run(&conn, 1).unwrap().status, "failed");
    }

    fn no(chave: &str, kind: &str, etapa: Option<i64>, dependencias: &[&str]) -> NoParaChecar {
        NoParaChecar {
            chave: chave.into(),
            rotulo: format!("nó “{chave}”"),
            kind: kind.into(),
            etapa,
            executor: "local".into(),
            operacao: String::new(),
            instrumento: String::new(),
            corte: String::new(),
            dependencias: dependencias.iter().map(|d| d.to_string()).collect(),
            paralelo: false,
        }
    }

    fn no_git(chave: &str, operacao: &str, dependencias: &[&str]) -> NoParaChecar {
        NoParaChecar {
            operacao: operacao.into(),
            ..no(chave, "git", None, dependencias)
        }
    }

    fn no_analise(
        chave: &str,
        instrumento: &str,
        corte: &str,
        dependencias: &[&str],
    ) -> NoParaChecar {
        NoParaChecar {
            instrumento: instrumento.into(),
            corte: corte.into(),
            ..no(chave, "analise", None, dependencias)
        }
    }

    fn no_no_servidor(chave: &str) -> NoParaChecar {
        let mut n = no(chave, "comando", None, &[]);
        n.executor = "ssh".into();
        n
    }

    /// O desenho que a etapa 3 promete aceitar: fila do motor com um ramal de
    /// comando e uma aprovação no meio.
    fn grafo_valido() -> Vec<NoParaChecar> {
        vec![
            no("validar", "motor", Some(1), &[]),
            no("git", "motor", Some(3), &["validar"]),
            no("coleta", "motor", Some(4), &["git"]),
            no("prepara", "motor", Some(8), &["coleta"]),
            no("migracao", "comando", None, &["prepara"]),
            no("envio", "motor", Some(10), &["prepara", "migracao"]),
            no("ativa", "motor", Some(11), &["envio"]),
            no("confere", "aprovacao", None, &["ativa"]),
        ]
    }

    #[test]
    fn grafo_bem_formado_passa_e_diz_o_que_e() {
        let resumo = checar_forma(&grafo_valido()).unwrap();
        assert!(resumo.contains("8 nó(s)"), "{resumo}");
        assert!(resumo.contains("6 etapa(s) do motor"), "{resumo}");
    }

    #[test]
    fn sem_nos_nao_ha_grafo_para_conferir() {
        assert!(checar_forma(&[]).is_err());
        // mas uma definição sem grafo é um estado válido: é a fila de quinze etapas
        assert!(checar_grafo(&[], false).unwrap().contains("fila"));
    }

    #[test]
    fn aresta_para_no_que_nao_existe_e_recusada() {
        let mut nos = grafo_valido();
        nos.push(no("depois", "comando", None, &["fantasma"]));
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("não existe neste grafo"), "{err}");
    }

    #[test]
    fn no_que_depende_de_si_mesmo_e_recusado() {
        let mut nos = grafo_valido();
        nos[4].dependencias.push("migracao".into());
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("depende dele mesmo"), "{err}");
    }

    #[test]
    fn ciclo_e_recusado_com_os_nos_que_o_formam() {
        let mut nos = grafo_valido();
        // "prepara" passa a esperar "confere", que já espera "ativa", que espera "prepara"
        nos[3].dependencias.push("confere".into());
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("ciclo"), "{err}");
        assert!(err.contains("confere"), "{err}");
    }

    #[test]
    fn etapa_do_motor_sem_quem_produz_o_estado_dela_e_recusada() {
        // "envio" (etapa 10) depende só do coletor: ninguém criou a release no
        // destino e o ramo de origem nunca foi lido
        let nos = vec![
            no("coleta", "motor", Some(4), &[]),
            no("prepara", "motor", Some(8), &["coleta"]),
            no("envio", "motor", Some(10), &["coleta"]),
        ];
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("roda a etapa 10"), "{err}");
        assert!(err.contains("etapa 3"), "{err}");
        assert!(err.contains("etapa 8"), "{err}");
    }

    #[test]
    fn prerequisito_atraves_de_terceiros_vale() {
        // "envio" não aponta para o coletor nem para o preparador: quem atende a
        // etapa 4 e a etapa 8 é a cadeia que passa pela aprovação
        let nos = vec![
            no("validar", "motor", Some(1), &[]),
            no("git", "motor", Some(3), &["validar"]),
            no("coleta", "motor", Some(4), &["git"]),
            no("prepara", "motor", Some(8), &["coleta"]),
            no("portao", "aprovacao", None, &["prepara"]),
            no("envio", "motor", Some(10), &["portao"]),
        ];
        assert!(checar_forma(&nos).is_ok());
    }

    #[test]
    fn paralelismo_so_em_no_de_comando() {
        let mut nos = grafo_valido();
        nos[5].paralelo = true; // "envio" é etapa do motor
        let err = checar_forma(&nos).unwrap_err();
        assert!(
            err.contains("só um nó de comando pode dividir ramal"),
            "{err}"
        );
        // um nó de comando marcado para dividir ramal passa
        let mut outros = grafo_valido();
        outros[4].paralelo = true; // "migracao"
        assert!(checar_forma(&outros).is_ok());
    }

    #[test]
    fn dois_nos_com_o_mesmo_identificador_sao_recusados() {
        let mut nos = grafo_valido();
        nos.push(no("migracao", "comando", None, &[]));
        let err = checar_forma(&nos).unwrap_err();
        assert!(
            err.contains("compartilham o identificador “migracao”"),
            "{err}"
        );
    }

    #[test]
    fn no_de_ssh_exige_servidor_na_definicao() {
        let nos = vec![no_no_servidor("migracao")];
        let err = checar_grafo(&nos, false).unwrap_err();
        assert!(err.contains("publica nesta máquina"), "{err}");
        assert!(checar_grafo(&nos, true).unwrap().contains("1 nó(s)"));
    }

    #[test]
    fn no_sem_identificador_e_recusado() {
        let mut nos = grafo_valido();
        nos[0].chave = " ".into();
        assert!(checar_forma(&nos)
            .unwrap_err()
            .contains("sem identificador"));
    }

    #[test]
    fn mais_nos_do_que_a_tela_aguenta_e_recusado() {
        let nos: Vec<NoParaChecar> = (0..=MAX_NODES)
            .map(|i| no(&format!("n{i}"), "comando", None, &[]))
            .collect();
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("no máximo"), "{err}");
    }

    #[test]
    fn etapa_fora_das_quinze_do_motor_e_recusada() {
        let nos = vec![no("estranha", "motor", Some(16), &[])];
        let err = checar_forma(&nos).unwrap_err();
        assert!(err.contains("que o motor não tem"), "{err}");
    }

    #[test]
    fn tipo_de_no_que_o_motor_nao_conhece_e_recusado() {
        let nos = vec![no("magica", "agenda", None, &[])];
        let err = checar_forma(&nos).unwrap_err();
        assert!(
            err.contains("motor, comando, aprovacao, git e analise"),
            "{err}"
        );
    }

    #[test]
    fn o_no_git_existe_no_grafo_porque_o_motor_roda_ele() {
        let mut nos = grafo_valido();
        nos.push(no_git("marca", "tag", &["confere"]));
        let resumo = checar_forma(&nos).unwrap();
        assert!(resumo.contains("1 de git"), "{resumo}");
        assert!(checar_forma(&[no_git("puxa", "pull", &[])]).is_ok());
        assert!(checar_forma(&[no_git("muda", "checkout", &[])]).is_ok());
        // sem operação o nó seria um desenho no quadro que nenhuma etapa executa
        let sem_operacao = checar_forma(&[no_git("marca", "", &[])]).unwrap_err();
        assert!(sem_operacao.contains("sem operação"), "{sem_operacao}");
        let inventada = checar_forma(&[no_git("marca", "deploy", &[])]).unwrap_err();
        assert!(inventada.contains("pull, checkout, tag"), "{inventada}");
        // a pasta de trabalho do git é uma só: nó git não divide ramal com irmão
        let mut paralelo = no_git("puxa", "pull", &[]);
        paralelo.paralelo = true;
        let err = checar_forma(&[paralelo]).unwrap_err();
        assert!(
            err.contains("só um nó de comando pode dividir ramal"),
            "{err}"
        );
    }

    #[test]
    fn o_no_de_analise_existe_no_grafo_porque_o_motor_varre_ele() {
        let mut nos = grafo_valido();
        nos.push(no_analise("varredura", "ambos", "ERROR", &["confere"]));
        let resumo = checar_forma(&nos).unwrap();
        assert!(resumo.contains("1 de análise"), "{resumo}");

        // sem corte o nó informa sem barrar: é uma escolha legítima, não um erro
        assert!(checar_forma(&[no_analise("informa", "embutido", "", &[])]).is_ok());
        assert!(checar_forma(&[no_analise("marca", "semgrep", "WARNING", &[])]).is_ok());

        // instrumento que não existe varreria nada e aprovaria tudo
        let inventado = checar_forma(&[no_analise("varre", "bandit", "ERROR", &[])]).unwrap_err();
        assert!(
            inventado.contains("sem instrumento que o motor rode"),
            "{inventado}"
        );
        assert!(
            inventado.contains("embutido, semgrep, ambos"),
            "{inventado}"
        );

        // um corte inventado barraria para sempre — ninguém entenderia o porquê
        let corte = checar_forma(&[no_analise("varre", "embutido", "CRITICO", &[])]).unwrap_err();
        assert!(corte.contains("e o corte dele aceita"), "{corte}");

        // análise remota não existe: o código lido é o desta máquina
        let remoto = NoParaChecar {
            executor: "ssh".into(),
            ..no_analise("varre", "embutido", "ERROR", &[])
        };
        let err = checar_forma(&[remoto]).unwrap_err();
        assert!(err.contains("executor dele é local por definição"), "{err}");

        // varrer o projeto inteiro antes de qualquer etapa é um desenho válido
        let standalone = checar_forma(&[
            no_analise("varre", "embutido", "ERROR", &[]),
            no("validar", "motor", Some(1), &["varre"]),
        ]);
        assert!(standalone.is_ok(), "{:?}", standalone);
    }

    #[test]
    fn a_porta_de_leitura_do_log_ve_as_tentativas_em_ordem() {
        let conn = conn_com_execucao("failed");
        db::append_step_logs(
            &conn,
            1,
            10,
            1,
            &[db::LinhaDeLog {
                stream: "stdout",
                text: "tentativa um: permissão negada",
            }],
        )
        .unwrap();
        db::append_step_logs(
            &conn,
            1,
            10,
            2,
            &[db::LinhaDeLog {
                stream: "stderr",
                text: "tentativa dois: publicada",
            }],
        )
        .unwrap();
        // Banco em memória não tem arquivo para reabrir: a alça empresta a única
        // conexão que existe.
        let alça: DbHandle = std::sync::Arc::new(db::Db::unica(conn));

        // A tela lê pelo mesmo portão que qualquer outro consultor: as duas
        // tentativas, na ordem, sem nada sobrescrito.
        let todas = step_logs(&alça, 1, 10, None, None).unwrap();
        assert_eq!(
            todas
                .iter()
                .map(|l| (l.attempt, l.seq, l.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (1, 0, "tentativa um: permissão negada"),
                (2, 0, "tentativa dois: publicada"),
            ]
        );

        // A busca é o que foi digitado, ligado por parâmetro.
        let buscas = step_logs(&alça, 1, 10, Some("publicada".to_string()), None).unwrap();
        assert_eq!(buscas.len(), 1);
        assert_eq!(buscas[0].attempt, 2);
        assert!(step_logs(&alça, 1, 10, Some("  ".to_string()), None).is_ok());

        // Pedir zero linha é um pedido que não se pode atender, não um vazio fingido.
        let nada = step_logs(&alça, 1, 10, None, Some(0)).unwrap_err();
        assert!(nada.contains("ao menos uma linha"), "{nada}");

        // E o tope pedido acima do teto do motor é contido pelo teto, não ignorado:
        // a leitura não promete mais do que a gravação pode ter deixado.
        assert_eq!(step_logs(&alça, 1, 10, None, Some(9_999)).unwrap().len(), 2);

        // Um bloco cheio — as 2.000 linhas do teto mais a única que declara o corte —
        // é lido inteiro. Uma leitura que se limitasse ao teto esconderia justamente o
        // aviso de que houve corte, que é a única pista de que falta saída ali.
        let saida: String = (0..2_500)
            .map(|i| format!("linha {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cheio = deploy::bloco_de_log("out", &saida, 0);
        let linhas: Vec<db::LinhaDeLog<'_>> = cheio
            .iter()
            .map(|(s, t)| db::LinhaDeLog {
                stream: s.as_str(),
                text: t.as_str(),
            })
            .collect();
        {
            let conn = alça.lock().unwrap();
            db::append_step_logs(&conn, 1, 10, 3, &linhas).unwrap();
        }
        let bloco = step_logs(&alça, 1, 10, None, Some(9_999)).unwrap();
        // As duas linhas das tentativas anteriores continuam junto: o tope é por
        // bloco, então o bloco cheio não engoliu quem veio antes dele.
        assert_eq!(bloco.len(), 2 + cheio.len());
        assert_eq!(bloco.iter().filter(|l| l.attempt == 3).count(), cheio.len());
        let ultima = bloco.last().unwrap();
        assert_eq!((ultima.attempt, ultima.stream.as_str()), (3, "system"));
        assert!(ultima.text.contains("truncado"), "{}", ultima.text);
    }
}
