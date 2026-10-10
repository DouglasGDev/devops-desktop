import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../lib/api";
import { formatAgo } from "../../lib/format";
import { useApp } from "../../store/AppContext";
import type { GitHubIssue, ViewId } from "../../types";
import {
  AlertTriangleIcon,
  CheckIcon,
  CommentIcon,
  ExternalLinkIcon,
  IssueIcon,
  PlusIcon,
  RefreshIcon,
  SearchIcon,
  XIcon,
} from "../Icons";
import { Badge, Button, Card, Confirm, EmptyState, Field, Input, Spinner, Textarea } from "../ui";

export function IssuePanel({
  repo,
  numeroInicial,
  onNavigate,
}: {
  repo: string;
  numeroInicial: number | null;
  onNavigate: (v: ViewId) => void;
}) {
  const { pushToast } = useApp();

  const [issues, setIssues] = useState<GitHubIssue[] | null>(null);
  const [listaErro, setListaErro] = useState<string | null>(null);
  const [filtro, setFiltro] = useState("");
  const [numero, setNumero] = useState<number | null>(numeroInicial);
  const [comentario, setComentario] = useState("");
  const [acao, setAcao] = useState<string | null>(null);
  const [fechando, setFechando] = useState(false);

  const [criando, setCriando] = useState(false);
  const [titulo, setTitulo] = useState("");
  const [corpo, setCorpo] = useState("");

  const lerLista = useCallback(async () => {
    setListaErro(null);
    try {
      setIssues(await api.githubIssues(repo));
    } catch (e) {
      setIssues(null);
      setListaErro(String(e));
    }
  }, [repo]);

  useEffect(() => {
    setNumero(numeroInicial);
    void lerLista();
  }, [lerLista, numeroInicial]);

  const visiveis = useMemo(() => {
    const todos = issues ?? [];
    const termo = filtro.trim().toLowerCase();
    if (termo === "") return todos;
    return todos.filter(
      (i) =>
        i.title.toLowerCase().includes(termo) ||
        String(i.number) === termo.replace("#", "") ||
        i.labels.some((l) => l.toLowerCase().includes(termo)),
    );
  }, [issues, filtro]);

  const escolhida = (issues ?? []).find((i) => i.number === numero) ?? null;

  const escrever = async (nome: string, rota: () => Promise<unknown>, depois?: () => void) => {
    setAcao(nome);
    try {
      await rota();
      pushToast("success", `${nome} concluído no GitHub.`);
      depois?.();
      await lerLista();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setAcao(null);
    }
  };

  const comentar = () =>
    escrever("Comentário", () => api.githubComment({ repo, number: numero!, body: comentario }), () =>
      setComentario(""),
    );

  const fecharIssue = () =>
    escrever("Fechamento da issue", () => api.githubCloseIssue(repo, numero!), () => {
      setFechando(false);
      setNumero(null);
    });

  const abrirIssue = async () => {
    setAcao("Nova issue");
    try {
      const criada = await api.githubCreateIssue({
        repo,
        title: titulo.trim(),
        body: corpo.trim() || null,
      });
      setCriando(false);
      setTitulo("");
      setCorpo("");
      setNumero(criada.number);
      pushToast("success", `Issue #${criada.number} aberta no GitHub.`);
      await lerLista();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setAcao(null);
    }
  };

  if (listaErro) {
    return (
      <Card className="flex flex-wrap items-center justify-between gap-3 p-4">
        <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {listaErro}
        </p>
        <Button size="sm" onClick={() => void lerLista()}>
          <RefreshIcon className="size-3.5" /> Tentar de novo
        </Button>
      </Card>
    );
  }

  if (issues === null) {
    return (
      <Card className="flex items-center gap-2 p-5 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner className="size-4" /> Lendo as issues de {repo}…
      </Card>
    );
  }

  return (
    <div className="grid min-h-0 gap-3 lg:grid-cols-[minmax(0,17rem)_minmax(0,1fr)]">
      <Card className="flex max-h-[34rem] flex-col gap-2 overflow-y-auto p-2">
        <div className="flex items-center gap-1.5 px-1">
          <div className="relative min-w-0 flex-1">
            <SearchIcon className="pointer-events-none absolute left-2 top-2 size-3.5 text-zinc-400" />
            <Input
              className="pl-8 text-xs"
              placeholder="filtrar por título ou label"
              value={filtro}
              onChange={(e) => setFiltro(e.target.value)}
            />
          </div>
          <Button size="sm" variant="ghost" onClick={() => void lerLista()}>
            <RefreshIcon className="size-3.5" />
          </Button>
        </div>
        <Button size="sm" variant="secondary" onClick={() => setCriando((v) => !v)}>
          <PlusIcon className="size-3.5" /> Nova issue
        </Button>
        {criando ? (
          <div className="flex flex-col gap-2 rounded-lg border border-zinc-200 p-2 dark:border-zinc-800">
            <Input placeholder="título" value={titulo} onChange={(e) => setTitulo(e.target.value)} />
            <Textarea rows={3} placeholder="o que acontece (opcional)" value={corpo} onChange={(e) => setCorpo(e.target.value)} />
            <Button
              size="sm"
              variant="primary"
              loading={acao === "Nova issue"}
              disabled={titulo.trim() === ""}
              onClick={() => void abrirIssue()}
            >
              Abrir no GitHub
            </Button>
          </div>
        ) : null}
        <p className="px-1 text-[11px] text-zinc-500 dark:text-zinc-400">{visiveis.length} abertas</p>
        {visiveis.length === 0 ? (
          <p className="px-1 text-xs text-zinc-500 dark:text-zinc-400">
            {issues.length === 0 ? "Nenhuma issue aberta aqui." : "Nada corresponde ao filtro."}
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {visiveis.map((i) => (
              <li key={i.number}>
                <button
                  type="button"
                  onClick={() => setNumero(i.number)}
                  className={`w-full rounded-lg border px-2.5 py-2 text-left transition ${
                    numero === i.number
                      ? "border-teal-500/70 bg-teal-500/5"
                      : "border-zinc-200 hover:border-zinc-300 dark:border-zinc-800 dark:hover:border-zinc-700"
                  }`}
                >
                  <p className="truncate text-sm">{i.title}</p>
                  <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                    <span className="font-mono">#{i.number}</span>
                    {i.labels.slice(0, 2).map((l) => (
                      <Badge key={l} tone="violet">
                        {l}
                      </Badge>
                    ))}
                    {i.comments > 0 ? <span>{i.comments} comentários</span> : null}
                    <span>{formatAgo(i.updatedAt)}</span>
                  </p>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card className="flex min-h-0 flex-col gap-3 p-4">
        {escolhida === null ? (
          <EmptyState
            icon={<IssueIcon className="size-8" />}
            title="Escolha uma issue"
            description="A esquerda lista as abertas no GitHub agora; a direita mostra a descrição, comenta e fecha."
          >
            {issues.length === 0 ? (
              <Button size="sm" variant="primary" onClick={() => setCriando(true)}>
                <PlusIcon className="size-3.5" /> Abrir a primeira
              </Button>
            ) : null}
          </EmptyState>
        ) : (
          <>
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <p className="text-sm font-medium">{escolhida.title}</p>
                <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                  <span className="font-mono">#{escolhida.number}</span>
                  <Badge tone={escolhida.state === "open" ? "success" : "neutral"}>{escolhida.state}</Badge>
                  {escolhida.author ? <span>@{escolhida.author}</span> : null}
                  <span>{formatAgo(escolhida.updatedAt)}</span>
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                {escolhida.labels.map((l) => (
                  <Badge key={l} tone="violet">
                    {l}
                  </Badge>
                ))}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void openUrl(escolhida.url).catch(() => pushToast("error", `Abra manualmente em ${escolhida.url}`))
                  }
                >
                  <ExternalLinkIcon className="size-3.5" /> Site
                </Button>
              </div>
            </div>

            {escolhida.body ? (
              <p className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-lg bg-zinc-100 p-2.5 text-xs text-zinc-600 dark:bg-zinc-900 dark:text-zinc-300">
                {escolhida.body}
              </p>
            ) : (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                O GitHub devolveu esta issue sem descrição — quem abriu não escreveu corpo.
              </p>
            )}

            <div className="flex flex-col gap-2">
              <Field label={`Comentar na issue #${escolhida.number}`}>
                <Textarea
                  rows={3}
                  value={comentario}
                  placeholder="escreva aqui o que você descobriu no repositório local"
                  onChange={(e) => setComentario(e.target.value)}
                />
              </Field>
              <div className="flex flex-wrap items-center gap-1.5">
                <Button
                  size="sm"
                  variant="primary"
                  disabled={comentario.trim() === "" || acao !== null}
                  onClick={() => void comentar()}
                >
                  <CommentIcon className="size-3.5" /> Comentar
                </Button>
                <Button
                  size="sm"
                  disabled={escolhida.state !== "open" || acao !== null}
                  onClick={() => setFechando(true)}
                >
                  <XIcon className="size-3.5" /> Fechar issue
                </Button>
                <Button size="sm" variant="ghost" onClick={() => onNavigate("git")}>
                  <CheckIcon className="size-3.5" /> Ver na seção Git
                </Button>
              </div>
            </div>

            {acao ? (
              <p className="flex items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400">
                <Spinner className="size-3" /> {acao} em andamento…
              </p>
            ) : null}
          </>
        )}

        <Confirm
          open={fechando}
          title={`Fechar a issue #${numero ?? ""}`}
          message={`Isto marca ${escolhida?.title ?? "a issue"} como fechada em ${repo} e avisa quem a abriu. O GitHub não desfaz isso sozinho.`}
          confirmLabel="Fechar no GitHub"
          busy={acao !== null}
          onConfirm={() => void fecharIssue()}
          onClose={() => setFechando(false)}
        />
      </Card>
    </div>
  );
}
