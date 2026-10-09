import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { SearchResults, ViewId } from "../types";
import { useApp } from "../store/AppContext";
import { MoonIcon, SearchIcon, SunIcon } from "./Icons";
import { VIEW_TITLES } from "./Sidebar";

export function Topbar({ view, onNavigate }: { view: ViewId; onNavigate: (v: ViewId) => void }) {
  const { theme, setTheme } = useApp();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResults | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResults(null);
      setOpen(false);
      return;
    }
    const t = window.setTimeout(() => {
      api
        .searchAll(q)
        .then((r) => {
          setResults(r);
          setOpen(true);
        })
        .catch(() => setResults(null));
    }, 250);
    return () => window.clearTimeout(t);
  }, [query]);

  const close = () => {
    setOpen(false);
    setQuery("");
    setResults(null);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    const target: ViewId | null = results
      ? results.servers.length > 0
        ? "servers"
        : results.projects.length > 0
          ? "projects"
          : results.history.length > 0
            ? "history"
            : null
      : null;
    if (target) {
      onNavigate(target);
      close();
    }
  };

  const total = results ? results.servers.length + results.projects.length + results.history.length : 0;

  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-zinc-200 px-5 dark:border-zinc-800">
      <h1 className="shrink-0 text-sm font-semibold">{VIEW_TITLES[view]}</h1>
      <div className="relative ml-auto w-full max-w-md">
        <SearchIcon className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-zinc-400" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Pesquisar servidores, projetos e histórico…"
          className="w-full rounded-lg border border-zinc-200 bg-zinc-100/70 py-1.5 pl-9 pr-3 text-sm outline-none placeholder:text-zinc-400 focus:ring-2 focus:ring-teal-500/40 dark:border-zinc-800 dark:bg-zinc-800/50 dark:placeholder:text-zinc-500"
        />
        {open && results ? (
          <>
            <div className="fixed inset-0 z-10" onClick={close} />
            <div className="absolute z-20 mt-1.5 max-h-80 w-full overflow-y-auto rounded-xl border border-zinc-200 bg-white p-1.5 shadow-xl dark:border-zinc-700 dark:bg-zinc-900">
              {total === 0 ? (
                <p className="px-2.5 py-3 text-sm text-zinc-500">Nada encontrado para “{query.trim()}”.</p>
              ) : null}
              {results.servers.length > 0 ? (
                <Group title="Servidores">
                  {results.servers.map((s) => (
                    <ResultRow key={`s-${s.id}`} onClick={() => (onNavigate("servers"), close())}>
                      <span className="font-medium">{s.name}</span>
                      <span className="font-mono text-xs text-zinc-500">{s.host}</span>
                    </ResultRow>
                  ))}
                </Group>
              ) : null}
              {results.projects.length > 0 ? (
                <Group title="Projetos">
                  {results.projects.map((p) => (
                    <ResultRow key={`p-${p.id}`} onClick={() => (onNavigate("projects"), close())}>
                      <span className="font-medium">{p.name}</span>
                      <span className="truncate font-mono text-xs text-zinc-500">{p.localPath}</span>
                    </ResultRow>
                  ))}
                </Group>
              ) : null}
              {results.history.length > 0 ? (
                <Group title="Histórico">
                  {results.history.map((h) => (
                    <ResultRow key={`h-${h.id}`} onClick={() => (onNavigate("history"), close())}>
                      <span className="font-medium">{h.operation}</span>
                      <span className="text-xs text-zinc-500">
                        {h.serverName ?? h.projectName ?? CATEGORY_FALLBACK(h.category)}
                      </span>
                    </ResultRow>
                  ))}
                </Group>
              ) : null}
            </div>
          </>
        ) : null}
      </div>
      <button
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
        title={theme === "dark" ? "Mudar para tema claro" : "Mudar para tema escuro"}
        className="rounded-lg p-2 text-zinc-500 transition hover:bg-zinc-200/70 dark:text-zinc-400 dark:hover:bg-zinc-800"
      >
        {theme === "dark" ? <SunIcon /> : <MoonIcon />}
      </button>
    </header>
  );
}

function CATEGORY_FALLBACK(category: string) {
  return category.charAt(0).toUpperCase() + category.slice(1);
}

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mb-1">
      <p className="px-2.5 pb-1 pt-1.5 text-[10px] font-semibold uppercase tracking-wider text-zinc-400">{title}</p>
      {children}
    </div>
  );
}

function ResultRow({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className="flex w-full flex-col items-start gap-0.5 rounded-lg px-2.5 py-1.5 text-left text-sm transition hover:bg-zinc-100 dark:hover:bg-zinc-800"
    >
      {children}
    </button>
  );
}
