import { useCallback, useEffect, useMemo, useState } from "react";
import { formatDateTime, formatKilobytes, formatRelative, formatUptime } from "../lib/format";
import { lerFrota, lerServidor, pararFrota, useInfraMonitor, type Leitura } from "../lib/infraMonitor";
import type { Server, ServerMetrics } from "../types";
import {
  AlertTriangleIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  RefreshIcon,
  ServerIcon,
  XIcon,
} from "./Icons";
import { Badge, Button, ENV_LABELS, ENV_TONES, Select, StatusDot } from "./ui";

// Os mesmos limiares do backend (infra.rs): 85% de disco, 90% de memória e
// carga de 1,5× o número de núcleos. A cor repete a regra, não inventa dado.
const LIMIAR_DISCO = 85;
const LIMIAR_MEMORIA = 90;
const LIMIAR_CARGA = 1.5;
const LEITURA_VALIDA_MS = 10 * 60 * 1000;

type Ordem = "inventario" | "alerta" | "antiga";

function memoriaPct(m: ServerMetrics): number | null {
  if (!m.memTotalKb || m.memTotalKb <= 0 || m.memAvailableKb == null) return null;
  return Math.round(((m.memTotalKb - m.memAvailableKb) * 100) / m.memTotalKb);
}

function piorDisco(m: ServerMetrics): ServerMetrics["disks"][number] | null {
  return m.disks.reduce<ServerMetrics["disks"][number] | null>(
    (pior, d) => (pior == null || d.usePercent > pior.usePercent ? d : pior),
    null,
  );
}

function cargaRelativa(m: ServerMetrics): number | null {
  if (m.load1 == null || !m.cpuCount || m.cpuCount <= 0) return null;
  return m.load1 / m.cpuCount;
}

function desatualizada(l: Leitura | undefined): boolean {
  return l?.readAt != null && Date.now() - l.readAt > LEITURA_VALIDA_MS;
}

function gravidade(l: Leitura | undefined): number {
  if (l?.error) return 3;
  if ((l?.metrics?.warnings.length ?? 0) > 0) return 2;
  if (l?.metrics) return 1;
  return 0;
}

function Barra({ pct, alerta, cuidado }: { pct: number; alerta: number; cuidado?: number }) {
  const limiteCuidado = cuidado ?? Math.max(0, alerta - 15);
  const tone = pct >= alerta ? "bg-red-500" : pct >= limiteCuidado ? "bg-amber-500" : "bg-teal-500";
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800">
      <div
        className={`h-full rounded-full ${tone}`}
        style={{ width: `${Math.min(100, Math.max(2, pct))}%` }}
      />
    </div>
  );
}

function Campo({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] text-zinc-500 dark:text-zinc-400">{label}</p>
      <p className="truncate text-xs font-medium" title={value}>
        {value}
      </p>
    </div>
  );
}

