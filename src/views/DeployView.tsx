import { useCallback, useEffect, useRef, useState } from "react";
import { api, onDeployDone, onDeployLog, onDeployStep, onPipelineApproval } from "../lib/api";
import { formatBytes, formatDateTime } from "../lib/format";
import type {
  ApprovalRequest,
  DeployActive,
  DeployPreflight,
  DeployProfile,
  DeployRun,
  EngineStep,
  LogEvent,
  ViewId,
} from "../types";
import { useApp } from "../store/AppContext";
import { AlertTriangleIcon, CheckIcon, KeyIcon, RefreshIcon, RocketIcon, UndoIcon, XIcon } from "../components/Icons";
import { PipelineGraph } from "../components/deploy/PipelineGraph";
import { StepPipeline } from "../components/deploy/StepPipeline";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  DESTINO_LOCAL,
  EmptyState,
  ENV_TONES,
  Field,
  Input,
  Modal,
  SectionTitle,
  Select,
  Spinner,
  STRATEGY_LABELS,
  type BadgeTone,
} from "../components/ui";

const LOG_LIMIT = 400;

/**
 * Tela de deploy (capítulo 10). Nada aqui simula resultado: o pipeline roda no
 * motor Rust e esta tela desenha apenas os eventos que chegam dele.
 */
