import { useCallback, useEffect, useState } from "react";
import { api, onTransferProgress } from "../lib/api";
import { formatBytes, formatDateTime, formatEta, formatRate, formatRelative, remoteBasename } from "../lib/format";
import { useInfraMonitor, type Leitura } from "../lib/infraMonitor";
import type {
  AuditEntry,
  DeployActive,
  DeployRun,
  Project,
  Server,
  ToolVersions,
  TransferInfo,
  TransferProgress,
  ViewId,
} from "../types";
import { useApp } from "../store/AppContext";
import { InfraPanel } from "../components/InfraPanel";
import {
  AlertTriangleIcon,
  CheckIcon,
  CloudIcon,
  DownloadIcon,
  FolderIcon,
  GitBranchIcon,
  HistoryIcon,
  InfoIcon,
  KeyIcon,
  PlusIcon,
  RefreshIcon,
  RocketIcon,
  ServerIcon,
  TerminalIcon,
  UploadIcon,
} from "../components/Icons";
import {
  Badge,
  type BadgeTone,
  Button,
  Card,
  DESTINO_LOCAL,
  EmptyState,
  LEVEL_LABELS,
  LEVEL_TONES,
  SectionTitle,
  Spinner,
  STRATEGY_LABELS,
} from "../components/ui";

const RUN_TONES: Record<DeployRun["status"], BadgeTone> = {
  running: "info",
  success: "success",
  failed: "error",
  cancelled: "warning",
};

const RUN_LABELS: Record<DeployRun["status"], string> = {
  running: "em andamento",
  success: "sucesso",
  failed: "falhou",
  cancelled: "cancelado",
};

type Alerta = {
  nivel: "error" | "warning" | "info";
  texto: string;
  acao?: { label: string; view?: ViewId; anchor?: string };
};

/**
 * Alertas derivados do que já está no banco local, das ferramentas realmente
 * detectadas e das leituras de métricas desta sessão. Nada aqui é suposição:
 * cada linha aponta um estado verificável.
 */
