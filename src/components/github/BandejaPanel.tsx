import { formatAgo } from "../../lib/format";
import type { GitHubInbox, GitHubSearchItem } from "../../types";
import { AlertTriangleIcon, InboxIcon, RefreshIcon } from "../Icons";
import { Badge, Button, Card, SectionTitle, Spinner } from "../ui";

function Lista({
  titulo,
  itens,
  vazio,
  onAbrir,
}: {
  titulo: string;
  itens: GitHubSearchItem[];
  vazio: string;
  onAbrir: (item: GitHubSearchItem) => void;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <SectionTitle>{titulo}</SectionTitle>
        <Badge tone={itens.length > 0 ? "accent" : "neutral"}>{itens.length}</Badge>
      </div>
      {itens.length === 0 ? (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">{vazio}</p>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {itens.map((item) => (
            <li key={`${item.repo}#${item.number}`}>
              <button
                type="button"
                onClick={() => onAbrir(item)}
                className="w-full rounded-lg border border-zinc-200 px-3 py-2 text-left transition hover:border-teal-500/60 dark:border-zinc-800 dark:hover:border-teal-500/60"
              >
                <p className="truncate text-sm">{item.title}</p>
                <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                  <span className="font-mono">
                    {item.repo}#{item.number}
                  </span>
                  {item.draft ? <Badge tone="warning">rascunho</Badge> : null}
                  {item.author ? <span>@{item.author}</span> : null}
                  <span>{formatAgo(item.updatedAt)}</span>
                </p>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * A bandeja da conta: três leituras da API de busca, cada uma com o que é meu.
 * Uma lista vazia é mostrada como vazia — nada de linha genérica para preencher
 * a tela.
 */
export function BandejaPanel({
  inbox,
  loading,
  error,
  onAbrir,
  onTentarDeNovo,
}: {
  inbox: GitHubInbox | null;
  loading: boolean;
  error: string | null;
  onAbrir: (item: GitHubSearchItem) => void;
  onTentarDeNovo: () => void;
}) {
  if (error) {
    return (
      <Card className="flex flex-wrap items-center justify-between gap-3 p-4">
        <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {error}
        </p>
        <Button size="sm" onClick={onTentarDeNovo}>
          <RefreshIcon className="size-3.5" /> Tentar de novo
        </Button>
      </Card>
    );
  }

  if (loading && inbox === null) {
    return (
      <Card className="flex items-center gap-2 p-5 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner className="size-4" /> Lendo a bandeja no GitHub…
      </Card>
    );
  }

  return (
    <Card className="p-5">
      <div className="mb-4 flex items-center gap-2">
        <InboxIcon className="size-4 text-teal-500" />
        <SectionTitle>Bandeja</SectionTitle>
        {loading ? <Spinner className="size-3.5" /> : null}
      </div>
      <div className="grid gap-6 md:grid-cols-3">
        <Lista
          titulo="Pediram sua revisão"
          itens={inbox?.reviewRequests ?? []}
          vazio="Nenhum pull request esperando você."
          onAbrir={onAbrir}
        />
        <Lista
          titulo="Seus pull requests"
          itens={inbox?.myPulls ?? []}
          vazio="Você não tem pull request aberto."
          onAbrir={onAbrir}
        />
        <Lista
          titulo="Atribuídas a você"
          itens={inbox?.myIssues ?? []}
          vazio="Nenhuma issue com você."
          onAbrir={onAbrir}
        />
      </div>
    </Card>
  );
}
