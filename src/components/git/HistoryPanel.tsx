import { useMemo, useState } from "react";
import type { GraphRow } from "../../types";
import { formatRelative, formatTimestamp } from "../../lib/format";
import { Badge, Button, Card, EmptyState, Input } from "../ui";
import { HistoryIcon, SearchIcon } from "../Icons";

/** O prefixo do `git log --graph` usa colunas de 2 caracteres; cada coluna é uma faixa. */
const LANE_COLORS = [
  "text-teal-500",
  "text-violet-500",
  "text-amber-500",
  "text-sky-500",
  "text-rose-500",
  "text-emerald-500",
  "text-fuchsia-500",
];

function GraphLane({ graph }: { graph: string }) {
  const cells = graph.match(/.{1,2}/g) ?? [];
  return (
    <span className="shrink-0 select-none whitespace-pre font-mono text-[11px] leading-4" aria-hidden="true">
      {cells.map((cell, index) =>
        /\S/.test(cell) ? (
          <span key={index} className={LANE_COLORS[index % LANE_COLORS.length]}>
            {cell}
          </span>
        ) : (
          <span key={index}>{cell}</span>
        ),
      )}
    </span>
  );
}

function refTone(ref: string): "success" | "accent" | "violet" | "info" {
  if (ref.startsWith("tag:")) return "violet";
  if (ref === "HEAD") return "success";
  if (ref.startsWith("origin/") || ref.includes("/")) return "info";
  return "accent";
}

export function HistoryPanel({
  history,
  selectedId,
  onSelect,
  busy,
  onLoadMore,
}: {
  history: GraphRow[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  busy: boolean;
  onLoadMore: (limit: number) => void;
}) {
  const [filter, setFilter] = useState("");

  const rows = useMemo(() => {
    const term = filter.trim().toLowerCase();
    if (term === "") return history;
    return history.filter(
      (row) =>
        row.commit.subject.toLowerCase().includes(term) ||
        row.commit.authorName.toLowerCase().includes(term) ||
        row.commit.shortId.toLowerCase().includes(term) ||
        row.commit.refs.some((r) => r.toLowerCase().includes(term)),
    );
  }, [filter, history]);

  if (history.length === 0) {
    return (
      <Card>
        <EmptyState
          icon={<HistoryIcon className="size-8" />}
          title="Sem commits ainda"
          description="O repositório não tem histórico para mostrar. Depois do primeiro commit as alterações aparecem aqui em grafo."
        />
      </Card>
    );
  }

  return (
    <div className="space-y-2">
      <div className="relative">
        <SearchIcon className="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-zinc-400" />
        <Input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filtrar por mensagem, autor, ref ou hash…"
          className="pl-8 text-xs"
        />
      </div>
      <Card className="divide-y divide-zinc-100 overflow-hidden dark:divide-zinc-800">
        {rows.length === 0 ? (
          <p className="px-3 py-6 text-center text-xs text-zinc-500 dark:text-zinc-400">
            Nenhum commit corresponde a “{filter.trim()}”.
          </p>
        ) : null}
        {rows.map((row) => {
          const active = row.commit.id === selectedId;
          return (
            <button
              key={row.commit.id}
              onClick={() => onSelect(row.commit.id)}
              className={`flex w-full items-start gap-2 px-2.5 py-2 text-left transition ${
                active ? "bg-teal-500/10" : "hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50"
              }`}
            >
              <GraphLane graph={row.graph} />
              <span className="min-w-0 flex-1">
                <span className="flex flex-wrap items-center gap-1.5">
                  {row.commit.refs.map((ref) => (
                    <Badge key={ref} tone={refTone(ref)}>
                      {ref}
                    </Badge>
                  ))}
                  <span className="truncate text-xs font-medium">{row.commit.subject}</span>
                </span>
                <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-[11px] text-zinc-500 dark:text-zinc-400">
                  <span className="font-mono text-zinc-400 dark:text-zinc-500">{row.commit.shortId}</span>
                  <span>{row.commit.authorName}</span>
                  <span title={formatTimestamp(row.commit.unixDate)}>{formatRelative(row.commit.unixDate)}</span>
                  {row.commit.parents.length > 1 ? <Badge tone="warning">merge</Badge> : null}
                </span>
              </span>
            </button>
          );
        })}
      </Card>
      <div className="flex items-center justify-between px-1">
        <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
          {history.length} commit(s) carregados{filter.trim() ? ` · ${rows.length} visíveis` : ""}
        </p>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => onLoadMore(history.length + 300)}>
          Carregar mais 300
        </Button>
      </div>
    </div>
  );
}
