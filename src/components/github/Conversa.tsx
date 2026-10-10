import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../lib/api";
import { formatAgo } from "../../lib/format";
import { useApp } from "../../store/AppContext";
import type { GitHubComment } from "../../types";
import { AlertTriangleIcon, CommentIcon, ExternalLinkIcon, RefreshIcon } from "../Icons";
import { Button, SectionTitle, Spinner } from "../ui";

/**
 * O fio de comentários de um issue ou pull request. `versao` muda quando alguém
 * escreve, e a lista é relida — o que aparece é o que o GitHub devolveu agora.
 */
export function Conversa({ repo, number, versao = 0 }: { repo: string; number: number; versao?: number }) {
  const { pushToast } = useApp();
  const [itens, setItens] = useState<GitHubComment[] | null>(null);
  const [erro, setErro] = useState<string | null>(null);

  const ler = async () => {
    setErro(null);
    try {
      setItens(await api.githubComments(repo, number));
    } catch (e) {
      setItens(null);
      setErro(String(e));
    }
  };

  useEffect(() => {
    let vivo = true;
    setItens(null);
    void (async () => {
      try {
        const r = await api.githubComments(repo, number);
        if (vivo) setItens(r);
      } catch (e) {
        if (vivo) {
          setItens(null);
          setErro(String(e));
        }
      }
    })();
    return () => {
      vivo = false;
    };
  }, [repo, number, versao]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <CommentIcon className="size-3.5 text-teal-500" />
        <SectionTitle>Conversa</SectionTitle>
        {itens !== null ? <span className="text-[11px] text-zinc-500 dark:text-zinc-400">{itens.length}</span> : null}
        <Button size="sm" variant="ghost" className="ml-auto" onClick={() => void ler()}>
          <RefreshIcon className="size-3" />
        </Button>
      </div>

      {erro ? (
        <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {erro}
        </p>
      ) : itens === null ? (
        <p className="flex items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400">
          <Spinner className="size-3" /> lendo os comentários…
        </p>
      ) : itens.length === 0 ? (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">Ninguém comentou #{number} ainda.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {itens.map((c) => (
            <li
              key={c.url}
              className="rounded-lg border border-zinc-200 px-2.5 py-2 dark:border-zinc-800"
            >
              <p className="flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                {c.author ? <span className="font-mono">@{c.author}</span> : <span>autor sem login na resposta</span>}
                <span>{formatAgo(c.createdAt)}</span>
                <Button
                  size="sm"
                  variant="ghost"
                  className="ml-auto"
                  onClick={() =>
                    void openUrl(c.url).catch(() => pushToast("error", `Abra manualmente em ${c.url}`))
                  }
                >
                  <ExternalLinkIcon className="size-3" />
                </Button>
              </p>
              {c.body ? (
                <p className="mt-1 max-h-40 overflow-y-auto whitespace-pre-wrap text-xs text-zinc-600 dark:text-zinc-300">
                  {c.body}
                </p>
              ) : (
                <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">O GitHub devolveu este comentário sem texto.</p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
