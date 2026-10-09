import { Fragment, useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import { formatDateTime } from "../lib/format";
import type { AuditEntry } from "../types";
import { ChevronDownIcon, ChevronRightIcon, HistoryIcon } from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  CATEGORY_LABELS,
  EmptyState,
  LEVEL_LABELS,
  LEVEL_TONES,
  Select,
  Spinner,
} from "../components/ui";

const PAGE_SIZE = 50;

export function HistoryView() {
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [category, setCategory] = useState("");
  const [level, setLevel] = useState("");
  const [search, setSearch] = useState("");
  const [applied, setApplied] = useState({ category: "", level: "", search: "" });
  const [offset, setOffset] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const load = useCallback(
    async (nextOffset: number) => {
      if (nextOffset === 0) setEntries(null);
      else setLoadingMore(true);
      try {
        const rows = await api.queryAuditLog({
          category: applied.category === "" ? null : applied.category,
          level: applied.level === "" ? null : applied.level,
          search: applied.search.trim() === "" ? null : applied.search.trim(),
          limit: PAGE_SIZE,
          offset: nextOffset,
        });
        setEntries((prev) => (prev === null || nextOffset === 0 ? rows : [...prev, ...rows]));
        setHasMore(rows.length === PAGE_SIZE);
        setOffset(nextOffset);
      } finally {
        setLoadingMore(false);
      }
    },
    [applied],
  );

  useEffect(() => {
    void load(0);
  }, [load]);

  const apply = () => {
    setApplied({ category, level, search });
    setExpanded(new Set());
  };

  const clear = () => {
    setCategory("");
    setLevel("");
    setSearch("");
    setApplied({ category: "", level: "", search: "" });
    setExpanded(new Set());
  };

  const toggleRow = (id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="flex flex-wrap items-end gap-2">
          <label className="block">
            <span className="mb-1 block text-xs font-medium text-zinc-500 dark:text-zinc-400">Categoria</span>
            <Select className="w-44" value={category} onChange={(e) => setCategory(e.target.value)}>
              <option value="">Todas</option>
              {Object.entries(CATEGORY_LABELS).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </Select>
          </label>
          <label className="block">
            <span className="mb-1 block text-xs font-medium text-zinc-500 dark:text-zinc-400">Nível</span>
            <Select className="w-36" value={level} onChange={(e) => setLevel(e.target.value)}>
              <option value="">Todos</option>
              {Object.entries(LEVEL_LABELS).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </Select>
          </label>
          <label className="block flex-1 min-w-52">
            <span className="mb-1 block text-xs font-medium text-zinc-500 dark:text-zinc-400">Buscar</span>
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") apply();
              }}
              placeholder="Operação, servidor, projeto, detalhe…"
              className="w-full rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-sm outline-none focus:ring-2 focus:ring-teal-500/40 dark:border-zinc-700 dark:bg-zinc-950"
            />
          </label>
          <div className="flex gap-2">
            <Button variant="primary" onClick={apply}>
              Filtrar
            </Button>
            <Button onClick={clear}>Limpar</Button>
          </div>
        </div>
      </Card>

      <Card className="p-4">
        {entries === null ? (
          <div className="flex items-center justify-center gap-2 py-10 text-sm text-zinc-500">
            <Spinner /> Carregando histórico…
          </div>
        ) : entries.length === 0 ? (
          <EmptyState
            icon={<HistoryIcon className="size-8" />}
            title="Nenhum registro"
            description="Nenhuma operação corresponde aos filtros atuais. Conexões, projetos, Git e deploys são registrados aqui automaticamente."
          />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-zinc-200 text-left text-[11px] uppercase tracking-wider text-zinc-400 dark:border-zinc-800 dark:text-zinc-500">
                  <th className="py-2 pr-3 font-medium">Quando</th>
                  <th className="py-2 pr-3 font-medium">Nível</th>
                  <th className="py-2 pr-3 font-medium">Categoria</th>
                  <th className="py-2 pr-3 font-medium">Operação</th>
                  <th className="py-2 pr-3 font-medium">Servidor / Projeto</th>
                  <th className="py-2 pr-3 font-medium">Saída</th>
                  <th className="w-8 py-2" />
                </tr>
              </thead>
              <tbody className="divide-y divide-zinc-100 dark:divide-zinc-800">
                {entries.map((e) => {
                  const open = expanded.has(e.id);
                  const subject = [e.serverName, e.projectName].filter(Boolean).join(" · ");
                  return (
                    <Fragment key={e.id}>
                      <tr className="align-top hover:bg-zinc-50 dark:hover:bg-zinc-800/40">
                        <td className="whitespace-nowrap py-2 pr-3 text-xs tabular-nums text-zinc-500 dark:text-zinc-400">
                          {formatDateTime(e.ts)}
                        </td>
                        <td className="py-2 pr-3">
                          <Badge tone={LEVEL_TONES[e.level] ?? "neutral"}>{LEVEL_LABELS[e.level] ?? e.level}</Badge>
                        </td>
                        <td className="whitespace-nowrap py-2 pr-3 text-xs text-zinc-500 dark:text-zinc-400">
                          {CATEGORY_LABELS[e.category] ?? e.category}
                        </td>
                        <td className="py-2 pr-3 font-medium">{e.operation}</td>
                        <td className="max-w-48 truncate py-2 pr-3 text-xs text-zinc-500 dark:text-zinc-400">{subject || "—"}</td>
                        <td className="py-2 pr-3 font-mono text-xs tabular-nums">{e.exitCode === null ? "—" : e.exitCode}</td>
                        <td className="py-2">
                          {e.detail ? (
                            <button
                              onClick={() => toggleRow(e.id)}
                              className="rounded p-1 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-700 dark:hover:bg-zinc-800 dark:hover:text-zinc-200"
                              title={open ? "Ocultar detalhe" : "Ver detalhe"}
                            >
                              {open ? <ChevronDownIcon className="size-3.5" /> : <ChevronRightIcon className="size-3.5" />}
                            </button>
                          ) : null}
                        </td>
                      </tr>
                      {open && e.detail ? (
                        <tr className="bg-zinc-50 dark:bg-zinc-800/30">
                          <td colSpan={7} className="px-3 pb-3">
                            <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-white p-2.5 font-mono text-[11px] leading-relaxed text-zinc-700 dark:bg-zinc-950 dark:text-zinc-300">
                              {e.detail}
                            </pre>
                          </td>
                        </tr>
                      ) : null}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
        {hasMore && entries !== null ? (
          <div className="mt-3 flex justify-center">
            <Button loading={loadingMore} onClick={() => void load(offset + PAGE_SIZE)}>
              Carregar mais
            </Button>
          </div>
        ) : null}
      </Card>
      <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
        Registros locais de auditoria. Senhas, chaves privadas, tokens e valores de segredos nunca são gravados no log.
      </p>
    </div>
  );
}