export function DeployView({ onNavigate }: { onNavigate: (v: ViewId) => void }) {
  const { pushToast } = useApp();
  const [profiles, setProfiles] = useState<DeployProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [profileId, setProfileId] = useState<number | null>(null);
  const [preflight, setPreflight] = useState<DeployPreflight | null>(null);
  const [checking, setChecking] = useState(false);
  const [allowSecrets, setAllowSecrets] = useState(false);
  const [run, setRun] = useState<DeployRun | null>(null);
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [runs, setRuns] = useState<DeployRun[]>([]);
  const [active, setActive] = useState<DeployActive[]>([]);
  const [retryBlock, setRetryBlock] = useState<string | null>(null);
  const [aba, setAba] = useState<"executar" | "grafo">("executar");
  const [engineSteps, setEngineSteps] = useState<EngineStep[]>([]);
  const [aprovacao, setAprovacao] = useState<ApprovalRequest | null>(null);
  const [secretsFor, setSecretsFor] = useState<
    | { kind: "deploy" }
    | { kind: "rollback"; runId: number }
    | { kind: "retry"; runId: number }
    | { kind: "no"; nodeKey: string; comDependencias: boolean }
    | null
  >(null);

  // Os listeners são montados uma vez; eles filtram pela execução atual por
  // aqui, sem se inscrever de novo a cada clique.
  const currentRun = useRef<number>(0);

  const profile = profiles.find((p) => p.id === profileId) ?? null;

  const openRun = useCallback(async (runId: number) => {
    currentRun.current = runId;
    setRetryBlock(null);
    setLogs([]);
    try {
      const detail = await api.deployRun(runId);
      setRun(detail);
      setRunning(detail.status === "running");
    } catch (e) {
      pushToast("error", `Falha ao abrir a execução #${runId}: ${String(e)}`);
    }
  }, [pushToast]);

  const loadHistory = useCallback(async () => {
    try {
      const [list, inFlight] = await Promise.all([api.deployRuns(null, 30), api.deployActive()]);
      setRuns(list);
      setActive(inFlight);
    } catch (e) {
      pushToast("error", `Falha ao ler o histórico de deploys: ${String(e)}`);
    }
  }, [pushToast]);

  useEffect(() => {
    let unstep: (() => void) | undefined;
    let unlog: (() => void) | undefined;
    let undone: (() => void) | undefined;
    let unapr: (() => void) | undefined;
    let alive = true;

    void (async () => {
      setLoading(true);
      try {
        const list = await api.listDeployProfiles(null);
        if (!alive) return;
        setProfiles(list);
        setProfileId((current) => {
          if (current !== null && list.some((p) => p.id === current)) return current;
          const fallback = list.find((p) => p.isDefault) ?? list[0];
          return fallback ? fallback.id : null;
        });
      } catch (e) {
        if (alive) pushToast("error", `Falha ao carregar os modelos: ${String(e)}`);
      } finally {
        if (alive) setLoading(false);
      }
    })();

    void (async () => {
      const [list, inFlight] = await Promise.all([api.deployRuns(null, 30), api.deployActive()]);
      if (!alive) return;
      setRuns(list);
      setActive(inFlight);
      // Se um deploy está em andamento, a tela se prende a ele ao abrir.
      if (inFlight.length === 1 && currentRun.current === 0) {
        currentRun.current = inFlight[0].runId;
        const detail = await api.deployRun(inFlight[0].runId).catch(() => null);
        if (detail && alive) {
          setRun(detail);
          setRunning(detail.status === "running");
        }
      }
    })();

    void (async () => {
      unstep = await onDeployStep((ev) => {
        if (ev.runId !== currentRun.current) return;
        setRun((actual) => {
          if (!actual || actual.id !== ev.runId) return actual;
          return {
            ...actual,
            steps: actual.steps.map((s) =>
              s.position === ev.position
                ? {
                    ...s,
                    status: ev.status,
                    command: ev.command ?? s.command,
                    exitCode: ev.exitCode ?? s.exitCode,
                    message: ev.message ?? s.message,
                  }
                : s,
            ),
          };
        });
      });
      unlog = await onDeployLog((ev) => {
        if (ev.runId !== currentRun.current) return;
        setLogs((actual) => {
          const next = [...actual, ev];
          return next.length > LOG_LIMIT ? next.slice(next.length - LOG_LIMIT) : next;
        });
      });
      undone = await onDeployDone((finished) => {
        if (finished.id === currentRun.current) {
          setRun(finished);
          setRunning(false);
          setStopping(false);
        }
        void loadHistory();
        pushToast(
          finished.status === "success" ? "success" : "error",
          finished.status === "success"
            ? `Deploy #${finished.id} concluído${finished.serverName ? ` em ${finished.serverName}` : " nesta máquina"}.`
            : `Deploy #${finished.id} terminou como “${finished.status}”: ${finished.detail ?? "sem detalhes"}`,
        );
      });
      unapr = await onPipelineApproval((pedido) => {
        // Um nó do grafo parou esperando decisão. O pedido é mostrado seja qual
        // for a execução na tela: sem ele o motor fica à espera até o prazo.
        setAprovacao(pedido);
        setAba("executar");
      });
    })();

    return () => {
      alive = false;
      unstep?.();
      unlog?.();
      undone?.();
      unapr?.();
    };
  }, [loadHistory, openRun, pushToast]);

  const check = async () => {
    if (!profile) return;
    setChecking(true);
    setPreflight(null);
    try {
      const result = await api.deployPreflight(profile.id);
      setPreflight(result);
      setAllowSecrets(false);
    } catch (e) {
      pushToast("error", `A pré-vistoria falhou: ${String(e)}`);
    } finally {
      setChecking(false);
    }
  };

  const pedirExecucao = () => {
    if (!profile) return;
    setSecretsFor({ kind: "deploy" });
  };

  // As quinze etapas do motor, com os nomes que o próprio motor usa neste modo.
  useEffect(() => {
    if (!profile) {
      setEngineSteps([]);
      return;
    }
    let alive = true;
    void api
      .pipelineEngineSteps(profile.id)
      .then((lista) => {
        if (alive) setEngineSteps(lista);
      })
      .catch((e) => {
        if (alive) pushToast("error", `Não foi possível ler as etapas do motor: ${String(e)}`);
      });
    return () => {
      alive = false;
    };
  }, [profile, pushToast]);

  const rodarNo = (nodeKey: string, comDependencias: boolean) => {
    setAba("executar");
    setSecretsFor({ kind: "no", nodeKey, comDependencias });
  };

  const executarNo = async (nodeKey: string, comDependencias: boolean, secretValues: Record<string, string>) => {
    if (!profile) return;
    try {
      const id = await api.pipelineRunNode({
        definitionId: profile.id,
        nodeKey,
        withDependencies: comDependencias,
        secretValues,
        allowSecrets,
      });
      setAllowSecrets(false);
      setLogs([]);
      await openRun(id);
      setRunning(true);
      await loadHistory();
    } catch (e) {
      pushToast("error", `Não foi possível executar o nó “${nodeKey}”: ${String(e)}`);
    }
  };

  const decidir = async (aprovado: boolean) => {
    if (!aprovacao) return;
    const pedido = aprovacao;
    setAprovacao(null);
    try {
      await api.pipelineApprove(pedido.id, aprovado);
      pushToast(
        aprovado ? "success" : "info",
        aprovado ? `Nó “${pedido.nodeKey}” aprovado.` : `Nó “${pedido.nodeKey}” rejeitado.`,
      );
    } catch (e) {
      pushToast("error", `A decisão não chegou ao motor: ${String(e)}`);
    }
  };

  const iniciar = async (secretValues: Record<string, string>) => {
    if (!profile) return;
    try {
      const id = await api.deployStart(profile.id, secretValues, allowSecrets);
      setAllowSecrets(false);
      setLogs([]);
      await openRun(id);
      setRunning(true);
      await loadHistory();
    } catch (e) {
      pushToast("error", `Não foi possível iniciar o deploy: ${String(e)}`);
    }
  };

  const reverter = async (runId: number, secretValues: Record<string, string>) => {
    try {
      const id = await api.deployRollback(runId, secretValues);
      setLogs([]);
      await openRun(id);
      setRunning(true);
      await loadHistory();
    } catch (e) {
      pushToast("error", `Não foi possível iniciar o rollback: ${String(e)}`);
    }
  };

  const pedirRepeticao = async (runId: number) => {
    try {
      const politica = await api.pipelineRetryPolicy(runId);
      if (politica.allowed) {
        setRetryBlock(null);
        setSecretsFor({ kind: "retry", runId });
      } else {
        setRetryBlock(politica.reason ?? `não é possível repetir a execução #${runId}`);
      }
    } catch (e) {
      pushToast("error", `Não foi possível consultar a política de tentativa: ${String(e)}`);
    }
  };

  const repetir = async (runId: number, secretValues: Record<string, string>) => {
    try {
      const id = await api.pipelineRetry(runId, secretValues);
      setRetryBlock(null);
      setLogs([]);
      await openRun(id);
      setRunning(true);
      await loadHistory();
    } catch (e) {
      pushToast("error", `Não foi possível repetir a execução: ${String(e)}`);
    }
  };

  const interromper = async () => {
    if (!run) return;
    setStopping(true);
    try {
      const label = await api.deployCancel(run.id);
      pushToast("info", `Interrompendo ${label}. A etapa em andamento termina antes de parar.`);
    } catch (e) {
      pushToast("error", String(e));
      setStopping(false);
    }
  };

  const recarregarModelos = async () => {
    setLoading(true);
    try {
      setProfiles(await api.listDeployProfiles(null));
    } catch (e) {
      pushToast("error", `Falha ao recarregar os modelos: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  const blocked = (preflight?.blocked ?? 0) > 0;
  const podePublicar = profile !== null && preflight !== null && (!blocked || allowSecrets);
  const motivoPublicar = !profile
    ? "Escolha um modelo de deploy"
    : !preflight
      ? "Rode a pré-vistoria antes de publicar"
      : blocked && !allowSecrets
        ? "Autorize os itens bloqueados para publicar"
        : running
          ? "Já existe uma execução em andamento"
          : "Publicar com este modelo";

  /** O modelo de quem a execução-alvo saiu — pode não ser o modelo escolhido na tela. */
  const modeloDoAlvo = (): DeployProfile | null => {
    if (!secretsFor) return profile;
    if (secretsFor.kind === "deploy" || secretsFor.kind === "no") return profile;
    const origem =
      runs.find((r) => r.id === secretsFor.runId) ?? (run?.id === secretsFor.runId ? run : null);
    if (!origem) return profile;
    return profiles.find((p) => p.id === origem.profileId) ?? profile;
  };

  if (loading && profiles.length === 0) {
    return (
      <div className="flex items-center gap-2 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner /> Carregando modelos de deploy…
      </div>
    );
  }

  if (profiles.length === 0) {
    return (
      <EmptyState
        icon={<RocketIcon className="size-8" />}
        title="Nenhum modelo de deploy cadastrado"
        description="Um modelo liga um projeto local a um destino — servidor SSH ou pasta desta máquina. Sem modelo não há o que publicar."
      >
        <Button variant="primary" onClick={() => onNavigate("projects")}>
          Cadastrar projeto e modelo
        </Button>
      </EmptyState>
    );
  }

  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div className="min-w-[16rem] flex-1">
            <SectionTitle>Publicar</SectionTitle>
            <Field label="Modelo de deploy">
              <Select
                value={profileId ?? 0}
                onChange={(e) => {
                  setProfileId(Number(e.target.value));
                  setPreflight(null);
                  setAllowSecrets(false);
                }}
              >
                {profiles.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name} · {p.serverName ?? DESTINO_LOCAL} · {STRATEGY_LABELS[p.strategy]}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          <div className="flex items-center gap-2">
            <Button onClick={() => void recarregarModelos()} title="Recarregar os modelos salvos">
              <RefreshIcon className="size-3.5" />
            </Button>
            <Button onClick={() => void check()} loading={checking} disabled={!profile || running}>
              Pré-vistoria
            </Button>
            // O motivo no pai, não no botão: um botão desabilitado tem
            // `pointer-events: none` e a dica de quem passa por cima nunca aparece.
            <span title={motivoPublicar} className="inline-flex">
              <Button variant="primary" disabled={!podePublicar || running} onClick={pedirExecucao}>
                <RocketIcon className="size-4" /> Publicar
              </Button>
            </span>
          </div>
        </div>

        {profile ? (
          <p className="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <Badge tone={ENV_TONES[profile.serverEnvironment ?? "other"] ?? "neutral"}>
              {profile.serverName ?? DESTINO_LOCAL}
            </Badge>
            <Badge>{STRATEGY_LABELS[profile.strategy]}</Badge>
            <Badge tone="neutral">v{profile.version}</Badge>
            <span className="font-mono text-zinc-500 dark:text-zinc-400">{profile.remotePath}</span>
            {profile.releasesEnabled ? <Badge tone="accent">releases + symlink</Badge> : null}
            {profile.deleteExtras ? <Badge tone="error">apaga excedentes</Badge> : null}
            {profile.secretKeys.length > 0 ? (
              <Badge tone="info">
                <KeyIcon className="size-3" /> {profile.secretKeys.length} variável(is)
              </Badge>
            ) : null}
          </p>
        ) : null}

        {preflight ? (
          <div className="mt-3 rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
            <p className="text-sm font-medium">{preflight.message}</p>
            <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
              {preflight.files} arquivo(s) · {formatBytes(preflight.bytes)} · {preflight.warnings} aviso(s) ·{" "}
              {preflight.blocked} bloqueio(s). A pré-vistoria só lê o disco local: nada foi enviado.
            </p>
            {preflight.sample.length > 0 ? (
              <p className="mt-1 truncate font-mono text-[11px] text-zinc-400 dark:text-zinc-500">
                {preflight.sample.join(" · ")}
              </p>
            ) : null}
            {preflight.findings.length > 0 ? (
              <ul className="mt-2 space-y-1">
                {preflight.findings.slice(0, 12).map((f) => (
                  <li key={`${f.path}-${f.kind}`} className="flex items-start gap-2 text-xs">
                    <AlertTriangleIcon
                      className={`mt-0.5 size-3.5 shrink-0 ${
                        f.severity === "block"
                          ? "text-red-600 dark:text-red-400"
                          : "text-amber-600 dark:text-amber-400"
                      }`}
                    />
                    <span>
                      <span className="font-mono">{f.path}</span> — {f.reason} {f.suggestion}
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
            {blocked ? (
              <div className="mt-3 border-t border-zinc-200 pt-3 dark:border-zinc-800">
                <Checkbox
                  checked={allowSecrets}
                  onChange={setAllowSecrets}
                  label={
                    <span className="text-xs">
                      Li os itens acima e autorizo explicitamente publicar mesmo com arquivos sensíveis marcados.
                    </span>
                  }
                />
              </div>
            ) : null}
          </div>
        ) : null}

        {active.length > 0 ? (
          <p className="mt-2 text-[11px] text-amber-700 dark:text-amber-300">
            {active.map((a) => a.label).join(" · ")}
          </p>
        ) : null}
      </Card>

      <div className="flex items-center gap-1 border-b border-zinc-200 dark:border-zinc-800">
        {(["executar", "grafo"] as const).map((opcao) => (
          <button
            key={opcao}
            onClick={() => setAba(opcao)}
            className={`-mb-px border-b-2 px-3 py-1.5 text-sm capitalize transition ${
              aba === opcao
                ? "border-teal-500 font-medium text-teal-700 dark:text-teal-300"
                : "border-transparent text-zinc-500 hover:text-zinc-800 dark:text-zinc-400 dark:hover:text-zinc-200"
            }`}
          >
            {opcao}
          </button>
        ))}
      </div>

      <div className={aba === "grafo" ? "" : "hidden"}>
        {profile ? (
          <PipelineGraph
            definitionId={profile.id}
            hasServer={profile.serverId !== null}
            engineSteps={engineSteps}
            runSteps={run?.steps ?? []}
            running={running}
            onRunNode={rodarNo}
            onSaved={() => void recarregarModelos()}
          />
        ) : null}
      </div>

      {aba === "executar" ? (
        <>
      {run ? (
        <Card className="p-4">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="min-w-0">
              <SectionTitle>
                Execução #{run.id} · {run.projectName} → {run.serverName ?? DESTINO_LOCAL}
              </SectionTitle>
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                {STRATEGY_LABELS[run.strategy] ?? run.strategy} · iniciada {formatDateTime(run.startedAt)}
                {run.finishedAt ? ` · terminada ${formatDateTime(run.finishedAt)}` : ""}
                {run.releaseDir ? (
                  <span className="font-mono"> · ativo {run.releaseDir}</span>
                ) : null}
                {run.previousRelease ? <span className="font-mono"> · anterior {run.previousRelease}</span> : null}
                {run.previousImage ? (
                  <span className="font-mono"> · imagem anterior {run.previousImage}</span>
                ) : null}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <StatusBadge status={run.status} />
              {running ? (
                <Button variant="danger" size="sm" loading={stopping} onClick={() => void interromper()}>
                  Interromper
                </Button>
              ) : (
                <Button
                  size="sm"
                  title="Reexecutar a mesma definição, quando o destino não foi alterado"
                  onClick={() => void pedirRepeticao(run.id)}
                >
                  <RefreshIcon className="size-3.5" /> Repetir
                </Button>
              )}
            </div>
          </div>

          <p className="mt-1 flex flex-wrap gap-x-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            <span>
              {run.definitionVersion
                ? `definição v${run.definitionVersion}`
                : "definição anterior à primeira versão registrada"}
            </span>
            {run.steps.some((s) => s.nodeKey) ? (
              <span>· grafo com {run.steps.filter((s) => s.nodeKey).length} nó(s)</span>
            ) : null}
            {run.environment ? <span>· {run.environment}</span> : null}
            {run.destination ? <span className="font-mono">· {run.destination}</span> : null}
            {run.commitSha ? (
              <span className="font-mono">· commit {run.commitSha.slice(0, 8)}</span>
            ) : null}
          </p>

          {retryBlock ? (
            <p className="mt-2 flex items-start gap-2 rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-700/60 dark:bg-amber-900/20 dark:text-amber-200">
              <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />
              <span>{retryBlock}</span>
            </p>
          ) : null}

          {run.detail ? (
            <p className="mt-2 rounded-lg bg-zinc-100 px-3 py-2 text-xs dark:bg-zinc-800/60">{run.detail}</p>
          ) : null}
          <div className="mt-3">
            <StepPipeline steps={run.steps} />
          </div>
        </Card>
      ) : null}

      {logs.length > 0 ? (
        <Card className="overflow-hidden">
          <div className="border-b border-zinc-200 px-4 py-2 dark:border-zinc-800">
            <SectionTitle>Log da execução</SectionTitle>
          </div>
          <div className="max-h-64 overflow-y-auto bg-zinc-950 px-4 py-3 font-mono text-[11px] leading-relaxed text-zinc-300">
            {logs.map((line, index) => (
              <p key={index} className={line.stream === "err" ? "text-red-300" : undefined}>
                <span className="select-none text-zinc-600">{String(line.position).padStart(2, "0")}</span>{" "}
                {line.text}
              </p>
            ))}
          </div>
          <p className="px-4 py-2 text-[11px] text-zinc-400 dark:text-zinc-500">
            As linhas passam por máscara antes de chegar aqui: os valores das variáveis do modelo nunca aparecem.
          </p>
        </Card>
      ) : null}

      <Card className="p-4">
        <div className="flex items-center justify-between">
          <SectionTitle>Deploys registrados</SectionTitle>
          <Button size="sm" variant="ghost" onClick={() => void loadHistory()}>
            <RefreshIcon className="size-3.5" /> Atualizar
          </Button>
        </div>
        {runs.length === 0 ? (
          <p className="text-xs text-zinc-500 dark:text-zinc-400">
            Nenhuma execução ainda. Cada deploy guarda as quinze etapas com comando, código de saída e mensagem.
          </p>
        ) : (
          <ul className="mt-2 divide-y divide-zinc-200 dark:divide-zinc-800">
            {runs.map((r) => (
              <li key={r.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                <button
                  onClick={() => void openRun(r.id)}
                  className="min-w-0 flex-1 text-left text-sm transition hover:text-teal-700 dark:hover:text-teal-300"
                >
                  <span className="font-medium">#{r.id}</span> {r.projectName} → {r.serverName ?? DESTINO_LOCAL}
                  <span className="block truncate text-[11px] text-zinc-500 dark:text-zinc-400">
                    {formatDateTime(r.startedAt)} · {STRATEGY_LABELS[r.strategy] ?? r.strategy}
                    {r.definitionVersion ? ` · v${r.definitionVersion}` : ""}
                    {r.commitSha ? ` · ${r.commitSha.slice(0, 8)}` : ""}
                    {r.detail ? ` · ${r.detail}` : ""}
                  </span>
                </button>
                <StatusBadge status={r.status} />
                {r.status === "success" || r.status === "failed" ? (
                  <Button
                    size="sm"
                    variant="danger"
                    title="Voltar ao estado anterior registrado desta execução"
                    onClick={() => setSecretsFor({ kind: "rollback", runId: r.id })}
                  >
                    <UndoIcon className="size-3.5" /> Reverter
                  </Button>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </Card>
        </>
      ) : null}

      {aprovacao ? (
        <Modal
          open
          onClose={() => setAprovacao(null)}
          title={`Aprovação · nó “${aprovacao.nodeKey}” · execução #${aprovacao.runId}`}
          width="max-w-md"
        >
          <div className="space-y-1.5 text-sm">
            <p className="font-medium">{aprovacao.title}</p>
            <p className="text-xs text-zinc-500 dark:text-zinc-400">{aprovacao.definition}</p>
            <p className="flex flex-wrap gap-2 pt-1 text-xs">
              <Badge tone="info">{aprovacao.environment}</Badge>
              <span className="font-mono text-zinc-500 dark:text-zinc-400">{aprovacao.destination}</span>
            </p>
          </div>
          <div className="mt-4 flex justify-end gap-2">
            <Button variant="danger" onClick={() => void decidir(false)}>
              <XIcon className="size-3.5" /> Rejeitar
            </Button>
            <Button variant="primary" onClick={() => void decidir(true)}>
              <CheckIcon className="size-3.5" /> Aprovar
            </Button>
          </div>
        </Modal>
      ) : null}

      {secretsFor ? (
        <SecretsDialog
          keys={modeloDoAlvo()?.secretKeys ?? []}
          local={modeloDoAlvo()?.serverName == null}
          title={
            secretsFor.kind === "rollback"
              ? `Variáveis do rollback · execução #${secretsFor.runId}`
              : secretsFor.kind === "retry"
                ? `Variáveis da nova tentativa · execução #${secretsFor.runId}`
                : secretsFor.kind === "no"
                  ? `Variáveis do nó “${secretsFor.nodeKey}”`
                  : `Variáveis do deploy · ${profile?.name ?? ""}`
          }
          actionLabel={
            secretsFor.kind === "rollback"
              ? "Reverter"
              : secretsFor.kind === "retry"
                ? "Repetir"
                : secretsFor.kind === "no"
                  ? "Executar nó"
                  : "Publicar"
          }
          onClose={() => setSecretsFor(null)}
          onSubmit={(values) => {
            const alvo = secretsFor;
            setSecretsFor(null);
            if (alvo.kind === "rollback") void reverter(alvo.runId, values);
            else if (alvo.kind === "retry") void repetir(alvo.runId, values);
            else if (alvo.kind === "no") void executarNo(alvo.nodeKey, alvo.comDependencias, values);
            else void iniciar(values);
          }}
        />
      ) : null}
    </div>
  );
}

