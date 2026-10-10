import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../lib/api";
import { formatAgo } from "../../lib/format";
import { useApp } from "../../store/AppContext";
import type { GitHubRun } from "../../types";
import { AlertTriangleIcon, ChecksIcon, ExternalLinkIcon, RefreshIcon, UndoIcon } from "../Icons";
import { Badge, Button, Card, Confirm, EmptyState, SectionTitle, Spinner } from "../ui";

/** O ponto do run: cor só quando o GitHub concluiu; em andamento é neutro. */
function tom(run: GitHubRun): "success" | "error" | "warning" | "neutral" {
  if (run.status !== "completed") return run.status === "failure" ? "error" : "neutral";
  switch (run.conclusion) {
    case "success":
      return "success";
    case "failure":
      return "error";
    case "cancelled":
    case "timed_out":
    case "action_required":
      return "warning";
    default:
      return "neutral";
  }
}

function rotulo(run: GitHubRun): string {
  if (run.status !== "completed") return run.status.replace(/_/g, " ");
  return (run.conclusion ?? "sem conclusão").replace(/_/g, " ");
}

export function ChecksPanel({ repo }: { repo: string }) {
  const { pushToast } = useApp();

  const [runs, setRuns] = useState<GitHubRun[] | null>(null);
  const [erro, setErro] = useState<string | null>(null);
  const [lendo, setLendo] = useState(false);
  const [reexecutando, setReexecutando] = useState<GitHubRun | null>(null);
  const [ocupado, setOcupado] = useState(false);

  const ler = useCallback(async () => {
    setLendo(true);
    setErro(null);
    try {
      setRuns(await api.githubRuns(repo));
    } catch (e) {
      setRuns(null);
      setErro(String(e));
    } finally {
      setLendo(false);
    }
  }, [repo]);

  useEffect(() => {
    setRuns(null);
    void ler();
  }, [ler]);

  const resumo = useMemo(() => {
    const todos = runs ?? [];
    return {
      ok: todos.filter((r) => r.conclusion === "success").length,
      falha: todos.filter((r) => r.conclusion === "failure").length,
      rodando: todos.filter((r) => r.status !== "completed").length,
    };
  }, [runs]);

  const rerun = async () => {
    if (reexecutando === null) return;
    setOcupado(true);
    try {
      await api.githubRerun(repo, reexecutando.id);
      pushToast("success", `Workflow “${reexecutando.name}” reexecutando no GitHub.`);
      setReexecutando(null);
      await ler();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setOcupado(false);
    }
  };

  if (erro) {
    return (
      <Card className="flex flex-wrap items-center justify-between gap-3 p-4">
        <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {erro}
        </p>
        <Button size="sm" onClick={() => void ler()}>
          <RefreshIcon className="size-3.5" /> Tentar de novo
        </Button>
      </Card>
    );
  }

  if (runs === null) {
    return (
      <Card className="flex items-center gap-2 p-5 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner className="size-4" /> Lendo as execuções de {repo}…
      </Card>
    );
  }

  return (
    <Card className="p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <ChecksIcon className="size-4 text-teal-500" />
          <SectionTitle>Verificações</SectionTitle>
          <Badge tone="success">{resumo.ok} ok</Badge>
          <Badge tone="error">{resumo.falha} falhas</Badge>
          {resumo.rodando > 0 ? <Badge tone="warning">{resumo.rodando} em andamento</Badge> : null}
        </div>
        <Button size="sm" loading={lendo} onClick={() => void ler()}>
          <RefreshIcon className="size-3.5" /> Atualizar
        </Button>
      </div>

      {runs.length === 0 ? (
        <EmptyState
          icon={<ChecksIcon className="size-8" />}
          title="Nenhuma execução de workflow"
          description="O GitHub respondeu zero runs para este repositório: ou não há arquivo de workflow, ou nada rodou ainda."
        />
      ) : (
        <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
          {runs.map((r) => (
            <li key={r.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5">
              <span
                className={`size-2 shrink-0 rounded-full ${
                  tom(r) === "success"
                    ? "bg-emerald-500"
                    : tom(r) === "error"
                      ? "bg-red-500"
                      : tom(r) === "warning"
                        ? "bg-amber-500"
                        : "animate-pulse bg-zinc-400 dark:bg-zinc-500"
                }`}
              />
              <span className="min-w-0 truncate text-sm">{r.displayTitle || r.name}</span>
              <Badge tone={tom(r)}>{rotulo(r)}</Badge>
              <span className="font-mono text-[11px] text-zinc-500 dark:text-zinc-400">{r.name}</span>
              <span className="text-[11px] text-zinc-500 dark:text-zinc-400">
                {r.headBranch} · {r.event} · {formatAgo(r.updatedAt ?? r.createdAt)}
              </span>
              <span className="ml-auto flex flex-wrap items-center gap-1.5">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={r.status !== "completed"}
                  onClick={() => setReexecutando(r)}
                >
                  <UndoIcon className="size-3.5" /> Reexecutar
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void openUrl(r.url).catch(() => pushToast("error", `Abra manualmente em ${r.url}`))
                  }
                >
                  <ExternalLinkIcon className="size-3.5" /> Log
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}

      <Confirm
        open={reexecutando !== null}
        busy={ocupado}
        title="Reexecutar no GitHub Actions"
        message={
          `Vai pedir ao GitHub para rodar de novo “${reexecutando?.name ?? ""}” sobre ${reexecutando?.headBranch ?? ""}. ` +
          "O workflow do repositório decide o que acontece — a aplicação não escolhe os passos."
        }
        confirmLabel="Reexecutar"
        onConfirm={() => void rerun()}
        onClose={() => setReexecutando(null)}
      />
    </Card>
  );
}
