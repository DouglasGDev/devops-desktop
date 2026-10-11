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
//! fonte de verdade sobre o que está acontecendo. Do mesmo jeito, *pedir uma
//! execução*, *interromper* e *consultar o estado* já têm dono no app —
//! `deploy_start`, `deploy_cancel`, `deploy_run` — e o contrato não reemporta essas
//! três portas. O que ele acrescenta é o que faltava para o pipeline ser uma coisa
//! só com história: a definição versionada, a conferência dela sem executar, a
//! procedência de cada execução e a nova tentativa só quando ela é segura.

use std::{collections::HashMap, path::Path};

use rusqlite::Connection;
use serde::Serialize;
use tauri::AppHandle;

use crate::{
    audit, auth,
    db::{self, DbHandle},
    deploy,
    models::{DefinitionRevision, DeployProfile, DeployRun},
};

/// Primeira etapa que toca o destino: 9 cria o backup/ponto de recuperação no
/// destino, 10 envia, 11 em diante mudam o que está lá. Antes da 9 nada aconteceu
/// fora da pasta do projeto, então repetir é seguro.
const PRIMEIRA_ETAPA_NO_DESTINO: i64 = 9;

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
pub fn definitions(db: &DbHandle, project_id: Option<i64>) -> Result<Vec<DefinitionSummary>, String> {
    let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
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
    let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
    db::list_revisions(&conn, definition_id)
}

/// Conferir a definição sem executar nada: as mesmas checagens da etapa 1 do motor,
/// através da mesma função. Os valores das variáveis não entram aqui — eles só
/// existem na hora da execução, e o motor continua cobrando a ausência deles.
pub fn validate(db: &DbHandle, definition_id: i64) -> Result<String, String> {
    let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
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
    Ok(format!(
        "definição “{}” v{} · {} · destino {} · {}",
        perfil.name,
        perfil.version,
        perfil.strategy,
        perfil.remote_path.trim(),
        if servidor.is_some() { "servidor SSH" } else { "nesta máquina" }
    ))
}

/// Estado atual de uma execução, inclusive enquanto ela roda.
fn run_state(db: &DbHandle, run_id: i64) -> Result<DeployRun, String> {
    let conn = db.lock().map_err(|_| "o banco local está indisponível".to_string())?;
    db::get_run(&conn, run_id)
}

/// A política de nova tentativa. `allowed = false` nunca é um "não" mudo: vem com
/// o que precisa ser conferido antes de publicar de novo.
pub fn retry_policy(db: &DbHandle, run_id: i64) -> Result<RetryPolicy, String> {
    let estado = run_state(db, run_id)?;
    Ok(match politica(&estado) {
        Ok(()) => RetryPolicy { allowed: true, reason: None },
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
    let tocou_destino = execucao
        .steps
        .iter()
        .any(|s| s.position >= PRIMEIRA_ETAPA_NO_DESTINO
            && matches!(s.status.as_str(), "running" | "done" | "failed"));
    if tocou_destino {
        return Err(format!(
            "a execução #{} chegou a mexer no destino ({}), então o resultado lá é desconhecido — confira o destino antes de repetir",
            execucao.id,
            descricao_da_etapa(execucao)
        ));
    }
    Ok(())
}

fn descricao_da_etapa(execucao: &DeployRun) -> String {
    execucao
        .steps
        .iter()
        .filter(|s| s.position >= PRIMEIRA_ETAPA_NO_DESTINO)
        .filter(|s| matches!(s.status.as_str(), "running" | "done" | "failed"))
        .map(|s| format!("etapa {}: {} · {}", s.position, s.name, s.status))
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
    let novo = deploy::start(app, state, db, broker, origem.profile_id, secret_values, false)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn execucao(status: &str, etapas: Vec<(i64, &str)>) -> DeployRun {
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
            detail: None,
            definition_version: Some(2),
            destination: Some("/var/www/site".into()),
            environment: Some("production".into()),
            commit_sha: Some("abc".into()),
            steps: etapas
                .into_iter()
                .map(|(position, st)| crate::models::DeployRunStep {
                    position,
                    name: format!("etapa {position}"),
                    status: st.into(),
                    command: None,
                    exit_code: None,
                    message: None,
                    started_at: None,
                    finished_at: None,
                })
                .collect(),
        }
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
        assert!(politica(&muda).unwrap_err().contains("nenhuma etapa registrada"));
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
}