function montarAlertas(
  servers: Server[],
  tools: ToolVersions | null,
  runs: DeployRun[] | null,
  projects: Project[],
  readings: Record<number, Leitura>,
): Alerta[] {
  const alertas: Alerta[] = [];

  if (servers.length === 0) {
    alertas.push({
      nivel: "info",
      texto: "Nenhum servidor cadastrado — importe os acessos do ~/.ssh/config para começar.",
      acao: { label: "Servidores", view: "servers" },
    });
  }
  if (tools && !tools.ssh) {
    alertas.push({
      nivel: "error",
      texto: "OpenSSH (ssh) não foi encontrado no PATH. Terminal, arquivos remotos e deploy dependem dele.",
      acao: { label: "Ver ferramentas", view: "settings" },
    });
  }
  if (tools && !tools.git) {
    alertas.push({
      nivel: "error",
      texto: "Git não foi encontrado no PATH. O gerenciador Git e o deploy por Git ficam indisponíveis.",
      acao: { label: "Ver ferramentas", view: "settings" },
    });
  }
  if (tools && tools.git && tools.gitUser == null) {
    alertas.push({
      nivel: "warning",
      texto:
        "O git não consegue resolver a identidade (user.name/user.email) desta máquina — o próximo commit de qualquer projeto falha com “Please tell me who you are”.",
      acao: { label: "Acesso da máquina", view: "settings" },
    });
  }
  if (tools && tools.agentRunning !== true && tools.ghAccount == null) {
    alertas.push({
      nivel: "warning",
      texto:
        "Nenhuma liberação detectada nesta máquina: o ssh-agent não entrega chaves e o gh não tem sessão. " +
        "Remoto privado não autentica até liberar o acesso (chave no agente, gh auth login ou o login do GitHub).",
      acao: { label: "Acesso da máquina", view: "settings" },
    });
  } else if (tools && (tools.agentKeys ?? 0) === 0) {
    alertas.push({
      nivel: "warning",
      texto: "O ssh-agent está rodando, mas sem nenhuma chave carregada — os remotos SSH vão recusar a conexão.",
      acao: { label: "Acesso da máquina", view: "settings" },
    });
  }

  for (const s of servers) {
    if (s.lastStatus === "failed") {
      alertas.push({
        nivel: "error",
        texto: `Último teste de "${s.name}" falhou: ${s.lastStatusDetail ?? "sem detalhe registrado"}.`,
        acao: { label: "Rever acesso", view: "servers" },
      });
    } else if (!s.lastCheckedAt) {
      alertas.push({
        nivel: "info",
        texto: `"${s.name}" ainda nunca foi testado — a chave de host só é validada na primeira conexão.`,
        acao: { label: "Testar conexão", view: "servers" },
      });
    }
    if (s.authMethod === "password") {
      alertas.push({
        nivel: "warning",
        texto: `"${s.name}" usa senha como autenticação. Uma chave SSH é mais segura e evita senha a cada operação.`,
        acao: { label: "Configurar chave", view: "servers" },
      });
    }
    if (s.authMethod === "key" && !s.identityFile) {
      alertas.push({
        nivel: "warning",
        texto: `"${s.name}" está como chave, mas nenhum arquivo de identidade foi indicado.`,
        acao: { label: "Rever acesso", view: "servers" },
      });
    }

    const leitura = readings[s.id];
    if (leitura?.error) {
      alertas.push({
        nivel: "warning",
        texto: `A leitura de métricas de "${s.name}" não concluiu: ${leitura.error}`,
        acao: { label: "Ver leitura", anchor: "infra" },
      });
    } else if (leitura?.metrics && leitura.metrics.warnings.length > 0) {
      const extras = leitura.metrics.warnings.length - 1;
      alertas.push({
        nivel: "warning",
        texto: `"${s.name}": ${leitura.metrics.warnings[0]}${extras > 0 ? ` (+${extras} outro(s) alerta(s))` : ""}.`,
        acao: { label: "Ver leitura", anchor: "infra" },
      });
    }
  }

  for (const p of projects) {
    if (!p.isGitRepo) {
      alertas.push({
        nivel: "warning",
        texto: `O projeto "${p.name}" não é um repositório Git — o gerenciador de commits não opera nele.`,
        acao: { label: "Projetos", view: "projects" },
      });
      continue;
    }
    if (!p.gitRemote) {
      alertas.push({
        nivel: "info",
        texto: `"${p.name}" não tem remoto configurado: push e fetch não têm destino.`,
        acao: { label: "Abrir Git", view: "git" },
      });
    }
    if (p.defaultServerId == null) {
      alertas.push({
        nivel: "info",
        texto: `"${p.name}" não tem servidor padrão — o deploy precisa de um alvo escolhido.`,
        acao: { label: "Projetos", view: "projects" },
      });
    }
  }

  const ultimo = runs?.[0];
  if (ultimo && ultimo.status === "failed") {
    alertas.push({
      nivel: "error",
      texto: `O último deploy (${ultimo.projectName} → ${ultimo.serverName ?? DESTINO_LOCAL}) falhou: ${
        ultimo.detail ?? "sem detalhe registrado"
      }. Nada foi marcado como publicado.`,
      acao: { label: "Ver deploy", view: "deploy" },
    });
  }

  // O painel mostra os primeiros: o que é erro precisa vir antes de aviso e de
  // informação, sem perder a ordem em que os fatos foram verificados.
  const peso: Record<Alerta["nivel"], number> = { error: 0, warning: 1, info: 2 };
  return alertas.sort((a, b) => peso[a.nivel] - peso[b.nivel]);
}