function StatusBadge({ status }: { status: DeployRun["status"] }) {
  const tone: BadgeTone =
    status === "success" ? "success" : status === "failed" ? "error" : status === "cancelled" ? "warning" : "info";
  const label =
    status === "success"
      ? "sucesso"
      : status === "failed"
        ? "falhou"
        : status === "cancelled"
          ? "interrompido"
          : "em andamento";
  return <Badge tone={tone}>{label}</Badge>;
}

/**
 * Coleta os valores das variáveis do modelo. O estado vive só nesta janela:
 * nada é gravado em disco, em log ou no cadastro.
 */
function SecretsDialog({
  keys,
  title,
  actionLabel,
  local,
  onClose,
  onSubmit,
}: {
  keys: string[];
  title: string;
  actionLabel: string;
  local: boolean;
  onClose: () => void;
  onSubmit: (values: Record<string, string>) => void;
}) {
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(keys.map((k) => [k, ""])),
  );

  const executar = () => {
    const limpos: Record<string, string> = {};
    for (const key of keys) {
      const value = (values[key] ?? "").trim();
      if (value !== "") limpos[key] = value;
    }
    onSubmit(limpos);
  };

  return (
    <Modal open onClose={onClose} title={title} width="max-w-md">
      {keys.length === 0 ? (
        <p className="text-sm text-zinc-600 dark:text-zinc-300">
          Este modelo não declara variáveis. Confirme para executar as etapas.
        </p>
      ) : (
        <div className="space-y-2.5">
          <p className="text-xs text-zinc-500 dark:text-zinc-400">
            {local
              ? "Os valores ficam apenas nesta janela e na memória da execução; entram pelo ambiente dos comandos desta máquina e não são gravados em lugar nenhum."
              : "Os valores ficam apenas nesta janela e na memória da execução; vão ao servidor dentro do script enviado pela stdin do SSH e não são gravados em lugar nenhum."}
          </p>
          {keys.map((key) => (
            <Field key={key} label={key}>
              <Input
                type="password"
                spellCheck={false}
                autoComplete="off"
                value={values[key] ?? ""}
                onChange={(e) => setValues((prev) => ({ ...prev, [key]: e.target.value }))}
              />
            </Field>
          ))}
        </div>
      )}
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose}>Cancelar</Button>
        <Button variant="primary" onClick={executar}>
          {actionLabel}
        </Button>
      </div>
    </Modal>
  );
}