function Metrica({ m }: { m: ServerMetrics }) {
  const [todosServicos, setTodosServicos] = useState(false);
  const memUsadaPct = memoriaPct(m);
  const carga = cargaRelativa(m);
  const pctCarga = carga != null ? Math.round(carga * 100) : null;
  const discos = useMemo(() => [...m.disks].sort((a, b) => b.usePercent - a.usePercent), [m.disks]);
  const servicos = useMemo(() => {
    const ruim = (state: string) => /exit|dead|failed|stopped|unhealthy/i.test(state);
    return [...m.services].sort((a, b) => Number(ruim(b.state)) - Number(ruim(a.state)) || a.name.localeCompare(b.name));
  }, [m.services]);
  const visiveis = todosServicos ? servicos : servicos.slice(0, 12);
  const servicosRuins = servicos.filter((s) => /exit|dead|failed|stopped|unhealthy/i.test(s.state)).length;

  return (
    <div className="mt-2 space-y-3 rounded-lg border border-zinc-200 bg-zinc-50/60 p-3 dark:border-zinc-800 dark:bg-zinc-900/40">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Campo label="Hostname" value={m.hostname ?? "não informado"} />
        <Campo label="Sistema" value={[m.os, m.kernel, m.arch].filter(Boolean).join(" · ") || "não informado"} />
        <Campo label="Tempo ligado" value={m.uptimeSeconds != null ? formatUptime(m.uptimeSeconds) : "não informado"} />
        <Campo
          label="CPU"
          value={
            m.cpuCount != null
              ? `${m.cpuCount} núcleo(s)${m.cpuModel ? ` · ${m.cpuModel}` : ""}`
              : "não informado"
          }
        />
      </div>

      <div className="grid gap-3 md:grid-cols-2">
        <div>
          <p className="flex items-baseline justify-between gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            <span>Carga de trabalho</span>
            <span className="tabular-nums">
              {m.load1 != null || m.load5 != null || m.load15 != null
                ? `${m.load1 ?? "—"} / ${m.load5 ?? "—"} / ${m.load15 ?? "—"} (1, 5, 15 min)`
                : "não informado"}
            </span>
          </p>
          {pctCarga != null ? (
            <div className="mt-1">
              <Barra pct={pctCarga} alerta={Math.round(LIMIAR_CARGA * 100)} cuidado={100} />
              <p className="mt-0.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                {(pctCarga / 100).toFixed(2)}× {m.cpuCount} núcleo(s) — média de 1 min
              </p>
            </div>
          ) : null}
        </div>

        <div>
          <p className="flex items-baseline justify-between gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            <span>Memória</span>
            <span className="tabular-nums">
              {memUsadaPct != null
                ? `${formatKilobytes(m.memTotalKb && m.memAvailableKb != null ? m.memTotalKb - m.memAvailableKb : null)} de ${formatKilobytes(m.memTotalKb)}`
                : "não informado"}
            </span>
          </p>
          {memUsadaPct != null ? (
            <div className="mt-1">
              <Barra pct={memUsadaPct} alerta={LIMIAR_MEMORIA} />
              <p className="mt-0.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                disponível {formatKilobytes(m.memAvailableKb)}
                {m.swapTotalKb && m.swapTotalKb > 0 ? ` · swap ${formatKilobytes(m.swapFreeKb)} livres` : ""}
              </p>
            </div>
          ) : null}
        </div>
      </div>

      <div>
        <p className="text-[11px] text-zinc-500 dark:text-zinc-400">Discos</p>
        {discos.length === 0 ? (
          <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
            O servidor não informou nenhum sistema de arquivos de armazenamento.
          </p>
        ) : (
          <ul className="mt-1 space-y-1.5">
            {discos.map((d) => (
              <li key={d.mount} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
                <div className="min-w-0">
                  <p className="truncate font-mono text-[11px]" title={d.mount}>
                    {d.mount}
                  </p>
                  <div className="mt-1">
                    <Barra pct={d.usePercent} alerta={LIMIAR_DISCO} />
                  </div>
                </div>
                <p className="shrink-0 text-right text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                  {d.usePercent}% · {formatKilobytes(d.usedKb)} / {formatKilobytes(d.totalKb)} · {formatKilobytes(d.availKb)} livres
                </p>
              </li>
            ))}
          </ul>
        )}
      </div>

      {servicos.length > 0 ? (
        <div>
          <p className="flex items-center gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            Containers e serviços reportados pelo próprio servidor
            <span className="tabular-nums">{servicos.length}</span>
            {servicosRuins > 0 ? <Badge tone="error">{servicosRuins} fora do ar</Badge> : null}
          </p>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {visiveis.map((s) => (
              <Badge
                key={`${s.kind}:${s.name}`}
                tone={
                  /up|running|active/i.test(s.state)
                    ? "success"
                    : /exit|dead|failed|stopped|unhealthy/i.test(s.state)
                      ? "error"
                      : "neutral"
                }
              >
                {s.kind === "container" ? "docker" : "systemd"} · {s.name}: {s.state || "sem estado"}
              </Badge>
            ))}
          </div>
          {servicos.length > 12 ? (
            <Button size="sm" variant="ghost" className="mt-1" onClick={() => setTodosServicos((v) => !v)}>
              {todosServicos ? "Mostrar só os 12 primeiros" : `Mostrar todos os ${servicos.length}`}
            </Button>
          ) : null}
        </div>
      ) : null}

      {m.warnings.length > 0 ? (
        <ul className="space-y-1">
          {m.warnings.map((w) => (
            <li key={w} className="flex items-start gap-1.5 text-[11px] text-amber-700 dark:text-amber-400">
              <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {w}
            </li>
          ))}
        </ul>
      ) : null}

      {m.notes.length > 0 ? (
        <ul className="space-y-0.5">
          {m.notes.map((n) => (
            <li key={n} className="text-[11px] text-zinc-500 dark:text-zinc-400">
              • {n}
            </li>
          ))}
        </ul>
      ) : null}

      <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
        Lido em {formatDateTime(new Date(m.collectedAt).toISOString())} · somente comandos de leitura.
      </p>
    </div>
  );
}

