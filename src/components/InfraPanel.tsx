import { useCallback, useState } from "react";
import { api } from "../lib/api";
import { formatDateTime, formatKilobytes, formatUptime } from "../lib/format";
import type { Server, ServerMetrics } from "../types";
import { useApp } from "../store/AppContext";
import { AlertTriangleIcon, ChevronDownIcon, ChevronRightIcon, RefreshIcon, ServerIcon } from "./Icons";
import { Badge, Button, ENV_LABELS, ENV_TONES, StatusDot } from "./ui";

type Leitura = { loading: boolean; metrics: ServerMetrics | null; error: string | null };

// Os limiares são os mesmos do backend (infra.rs): 85% de disco e 90% de
// memória. Mostrar a cor aqui repete a regra, não cria um dado novo.
const DISCO_ALERTA = 85;
const MEMORIA_ALERTA = 90;

function barraTone(pct: number, alerta: number) {
  if (pct >= alerta) return "bg-red-500";
  if (pct >= alerta - 15) return "bg-amber-500";
  return "bg-teal-500";
}

function Barra({ pct, alerta }: { pct: number; alerta: number }) {
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800">
      <div
        className={`h-full rounded-full ${barraTone(pct, alerta)}`}
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
  const memUsadaPct =
    m.memTotalKb && m.memTotalKb > 0 && m.memAvailableKb != null
      ? Math.round(((m.memTotalKb - m.memAvailableKb) * 100) / m.memTotalKb)
      : null;
  const carga = m.load1 != null && m.cpuCount && m.cpuCount > 0 ? Math.round((m.load1 / m.cpuCount) * 100) : null;

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
          <p className="flex items-baseline justify-between text-[11px] text-zinc-500 dark:text-zinc-400">
            <span>Carga de trabalho</span>
            <span className="tabular-nums">
              {m.load1 != null || m.load5 != null || m.load15 != null
                ? `${m.load1 ?? "—"} / ${m.load5 ?? "—"} / ${m.load15 ?? "—"} (1, 5, 15 min)`
                : "não informado"}
            </span>
          </p>
          {carga != null ? (
            <div className="mt-1">
              <Barra pct={carga} alerta={DISCO_ALERTA} />
              <p className="mt-0.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                {carga}% de {m.cpuCount} núcleo(s) — média de 1 min
              </p>
            </div>
          ) : null}
        </div>

        <div>
          <p className="flex items-baseline justify-between text-[11px] text-zinc-500 dark:text-zinc-400">
            <span>Memória</span>
            <span className="tabular-nums">
              {memUsadaPct != null
                ? `${formatKilobytes(m.memTotalKb && m.memAvailableKb != null ? m.memTotalKb - m.memAvailableKb : null)} de ${formatKilobytes(m.memTotalKb)}`
                : "não informado"}
            </span>
          </p>
          {memUsadaPct != null ? (
            <div className="mt-1">
              <Barra pct={memUsadaPct} alerta={MEMORIA_ALERTA} />
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
        {m.disks.length === 0 ? (
          <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
            O servidor não informou nenhum sistema de arquivos legível.
          </p>
        ) : (
          <ul className="mt-1 space-y-1.5">
            {m.disks.map((d) => (
              <li key={d.mount} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
                <div className="min-w-0">
                  <p className="truncate font-mono text-[11px]" title={d.mount}>
                    {d.mount}
                  </p>
                  <div className="mt-1">
                    <Barra pct={d.usePercent} alerta={DISCO_ALERTA} />
                  </div>
                </div>
                <p className="shrink-0 text-right text-[11px] tabular-nums text-zinc-500 dark:text-zinc-400">
                  {d.usePercent}% · {formatKilobytes(d.usedKb)} / {formatKilobytes(d.totalKb)}
                </p>
              </li>
            ))}
          </ul>
        )}
      </div>

      {m.services.length > 0 ? (
        <div>
          <p className="text-[11px] text-zinc-500 dark:text-zinc-400">
            Containers e serviços reportados pelo próprio servidor
          </p>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {m.services.map((s) => (
              <Badge
                key={`${s.kind}:${s.name}`}
                tone={/up|running|active/i.test(s.state) ? "success" : /exit|dead|failed|stopped|unhealthy/i.test(s.state) ? "error" : "neutral"}
              >
                {s.kind === "container" ? "docker" : "systemd"} · {s.name}: {s.state || "sem estado"}
              </Badge>
            ))}
          </div>
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
        Lido em {formatDateTime(new Date(m.collectedAt).toISOString())} por comandos de leitura sem impacto.
      </p>
    </div>
  );
}

export function InfraPanel({ servers }: { servers: Server[] }) {
  const { pushToast } = useApp();
  const [leituras, setLeituras] = useState<Record<number, Leitura>>({});
  const [aberto, setAberto] = useState<number | null>(null);

  const coletar = useCallback(
    async (server: Server) => {
      setLeituras((prev) => ({ ...prev, [server.id]: { loading: true, metrics: null, error: null } }));
      try {
        const m = await api.serverMetrics(server.id);
        setLeituras((prev) => ({ ...prev, [server.id]: { loading: false, metrics: m, error: null } }));
        setAberto(server.id);
      } catch (e) {
        const msg = String(e);
        setLeituras((prev) => ({ ...prev, [server.id]: { loading: false, metrics: null, error: msg } }));
        setAberto(server.id);
        pushToast("error", `Métricas de ${server.name}: ${msg}`);
      }
    },
    [pushToast],
  );

  return (
    <div className="mt-1">
      <p className="text-xs text-zinc-500 dark:text-zinc-400">
        Nada é consultado sozinho: cada leitura executa apenas comandos de leitura no servidor escolhido e mostra o que
        o host respondeu. Campo sem resposta aparece como "não informado" — nunca um valor inventado.
      </p>
      <ul className="mt-3 divide-y divide-zinc-100 dark:divide-zinc-800">
        {servers.map((s) => {
          const l = leituras[s.id];
          const expansion = aberto === s.id ? l : undefined;
          return (
            <li key={s.id} className="py-2">
              <div className="flex items-center gap-2 text-sm">
                <StatusDot status={s.lastStatus} />
                <span className="font-medium">{s.name}</span>
                <Badge tone={ENV_TONES[s.environment] ?? "neutral"}>{ENV_LABELS[s.environment] ?? s.environment}</Badge>
                <span className="truncate font-mono text-xs text-zinc-500 dark:text-zinc-400" title={s.lastStatusDetail ?? undefined}>
                  {s.username}@{s.host}:{s.port}
                </span>
                <span className="ml-auto shrink-0 text-[11px] text-zinc-500 dark:text-zinc-400">
                  {s.lastCheckedAt ? formatDateTime(s.lastCheckedAt) : "nunca testado"}
                </span>
                <div className="ml-auto flex shrink-0 items-center gap-1">
                  {l?.metrics ? (
                    <button
                      type="button"
                      className="rounded-md px-1.5 py-1 text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                      onClick={() => setAberto((prev) => (prev === s.id ? null : s.id))}
                      title={aberto === s.id ? "Ocultar a leitura" : "Mostrar a leitura"}
                      aria-label={aberto === s.id ? "Ocultar a leitura" : "Mostrar a leitura"}
                    >
                      {aberto === s.id ? (
                        <ChevronDownIcon className="size-4" />
                      ) : (
                        <ChevronRightIcon className="size-4" />
                      )}
                    </button>
                  ) : null}
                  <Button variant="ghost" size="sm" loading={l?.loading} onClick={() => void coletar(s)}>
                    {l?.loading ? null : <RefreshIcon className="size-3.5" />}
                    {l?.metrics ? "Reler" : "Consultar métricas"}
                  </Button>
                </div>
              </div>

              {expansion?.error ? (
                <p className="mt-1.5 flex items-start gap-1.5 rounded-lg bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-700 dark:text-red-300">
                  <ServerIcon className="mt-0.5 size-3.5 shrink-0" />
                  <span>{expansion.error}</span>
                </p>
              ) : null}
              {expansion?.metrics ? <Metrica m={expansion.metrics} /> : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
