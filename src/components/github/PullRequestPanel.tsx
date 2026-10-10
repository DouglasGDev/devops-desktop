import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../lib/api";
import { apontaPara } from "../../lib/repo";
import { useMapearPasta } from "../../lib/useMapearPasta";
import { formatAgo } from "../../lib/format";
import { useApp } from "../../store/AppContext";
import type { BranchInfo, FileDiff, GitHubPull, GitHubPullDetail, MergeMethod, ViewId } from "../../types";
import { Conversa } from "./Conversa";
import { DiffView } from "../git/DiffView";
import {
  AlertTriangleIcon,
  CheckIcon,
  CommentIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FolderIcon,
  GitBranchIcon,
  MergeIcon,
  PlusIcon,
  RefreshIcon,
  XIcon,
} from "../Icons";
import { Badge, Button, Card, Checkbox, Confirm, EmptyState, Field, Input, Modal, Select, Spinner, Textarea } from "../ui";

const METODOS: { valor: MergeMethod; nome: string; porque: string }[] = [
  { valor: "merge", nome: "Merge commit", porque: "guarda os commits do ramo e um commit de merge" },
  { valor: "squash", nome: "Squash", porque: "junta tudo em um commit só no ramo base" },
  { valor: "rebase", nome: "Rebase", porque: "replica os commits um a um, sem commit de merge" },
];

/** O que `mergeable_state` do GitHub quer dizer em uma frase, sem inventar. */
function estado_merge(detalhe: GitHubPullDetail): { tom: "success" | "warning" | "error" | "neutral"; texto: string } {
  if (detalhe.mergeable === null || detalhe.mergeable === undefined) {
    return { tom: "neutral", texto: "o GitHub ainda está calculando se dá para mergear" };
  }
  if (detalhe.mergeable === false) {
    return { tom: "error", texto: "há conflito com o ramo base — o GitHub não mescla sozinho" };
  }
  switch (detalhe.mergeableState) {
    case "clean":
      return { tom: "success", texto: "pronto para mesclar" };
    case "blocked":
      return { tom: "warning", texto: "bloqueado: faltam aprovações ou checagens exigidas" };
    case "behind":
      return { tom: "warning", texto: "o ramo base avançou; atualize antes de mesclar" };
    case "unstable":
      return { tom: "warning", texto: "as checagens passaram com falha ou aviso" };
    case "draft":
      return { tom: "warning", texto: "ainda é rascunho" };
    default:
      return { tom: "neutral", texto: detalhe.mergeableState ?? "o GitHub não informou o estado" };
  }
}