export function InfraPanel({ servers }: { servers: Server[] }) {
  const { readings, fleet } = useInfraMonitor();
  const [aberto, setAberto] = useState<number | null>(null);
  const [ordem, setOrdem] = useState<Ordem>("inventario");
  const [soAlertas, setSoAlertas] = useState(false);
  const [, setTick] = useState(0);

  // Recalcula idade/desatualização a cada minuto sem consultar nada no servidor.
  useEffect(() => {
    const id = window.setInterval(() => setTick((t) => t + 1), 60_000);
    return () => window.clearInterval(id);
  }, []);

  const coletar = useCallback(async (server: Server) => {
    await lerServidor(server);
    setAberto(server.id);
  }, []);

  const lidos = servers.filter((s) => readings[s.id]?.metrics).length;
  const emAlerta = servers.filter((s) => (readings[s.id]?.metrics?.warnings.length ?? 0) > 0).length;
  const semResposta = servers.filter((s) => readings[s.id]?.error).length;
  const comSenha = servers.filter((s) => s.authMethod === "password").length;
  const atual = fleet.current != null ? servers.find((s) => s.id === fleet.current)?.name : null;

  const linhas = useMemo(() => {
    const filtrados = servers.filter((s) => !soAlertas || gravidade(readings[s.id]) >= 2);
    const copia = [...filtrados];
    if (ordem === "alerta") {
      copia.sort(
        (a, b) =>
          gravidade(readings[b.id]) - gravidade(readings[a.id]) ||
          (readings[b.id]?.metrics?.warnings.length ?? 0) - (readings[a.id]?.metrics?.warnings.length ?? 0) ||
          a.name.localeCompare(b.name),
      );
    } else if (ordem === "antiga") {
      copia.sort((a, b) => {
        const ta = readings[a.id]?.readAt ?? 0;
        const tb = readings[b.id]?.readAt ?? 0;
        return ta - tb || a.name.localeCompare(b.name);
      });
    }
    return copia;
  }, [servers, readings, ordem, soAlertas]);

  const resumoFrota = fleet.active
    ? `${fleet.done}/${fleet.total} lidos${atual ? ` · agora ${atual}` : ""}${fleet.failures > 0 ? ` · ${fleet.failures} falha(s)` : ""}${fleet.skipped > 0 ? ` · ${fleet.skipped} com senha fora` : ""}${fleet.stopping ? " · parando…" : ""}`
    : fleet.total > 0
      ? `Última frota: ${fleet.done}/${fleet.total}${fleet.failures > 0 ? ` · ${fleet.failures} falha(s)` : ""}${fleet.skipped > 0 ? ` · ${fleet.skipped} com senha fora` : ""}${fleet.cancelled ? " · interrompida" : ""}`
      : fleet.skipped > 0
        ? `Última frota sem servidores elegíveis · ${fleet.skipped} com senha fora`
        : lidos > 0
          ? `${lidos} leitura(s) individual(is) nesta sessão`
          : "Sem leituras nesta sessão";

  return (
    <div id="infra" className="mt-1 scroll-mt-4">
      <div className="rounded-lg border border-zinc-200 p-2.5 dark:border-zinc-800">
        <div className="flex flex-wrap items-center gap-2">
          <div className="min-w-0">
            <p className="text-xs font-medium">
              {lidos} de {servers.length} com leitura
              {emAlerta > 0 ? <span className="text-amber-700 dark:text-amber-400"> · {emAlerta} em alerta</span> : null}
              {semResposta > 0 ? <span className="text-red-600 dark:text-red-400"> · {semResposta} sem resposta</span> : null}
              {comSenha > 0 ? <span className="text-zinc-500 dark:text-zinc-400"> · {comSenha} com senha</span> : null}
            </p>
            <p className={`text-[11px] ${fleet.active ? "text-teal-700 dark:text-teal-300" : "text-zinc-500 dark:text-zinc-400"}`}>
              {resumoFrota}
            </p>
          </div>

          <div className="ml-auto flex flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              variant={soAlertas ? "primary" : "ghost"}
              aria-pressed={soAlertas}
              onClick={() => setSoAlertas((v) => !v)}
              title="Mostra apenas servidores com alerta ou sem resposta"
            >
              <AlertTriangleIcon className="size-3.5" /> Só problemas
            </Button>
            <Select className="max-w-44" value={ordem} onChange={(e) => setOrdem(e.target.value as Ordem)}>
              <option value="inventario">Ordem do inventário</option>
              <option value="alerta">Alerta primeiro</option>
              <option value="antiga">Leitura mais antiga</option>
            </Select>
            {fleet.active ? (
              <Button size="sm" variant="danger" loading={fleet.stopping} onClick={pararFrota}>
                {fleet.stopping ? null : <XIcon className="size-3.5" />} {fleet.stopping ? "Parando…" : "Parar"}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="primary"
                disabled={servers.length === 0}
                onClick={() => void lerFrota(servers)}
                title={
                  comSenha > 0
                    ? `${comSenha} servidor(es) com senha ficam fora da frota; consulte individualmente para abrir o prompt`
                    : "Lê os servidores em sequência, um por vez"
                }
              >
                <RefreshIcon className="size-3.5" /> {lidos > 0 ? "Reler frota" : "Ler frota"}
              </Button>
            )}
          </div>
        </div>
        {fleet.active ? (
          <div className="mt-2 h-1 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800">
            <div
              className="h-full rounded-full bg-teal-500 transition-[width]"
              style={{ width: `${fleet.total > 0 ? Math.max(2, Math.round((fleet.done * 100) / fleet.total)) : 2}%` }}
            />
          </div>
        ) : null}
      </div>

      {linhas.length === 0 ? (
        <p className="mt-3 text-xs text-zinc-500 dark:text-zinc-400">
          {soAlertas
            ? "Nenhum servidor com alerta ou sem resposta nas leituras desta sessão."
            : "Nenhum servidor cadastrado para ler."}
        </p>
      ) : (
        <ul className="mt-2 divide-y divide-zinc-100 dark:divide-zinc-800">
          {linhas.map((s) => {
            const l = readings[s.id];
            const m = l?.metrics ?? null;
            const pior = m ? piorDisco(m) : null;
            const mem = m ? memoriaPct(m) : null;
            const carga = m ? cargaRelativa(m) : null;
            const velha = desatualizada(l);
            const expandido = aberto === s.id && m != null;
            return (
              <li key={s.id} className="py-2">
                <div className="flex flex-wrap items-center gap-2 text-sm">
                  <StatusDot status={s.lastStatus} />
                  <span className="font-medium">{s.name}</span>
                  <Badge tone={ENV_TONES[s.environment] ?? "neutral"}>{ENV_LABELS[s.environment] ?? s.environment}</Badge>
                  {s.groupName ? <Badge>{s.groupName}</Badge> : null}
                  <span className="truncate font-mono text-xs text-zinc-500 dark:text-zinc-400" title={s.lastStatusDetail ?? undefined}>
                    {s.username}@{s.host}:{s.port}
                  </span>
                  <span className="ml-auto shrink-0 text-[11px] text-zinc-500 dark:text-zinc-400">
                    {s.lastCheckedAt ? `teste ${formatRelative(Math.floor(new Date(s.lastCheckedAt).getTime() / 1000))}` : "nunca testado"}
                  </span>
                  <div className="flex shrink-0 items-center gap-1">
                    {m ? (
                      <button
                        type="button"
                        className="rounded-md px-1.5 py-1 text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                        onClick={() => setAberto((prev) => (prev === s.id ? null : s.id))}
                        title={expandido ? "Ocultar a leitura" : "Mostrar a leitura"}
                        aria-label={expandido ? "Ocultar a leitura" : "Mostrar a leitura"}
                      >
                        {expandido ? <ChevronDownIcon className="size-4" /> : <ChevronRightIcon className="size-4" />}
                      </button>
                    ) : null}
                    <Button
                      variant="ghost"
                      size="sm"
                      loading={l?.loading}
                      onClick={() => void coletar(s)}
                      title={
                        s.authMethod === "password"
                          ? "Este servidor usa senha: a leitura individual abre o prompt"
                          : undefined
                      }
                    >
                      {l?.loading ? null : <RefreshIcon className="size-3.5" />}
                      {m ? "Reler" : "Consultar"}
                    </Button>
                  </div>
                </div>

                {m ? (
                  <div className="mt-1 flex flex-wrap items-center gap-1.5 pl-4 text-[11px]">
                    {pior ? (
                      <Badge tone={pior.usePercent >= LIMIAR_DISCO ? "warning" : "neutral"} title={`${pior.mount}: ${pior.usePercent}% usado`}>
                        disco {pior.usePercent}%
                      </Badge>
                    ) : null}
                    {mem != null ? (
                      <Badge tone={mem >= LIMIAR_MEMORIA ? "warning" : "neutral"} title="Memória em uso">
                        mem {mem}%
                      </Badge>
                    ) : null}
                    {carga != null ? (
                      <Badge tone={carga >= LIMIAR_CARGA ? "warning" : "neutral"} title="Carga média de 1 min dividida pelo número de núcleos">
                        carga {carga.toFixed(2)}×
                      </Badge>
                    ) : null}
                    {m.warnings.length > 0 ? (
                      <Badge tone="warning">{m.warnings.length} alerta(s)</Badge>
                    ) : (
                      <Badge tone="success">sem alerta</Badge>
                    )}
                    {velha ? <Badge tone="warning">leitura antiga</Badge> : null}
                    {l?.readAt ? (
                      <span
                        className="text-zinc-500 dark:text-zinc-400"
                        title={formatDateTime(new Date(l.readAt).toISOString())}
                      >
                        lido {formatRelative(Math.floor(l.readAt / 1000))}
                      </span>
                    ) : null}
                  </div>
                ) : null}

                {l?.error ? (
                  <p className="mt-1.5 flex items-start gap-1.5 rounded-lg bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-700 dark:text-red-300">
                    <ServerIcon className="mt-0.5 size-3.5 shrink-0" />
                    <span>{l.error}</span>
                  </p>
                ) : null}
                {expandido && m ? <Metrica m={m} /> : null}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