export function DashboardView({ onNavigate }: { onNavigate: (v: ViewId) => void }) {
  const { servers, projects, pushToast } = useApp();
  const { readings } = useInfraMonitor();
  const [recent, setRecent] = useState<AuditEntry[] | null>(null);
  const [failures, setFailures] = useState<AuditEntry[] | null>(null);
  const [runs, setRuns] = useState<DeployRun[] | null>(null);
  const [inFlight, setInFlight] = useState<DeployActive[]>([]);
  const [transfers, setTransfers] = useState<TransferInfo[]>([]);
  const [progress, setProgress] = useState<Record<number, TransferProgress>>({});
  const [tools, setTools] = useState<ToolVersions | null>(null);
  const [reloading, setReloading] = useState(false);

  const reload = useCallback(async () => {
    setReloading(true);
    try {
      const [atividade, falhas, execuções, versões, emAndamento, transferencias] = await Promise.all([
        api.queryAuditLog({ limit: 8, offset: 0 }),
        api.queryAuditLog({ limit: 5, level: "error", offset: 0 }),
        api.deployRuns(null, 4),
        api.getToolVersions(),
        api.deployActive().catch(() => [] as DeployActive[]),
        api.transfersList().catch(() => [] as TransferInfo[]),
      ]);
      setRecent(atividade);
      setFailures(falhas);
      setRuns(execuções);
      setTools(versões);
      setInFlight(emAndamento);
      setTransfers(transferencias);
    } catch (e) {
      pushToast("error", `Falha ao atualizar o painel: ${String(e)}`);
    } finally {
      setReloading(false);
    }
  }, [pushToast]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    onTransferProgress((p) => {
      setProgress((prev) => ({ ...prev, [p.transferId]: p }));
      if (p.done) {
        // A lista do backend é a fonte: ao terminar, ela já não traz a
        // transferência, então o painel só precisa reler.
        void api
          .transfersList()
          .then(setTransfers)
          .catch(() => undefined);
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  const okCount = servers.filter((s) => s.lastStatus === "ok").length;
  const failedCount = servers.filter((s) => s.lastStatus === "failed").length;
  const gitCount = projects.filter((p) => p.isGitRepo).length;
  const alertas = montarAlertas(servers, tools, runs, projects, readings);
  const ultimo = runs?.[0] ?? null;
  const rodando = transfers.length + inFlight.length;
  const lidosMetricas = servers.filter((s) => readings[s.id]?.metrics).length;
  const emAlertaMetricas = servers.filter((s) => (readings[s.id]?.metrics?.warnings.length ?? 0) > 0).length;
  const ultimaLeitura = Math.max(0, ...Object.values(readings).map((l) => l.readAt ?? 0)) || null;

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-4 xl:grid-cols-4">
        <Card className="p-4">
          <p className="text-xs text-zinc-500 dark:text-zinc-400">Servidores cadastrados</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{servers.length}</p>
          <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">
            {okCount} com teste bem-sucedido · {failedCount} com falha
          </p>
          <p className="mt-0.5 text-[11px] text-zinc-500 dark:text-zinc-400">
            {lidosMetricas > 0
              ? `${lidosMetricas} com métricas · ${emAlertaMetricas} em alerta`
              : "Nenhuma leitura de métricas nesta sessão"}
          </p>
        </Card>
        <Card className="p-4">
          <p className="text-xs text-zinc-500 dark:text-zinc-400">Projetos locais</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{projects.length}</p>
          <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">{gitCount} com repositório Git detectado</p>
        </Card>
        <Card className="p-4">
          <p className="text-xs text-zinc-500 dark:text-zinc-400">Última conexão bem-sucedida</p>
          <p className="mt-1 truncate text-sm font-medium">
            {servers.filter((s) => s.lastOkAt).length === 0
              ? "—"
              : formatDateTime(
                  servers
                    .filter((s) => s.lastOkAt)
                    .map((s) => s.lastOkAt as string)
                    .sort()
                    .at(-1) as string,
                )}
          </p>
          <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">
            {ultimaLeitura
              ? `Métricas lidas ${formatRelative(Math.floor(ultimaLeitura / 1000))} · nada é consultado sozinho.`
              : "Métricas de CPU, memória e disco são lidas do servidor quando você pedir."}
          </p>
        </Card>
        <Card className="p-4">
          <p className="flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
            Em andamento
            {reloading ? <Spinner className="size-3" /> : null}
          </p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{rodando}</p>
          <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">
            {transfers.length} transferência(s) · {inFlight.length} deploy(s)
          </p>
        </Card>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <div className="space-y-4 lg:col-span-2">
          <Card className="p-4">
            <div className="flex items-center gap-2">
              <SectionTitle>Servidores e infraestrutura</SectionTitle>
              <Button
                size="sm"
                variant="ghost"
                className="ml-auto"
                loading={reloading}
                onClick={() => void reload()}
                title="Reler dados locais e operações em andamento"
              >
                <RefreshIcon className="size-3.5" /> Atualizar
              </Button>
            </div>
            {servers.length === 0 ? (
              <EmptyState
                icon={<ServerIcon className="size-8" />}
                title="Nenhum servidor cadastrado"
                description="Importe os acessos existentes do seu ~/.ssh/config ou cadastre manualmente."
              >
                <Button variant="primary" onClick={() => onNavigate("servers")}>
                  Ir para Servidores
                </Button>
              </EmptyState>
            ) : (
              <InfraPanel servers={servers} />
            )}
          </Card>

          <Card className="p-4">
            <SectionTitle>Deploys</SectionTitle>
            {runs === null ? (
              <p className="py-3 text-sm text-zinc-500">Consultando o histórico local…</p>
            ) : runs.length === 0 ? (
              <p className="py-2 text-sm text-zinc-500 dark:text-zinc-400">
                Nenhum deploy registrado ainda. Cadastre um perfil e publique um projeto para ver o resultado aqui.
              </p>
            ) : (
              <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
                {runs.map((r) => (
                  <li key={r.id} className="py-2">
                    <div className="flex flex-wrap items-center gap-2 text-sm">
                      <Badge tone={RUN_TONES[r.status]}>{RUN_LABELS[r.status]}</Badge>
                      <span className="font-medium">{r.projectName}</span>
                      <span className="text-xs text-zinc-500 dark:text-zinc-400">
                        → {r.serverName ?? DESTINO_LOCAL}
                      </span>
                      <span className="text-[11px] text-zinc-500 dark:text-zinc-400">
                        {STRATEGY_LABELS[r.strategy] ?? r.strategy}
                      </span>
                      <span className="ml-auto shrink-0 text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                        {formatDateTime(r.startedAt)}
                      </span>
                    </div>
                    {r.detail ? (
                      <p
                        className={`mt-1 truncate text-[11px] ${
                          r.status === "failed" ? "text-red-600 dark:text-red-400" : "text-zinc-500 dark:text-zinc-400"
                        }`}
                        title={r.detail}
                      >
                        {r.detail}
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
            {ultimo ? (
              <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
                Último resultado registrado: {RUN_LABELS[ultimo.status]} em {formatDateTime(ultimo.finishedAt ?? ultimo.startedAt)}
                {ultimo.releaseDir ? ` · release ${ultimo.releaseDir}` : ""}
              </p>
            ) : null}
          </Card>
        </div>

        <div className="space-y-4">
          <Card className="p-4">
            <SectionTitle>Alertas de configuração</SectionTitle>
            {alertas.length === 0 ? (
              <p className="flex items-start gap-1.5 text-xs text-emerald-700 dark:text-emerald-400">
                <CheckIcon className="mt-0.5 size-3.5 shrink-0" />
                Nenhum alerta. Ferramentas, acessos e projetos verificados até agora sem inconsistência.
              </p>
            ) : (
              <ul className="space-y-2">
                {alertas.slice(0, 6).map(({ texto, nivel, acao }) => (
                  <li key={texto} className="text-xs">
                    <p className="flex items-start gap-1.5">
                      {nivel === "error" ? (
                        <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0 text-red-500" />
                      ) : nivel === "warning" ? (
                        <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0 text-amber-500" />
                      ) : (
                        <InfoIcon className="mt-0.5 size-3.5 shrink-0 text-zinc-400" />
                      )}
                      <span className="text-zinc-700 dark:text-zinc-300">{texto}</span>
                    </p>
                    {acao ? (
                      <button
                        type="button"
                        className="mt-0.5 text-[11px] font-medium text-teal-700 underline-offset-2 hover:underline dark:text-teal-300"
                        onClick={() => {
                          if (acao.view) onNavigate(acao.view);
                          if (acao.anchor) {
                            document.getElementById(acao.anchor)?.scrollIntoView({ behavior: "smooth", block: "start" });
                          }
                        }}
                      >
                        {acao.label}
                      </button>
                    ) : null}
                  </li>
                ))}
                {alertas.length > 6 ? (
                  <li className="text-[11px] text-zinc-500 dark:text-zinc-400">+{alertas.length - 6} alerta(s)</li>
                ) : null}
              </ul>
            )}
          </Card>

          <Card className="p-4">
            <SectionTitle>Transferências em andamento</SectionTitle>
            {transfers.length === 0 ? (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                Nenhuma cópia de arquivos ativa agora.
              </p>
            ) : (
              <ul className="space-y-2">
                {transfers.map((t) => {
                  const p = progress[t.id];
                  const arquivo = p?.fileName || remoteBasename(t.kind === "upload" ? t.remotePath : t.localPath);
                  return (
                    <li key={t.id} className="text-xs">
                      <p className="flex items-center gap-1.5">
                        {t.kind === "upload" ? (
                          <UploadIcon className="size-3.5 shrink-0 text-teal-600 dark:text-teal-400" />
                        ) : (
                          <DownloadIcon className="size-3.5 shrink-0 text-sky-600 dark:text-sky-400" />
                        )}
                        <span className="truncate font-medium">{arquivo}</span>
                        <span className="ml-auto shrink-0 tabular-nums text-zinc-500 dark:text-zinc-400">
                          {p ? `${p.pct.toFixed(0)}%` : "…"}
                        </span>
                      </p>
                      <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800">
                        <div
                          className="h-full rounded-full bg-teal-500"
                          style={{ width: `${Math.max(2, p?.pct ?? 0)}%` }}
                        />
                      </div>
                      <p className="mt-0.5 truncate text-[11px] text-zinc-500 dark:text-zinc-400">
                        {t.serverName} · {p ? `${formatBytes(p.transferred)} de ${formatBytes(p.total)}` : "aguardando progresso"}
                        {p && p.rate ? ` · ${formatRate(p.rate)}` : ""}
                        {p && p.eta ? ` · restam ${formatEta(p.eta)}` : ""}
                        {p?.error ? ` · ${p.error}` : ""}
                      </p>
                    </li>
                  );
                })}
              </ul>
            )}
            <Button size="sm" className="mt-2" onClick={() => onNavigate("files")}>
              <FolderIcon className="size-3.5" /> Gerenciar arquivos
            </Button>
          </Card>

          <Card className="p-4">
            <SectionTitle>Falhas recentes</SectionTitle>
            {failures === null ? (
              <p className="text-xs text-zinc-500">Consultando o histórico…</p>
            ) : failures.length === 0 ? (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                Nenhuma falha registrada. Toda operação — inclusive as que dão errado — fica no histórico.
              </p>
            ) : (
              <ul className="space-y-1.5">
                {failures.map((e) => (
                  <li key={e.id} className="text-xs">
                    <p className="flex items-center gap-1.5">
                      <Badge tone={LEVEL_TONES[e.level] ?? "neutral"}>{LEVEL_LABELS[e.level] ?? e.level}</Badge>
                      <span className="truncate font-medium">{e.operation}</span>
                      <span className="ml-auto shrink-0 text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                        {formatDateTime(e.ts)}
                      </span>
                    </p>
                    {e.detail ? (
                      <p className="mt-0.5 truncate text-[11px] text-zinc-500 dark:text-zinc-400" title={e.detail}>
                        {[e.serverName, e.projectName, e.detail].filter(Boolean).join(" · ")}
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
            <Button size="sm" className="mt-2" onClick={() => onNavigate("history")}>
              <HistoryIcon className="size-3.5" /> Ver histórico completo
            </Button>
          </Card>
        </div>
      </div>

      <Card className="p-4">
        <SectionTitle>Atalhos</SectionTitle>
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" onClick={() => onNavigate("servers")}>
            <PlusIcon /> Adicionar servidor
          </Button>
          <Button onClick={() => onNavigate("servers")}>
            <KeyIcon /> Importar de ~/.ssh/config
          </Button>
          <Button onClick={() => onNavigate("projects")}>
            <GitBranchIcon /> Adicionar projeto
          </Button>
          <Button onClick={() => onNavigate("terminal")}>
            <TerminalIcon /> Abrir terminal
          </Button>
          <Button onClick={() => onNavigate("files")}>
            <UploadIcon /> Explorar arquivos remotos
          </Button>
          <Button onClick={() => onNavigate("deploy")}>
            <RocketIcon /> Publicar um projeto
          </Button>
          <Button onClick={() => onNavigate("git")}>
            <GitBranchIcon /> Abrir o gerenciador Git
          </Button>
          <Button onClick={() => onNavigate("github")}>
            <CloudIcon /> Conectar ao GitHub
          </Button>
          <Button onClick={() => onNavigate("history")}>
            <ServerIcon /> Ver histórico
          </Button>
        </div>
      </Card>

      <Card className="p-4">
        <SectionTitle>Atividade registrada</SectionTitle>
        {recent === null ? (
          <p className="py-4 text-sm text-zinc-500">Carregando…</p>
        ) : recent.length === 0 ? (
          <p className="py-4 text-sm text-zinc-500 dark:text-zinc-400">
            Nenhum evento registrado ainda. As operações da aplicação aparecem aqui automaticamente.
          </p>
        ) : (
          <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
            {recent.map((e) => (
              <li key={e.id} className="flex items-center gap-3 py-2 text-sm">
                <Badge tone={LEVEL_TONES[e.level] ?? "neutral"}>{LEVEL_LABELS[e.level] ?? e.level}</Badge>
                <span className="font-medium">{e.operation}</span>
                <span className="truncate text-xs text-zinc-500 dark:text-zinc-400">
                  {[e.serverName, e.projectName].filter(Boolean).join(" · ")}
                </span>
                <span className="ml-auto shrink-0 text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                  {formatDateTime(e.ts)}
                </span>
              </li>
            ))}
          </ul>
        )}
        <div className="mt-3 flex flex-wrap gap-2">
          <Button size="sm" onClick={() => onNavigate("history")}>
            <HistoryIcon className="size-3.5" /> Abrir histórico e auditoria
          </Button>
          <Button size="sm" onClick={() => onNavigate("settings")}>
            Configurações e ferramentas
          </Button>
        </div>
      </Card>

      <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
        Datas em horário local · dados armazenados apenas neste computador (SQLite) · métricas de servidor lidas na
        hora, sob pedido · nenhuma informação simulada.
      </p>
    </div>
  );
}