export function PullRequestPanel({
  repo,
  numeroInicial,
  branchPadrao,
  onNavigate,
}: {
  repo: string;
  numeroInicial: number | null;
  branchPadrao: string | null;
  onNavigate: (v: ViewId) => void;
}) {
  const { projects, pushToast } = useApp();
  const { mapear, mapeando } = useMapearPasta();

  const [pulls, setPulls] = useState<GitHubPull[] | null>(null);
  const [listaErro, setListaErro] = useState<string | null>(null);
  const [numero, setNumero] = useState<number | null>(numeroInicial);
  const [detalhe, setDetalhe] = useState<GitHubPullDetail | null>(null);
  const [arquivos, setArquivos] = useState<FileDiff[] | null>(null);
  const [lendo, setLendo] = useState(false);
  const [detalheErro, setDetalheErro] = useState<string | null>(null);
  const [comentario, setComentario] = useState("");
  const [acao, setAcao] = useState<string | null>(null);
  const [projeto, setProjeto] = useState<number | "">("");
  const [mergeAsk, setMergeAsk] = useState<MergeMethod | null>(null);
  const [conversa, setConversa] = useState(0);

  // Abrir pull request a partir de um ramo desta máquina.
  const [novoAberto, setNovoAberto] = useState(false);
  const [novoProjeto, setNovoProjeto] = useState<number | "">("");
  const [ramosLocais, setRamosLocais] = useState<BranchInfo[] | null>(null);
  const [ramosRemotos, setRamosRemotos] = useState<string[] | null>(null);
  const [head, setHead] = useState("");
  const [base, setBase] = useState("");
  const [titulo, setTitulo] = useState("");
  const [descricao, setDescricao] = useState("");
  const [rascunho, setRascunho] = useState(false);

  const repositorios = useMemo(() => projects.filter((p) => p.isGitRepo), [projects]);

  // A pasta mapeada para este repositório é a que as ações locais usam por
  // padrão: buscar o ramo do PR e abrir PR nascem do lugar certo.
  const pastaDesteRepo = useMemo(
    () => projects.find((p) => apontaPara(p.gitRemote, repo)) ?? null,
    [projects, repo],
  );

  useEffect(() => {
    if (pastaDesteRepo === null) return;
    setProjeto((atual) => (atual === "" ? pastaDesteRepo.id : atual));
    setNovoProjeto((atual) => (atual === "" ? pastaDesteRepo.id : atual));
  }, [pastaDesteRepo]);

  const lerLista = useCallback(async () => {
    setListaErro(null);
    try {
      setPulls(await api.githubPulls(repo));
    } catch (e) {
      setPulls(null);
      setListaErro(String(e));
    }
  }, [repo]);

  useEffect(() => {
    setNumero(numeroInicial);
    void lerLista();
  }, [lerLista, numeroInicial]);

  // Detalhe e arquivos vêm juntos: o diff é a razão de ser da tela, e um sem o
  // outro deixaria a pessoa lendo um pull request pela metade.
  useEffect(() => {
    if (numero === null) {
      setDetalhe(null);
      setArquivos(null);
      return;
    }
    let vivo = true;
    setLendo(true);
    setDetalheErro(null);
    setArquivos(null);
    void (async () => {
      try {
        const [d, f] = await Promise.all([api.githubPullDetail(repo, numero), api.githubPullFiles(repo, numero)]);
        if (!vivo) return;
        setDetalhe(d);
        setArquivos(f);
      } catch (e) {
        if (!vivo) return;
        setDetalhe(null);
        setArquivos(null);
        setDetalheErro(String(e));
      } finally {
        if (vivo) setLendo(false);
      }
    })();
    return () => {
      vivo = false;
    };
  }, [repo, numero]);

  const escrever = async (nome: string, rota: () => Promise<unknown>, depois?: (r: unknown) => void) => {
    setAcao(nome);
    try {
      const r = await rota();
      pushToast("success", `${nome} concluído no GitHub.`);
      depois?.(r);
      setComentario("");
      setConversa((v) => v + 1);
      await lerLista();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setAcao(null);
    }
  };

  const comentar = () =>
    escrever("Comentário", () => api.githubComment({ repo, number: numero!, body: comentario }));

  const revisar = (evento: "APPROVE" | "REQUEST_CHANGES") =>
    escrever(evento === "APPROVE" ? "Aprovação" : "Pedido de alterações", () =>
      api.githubReview({ repo, number: numero!, event: evento, body: comentario.trim() || null }),
    );

  const mesclar = (metodo: MergeMethod) => {
    const sha = detalhe?.headSha ?? "";
    return escrever(`Merge (${metodo})`, () => api.githubMerge({ repo, number: numero!, method: metodo, sha }), () => {
      setMergeAsk(null);
      setNumero(null);
      setDetalhe(null);
      setArquivos(null);
    });
  };

  const trazerParaMaquina = async () => {
    if (projeto === "" || numero === null) return;
    setAcao("Busca do pull request");
    try {
      const r = await api.gitCheckoutPull(projeto, numero);
      const ramo = r.output.split("\n").find((l) => l.includes("github/pr-")) ?? `github/pr-${numero}`;
      pushToast("success", `Pull request #${numero} está em ${ramo}. Abra a seção Git para trabalhar nele.`);
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setAcao(null);
    }
  };

  // Os ramos que servem de origem são os desta máquina; os de destino são os que
  // o remoto tem. Nada aqui empurra branch nenhuma — isso é leitura.
  useEffect(() => {
    if (!novoAberto) {
      setRamosRemotos(null);
      return;
    }
    let vivo = true;
    api
      .githubBranches(repo)
      .then((r) => {
        if (!vivo) return;
        setRamosRemotos(r);
        setBase((atual) => atual || branchPadrao || (r.includes("main") ? "main" : (r[0] ?? "")));
      })
      .catch((e) => {
        if (vivo) {
          setRamosRemotos([]);
          pushToast("error", String(e));
        }
      });
    return () => {
      vivo = false;
    };
  }, [novoAberto, repo, branchPadrao, pushToast]);

  useEffect(() => {
    if (novoProjeto === "") {
      setRamosLocais(null);
      return;
    }
    let vivo = true;
    api
      .gitBranches(novoProjeto)
      .then((r) => {
        if (!vivo) return;
        const cabeca = r.find((b) => b.isHead);
        setRamosLocais(r);
        setHead((atual) => atual || cabeca?.name || "");
      })
      .catch((e) => {
        if (vivo) {
          setRamosLocais([]);
          pushToast("error", String(e));
        }
      });
    return () => {
      vivo = false;
    };
  }, [novoProjeto, pushToast]);

  const abrirPull = async () => {
    setAcao("Pull request");
    try {
      const criado = await api.githubCreatePull({
        repo,
        title: titulo.trim(),
        head,
        base,
        body: descricao.trim() || null,
        draft: rascunho,
      });
      setNovoAberto(false);
      setTitulo("");
      setDescricao("");
      setRascunho(false);
      setHead("");
      pushToast("success", `Pull request #${criado.number} aberto em ${repo}.`);
      setNumero(criado.number);
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

  if (pulls === null) {
    return (
      <Card className="flex items-center gap-2 p-5 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner className="size-4" /> Lendo os pull requests de {repo}…
      </Card>
    );
  }

  if (pulls.length === 0) {
    return (
      <Card>
        <EmptyState
          icon={<GitBranchIcon className="size-8" />}
          title="Nenhum pull request aberto"
          description={`${repo} não tem pull request aguardando. Se você acabou de enviar um ramo, use “Atualizar”.`}
        >
          <Button size="sm" onClick={() => void lerLista()}>
            <RefreshIcon className="size-3.5" /> Atualizar
          </Button>
          <Button size="sm" variant="primary" onClick={() => setNovoAberto(true)}>
            <PlusIcon className="size-3.5" /> Abrir pull request de um ramo local
          </Button>
        </EmptyState>
      </Card>
    );
  }

  const estado = detalhe ? estado_merge(detalhe) : null;

  return (
    <div className="grid min-h-0 gap-3 lg:grid-cols-[minmax(0,17rem)_minmax(0,1fr)]">
      <Card className="flex max-h-[34rem] flex-col gap-1 overflow-y-auto p-2">
        <div className="flex items-center justify-between gap-2 px-1 py-1">
          <span className="text-xs text-zinc-500 dark:text-zinc-400">{pulls.length} abertos</span>
          <div className="flex items-center gap-1">
            <Button size="sm" variant="ghost" onClick={() => setNovoAberto(true)}>
              <PlusIcon className="size-3.5" /> Abrir PR
            </Button>
            <Button size="sm" variant="ghost" onClick={() => void lerLista()}>
              <RefreshIcon className="size-3.5" />
            </Button>
          </div>
        </div>
        {pulls.map((p) => (
          <button
            key={p.number}
            type="button"
            onClick={() => setNumero(p.number)}
            className={`rounded-lg border px-2.5 py-2 text-left transition ${
              numero === p.number
                ? "border-teal-500/70 bg-teal-500/5"
                : "border-zinc-200 hover:border-zinc-300 dark:border-zinc-800 dark:hover:border-zinc-700"
            }`}
          >
            <p className="truncate text-sm">{p.title}</p>
            <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
              <span className="font-mono">#{p.number}</span>
              <span className="font-mono line-clamp-1">
                {p.headRef} → {p.baseRef}
              </span>
              {p.draft ? <Badge tone="warning">rascunho</Badge> : null}
              {p.author ? <span>@{p.author}</span> : null}
            </p>
          </button>
        ))}
      </Card>

      <Card className="flex min-h-0 flex-col gap-3 p-4">
        {numero === null ? (
          <EmptyState
            icon={<GitBranchIcon className="size-8" />}
            title="Escolha um pull request"
            description="A esquerda lista os abertos; a direita mostra o diff real, a revisão e o merge."
          />
        ) : detalheErro ? (
          <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
            <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {detalheErro}
          </p>
        ) : lendo || detalhe === null ? (
          <div className="flex items-center gap-2 text-sm text-zinc-500 dark:text-zinc-400">
            <Spinner className="size-4" /> Lendo o pull request #{numero}…
          </div>
        ) : (
          <>
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <p className="text-sm font-medium">{detalhe.title}</p>
                <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
                  <span className="font-mono">#{detalhe.number}</span>
                  <span className="font-mono">
                    {detalhe.headRef} → {detalhe.baseRef}
                  </span>
                  <span className="font-mono">{detalhe.headSha.slice(0, 8)}</span>
                  {detalhe.author ? <span>@{detalhe.author}</span> : null}
                  <span>{formatAgo(detalhe.updatedAt)}</span>
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                <Badge tone="success">+{detalhe.additions}</Badge>
                <Badge tone="error">-{detalhe.deletions}</Badge>
                <Badge>{detalhe.changedFiles} arquivos</Badge>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void openUrl(detalhe.url).catch(() => pushToast("error", `Abra manualmente em ${detalhe.url}`))
                  }
                >
                  <ExternalLinkIcon className="size-3.5" /> Site
                </Button>
              </div>
            </div>

            {estado ? (
              <p
                className={`flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs ${
                  estado.tom === "success"
                    ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                    : estado.tom === "error"
                      ? "bg-red-500/10 text-red-600 dark:text-red-400"
                      : estado.tom === "warning"
                        ? "bg-amber-500/10 text-amber-600 dark:text-amber-400"
                        : "bg-zinc-500/10 text-zinc-500 dark:text-zinc-400"
                }`}
              >
                {estado.tom === "success" ? <CheckIcon className="size-3.5" /> : <AlertTriangleIcon className="size-3.5" />}
                {estado.texto}
              </p>
            ) : null}

            {detalhe.body ? (
              <p className="max-h-28 overflow-y-auto whitespace-pre-wrap rounded-lg bg-zinc-100 p-2.5 text-xs text-zinc-600 dark:bg-zinc-900 dark:text-zinc-300">
                {detalhe.body}
              </p>
            ) : null}

            <Field label={`Escrever para o pull request #${detalhe.number}`}>
              <Textarea
                rows={3}
                value={comentario}
                placeholder="comentário, motivo do pedido de alteração ou da aprovação"
                onChange={(e) => setComentario(e.target.value)}
              />
            </Field>

            <div className="flex flex-wrap items-center gap-1.5">
              <Button
                size="sm"
                variant="primary"
                disabled={detalhe.mergeable !== true || acao !== null}
                onClick={() => setMergeAsk("squash")}
              >
                <MergeIcon className="size-3.5" /> Mesclar
              </Button>
              <Button size="sm" disabled={acao !== null} onClick={() => void revisar("APPROVE")}>
                <CheckIcon className="size-3.5" /> Aprovar
              </Button>
              <Button size="sm" disabled={acao !== null || comentario.trim() === ""} onClick={() => void revisar("REQUEST_CHANGES")}>
                <XIcon className="size-3.5" /> Pedir alterações
              </Button>
              <Button size="sm" disabled={acao !== null || comentario.trim() === ""} onClick={() => void comentar()}>
                <CommentIcon className="size-3.5" /> Comentar
              </Button>
            </div>

            <div className="flex flex-wrap items-end gap-2 rounded-lg border border-zinc-200 p-2.5 dark:border-zinc-800">
              <div className="min-w-48">
                <Field label="Trazer para um repositório desta máquina">
                  <Select value={projeto} onChange={(e) => setProjeto(e.target.value === "" ? "" : Number(e.target.value))}>
                    <option value="">escolha o projeto</option>
                    {repositorios.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name}
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
              <Button size="sm" disabled={projeto === "" || acao !== null} onClick={() => void trazerParaMaquina()}>
                <DownloadIcon className="size-3.5" /> Buscar ramo do PR
              </Button>
              {pastaDesteRepo === null ? (
                <Button size="sm" variant="secondary" loading={mapeando} onClick={() => void mapear(repo)}>
                  <FolderIcon className="size-3.5" /> Mapear a pasta deste repo
                </Button>
              ) : (
                <Button size="sm" variant="ghost" onClick={() => onNavigate("git")}>
                  <GitBranchIcon className="size-3.5" /> Abrir na seção Git
                </Button>
              )}
            </div>

            <div className="border-t border-zinc-200 pt-3 dark:border-zinc-800">
              <Conversa repo={repo} number={detalhe.number} versao={conversa} />
            </div>

            {acao ? (
              <p className="flex items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400">
                <Spinner className="size-3" /> {acao} em andamento…
              </p>
            ) : null}

            <div className="min-h-0 flex-1 border-t border-zinc-200 pt-3 dark:border-zinc-800">
              <DiffView
                files={arquivos ?? []}
                loading={lendo}
                emptyHint={`O pull request #${numero} não devolveu patches legíveis — arquivos binários ou grandes demais ficam de fora.`}
              />
            </div>
          </>
        )}

        <Confirm
          open={mergeAsk !== null}
          title={`Mesclar o pull request #${numero ?? ""}`}
          message={`Isto escreve no ramo ${detalhe?.baseRef ?? "base"} de ${repo} e fecha o pull request. Nada é desfeito automaticamente.`}
          confirmLabel="Mesclar agora"
          busy={acao !== null}
          onConfirm={() => {
            if (mergeAsk) void mesclar(mergeAsk);
          }}
          onClose={() => setMergeAsk(null)}
        >
          <Field label="Método">
            <Select value={mergeAsk ?? "squash"} onChange={(e) => setMergeAsk(e.target.value as MergeMethod)}>
              {METODOS.map((m) => (
                <option key={m.valor} value={m.valor}>
                  {m.nome}
                </option>
              ))}
            </Select>
          </Field>
          <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            {METODOS.find((m) => m.valor === (mergeAsk ?? "squash"))?.porque} · entra o commit {detalhe?.headSha.slice(0, 8) ?? "lido agora"}.
          </p>
        </Confirm>
      </Card>

      <Modal open={novoAberto} onClose={acao === "Pull request" ? () => {} : () => setNovoAberto(false)} title="Abrir pull request">
        <div className="grid gap-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="De um repositório desta máquina">
              <Select
                value={novoProjeto === "" ? "" : String(novoProjeto)}
                onChange={(e) => {
                  const v = e.target.value;
                  setNovoProjeto(v === "" ? "" : Number(v));
                  setHead("");
                }}
              >
                <option value="">selecione o projeto</option>
                {repositorios.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Ramo de origem">
              <Select value={head} onChange={(e) => setHead(e.target.value)} disabled={ramosLocais === null}>
                <option value="">{ramosLocais === null ? "escolha o projeto antes" : "escolha o ramo"}</option>
                {(ramosLocais ?? [])
                  .filter((b) => !b.name.includes("/"))
                  .map((b) => (
                    <option key={b.name} value={b.name}>
                      {b.name}
                      {b.isHead ? " (você está aqui)" : ""}
                      {b.remote ? ` · ${b.ahead > 0 ? `${b.ahead} à frente` : b.behind > 0 ? `${b.behind} atrás` : "sincronizado"}` : " · sem remoto"}
                    </option>
                  ))}
              </Select>
            </Field>
          </div>
          <Field label="Ramo de destino">
            <Select value={base} onChange={(e) => setBase(e.target.value)} disabled={ramosRemotos === null}>
              <option value="">{ramosRemotos === null ? "lendo os ramos do remoto…" : "escolha o destino"}</option>
              {(ramosRemotos ?? []).map((b) => (
                <option key={b} value={b}>
                  {b}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Título">
            <Input value={titulo} onChange={(e) => setTitulo(e.target.value)} placeholder="o que este pull request faz" />
          </Field>
          <Field label="Descrição (opcional)">
            <Textarea rows={3} value={descricao} onChange={(e) => setDescricao(e.target.value)} />
          </Field>
          <Checkbox checked={rascunho} onChange={setRascunho} label="Criar como rascunho" />
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setNovoAberto(false)} disabled={acao === "Pull request"}>
            Cancelar
          </Button>
          <Button
            variant="primary"
            loading={acao === "Pull request"}
            disabled={titulo.trim() === "" || head === "" || base === "" || head === base}
            onClick={() => void abrirPull()}
          >
            Abrir no GitHub
          </Button>
        </div>
      </Modal>
    </div>
  );
}
