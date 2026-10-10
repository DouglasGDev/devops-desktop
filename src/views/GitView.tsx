import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { useGitRepo } from "../lib/useGitRepo";
import { formatRelative, formatTimestamp } from "../lib/format";
import { useApp } from "../store/AppContext";
import type { CommitDetail, FileDiff, GitEditor, PullStrategy, RemoteAccess, ResetMode, SecretFinding, ViewId } from "../types";
import {
  AlertTriangleIcon,
  CheckIcon,
  CloudIcon,
  CommitIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FolderIcon,
  GitBranchIcon,
  KeyIcon,
  PlusIcon,
  RefreshIcon,
  RocketIcon,
  UndoIcon,
  UploadIcon,
  XIcon,
} from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  Confirm,
  EmptyState,
  Field,
  Input,
  Modal,
  Select,
  SectionTitle,
  Spinner,
} from "../components/ui";
import { BranchesPanel } from "../components/git/BranchesPanel";
import { ChangesPanel } from "../components/git/ChangesPanel";
import { CloneDialog } from "../components/git/CloneDialog";
import { DiffView } from "../components/git/DiffView";
import { HistoryPanel } from "../components/git/HistoryPanel";
import { PublishDialog } from "../components/git/PublishDialog";
import { RemotesPanel } from "../components/git/RemotesPanel";
import { SecretsPanel } from "../components/git/SecretsPanel";
import { TagsPanel } from "../components/git/TagsPanel";

type Tab = "changes" | "history" | "branches" | "tags" | "remotes" | "secrets";

/** O que cada modo de `git reset` faz, do jeito que os testes do backend
 * verificam: soft preserva o índice, mixed solta o índice, hard substitui o
 * conteúdo em trabalho. */
const RESET_LABELS: Record<ResetMode, string> = {
  soft: "soft — mantém as alterações preparadas",
  mixed: "mixed — solta o índice, mantém os arquivos",
  hard: "hard — descarta o conteúdo em trabalho",
};

const RESET_HINTS: Record<ResetMode, string> = {
  soft: "A cabeça do ramo volta para o commit e o índice continua igual: o que estava preparado permanece preparado.",
  mixed: "A cabeça volta e o índice é refeito a partir do commit escolhido. Os arquivos ficam no disco como estavam, só deixam de estar preparados.",
  hard: "A cabeça volta e os arquivos rastreados são substituídos pelo conteúdo do commit. O que não estiver em um commit é perdido — arquivos não rastreados continuam no disco.",
};

const TABS: { id: Tab; label: string }[] = [
  { id: "changes", label: "Alterações" },
  { id: "history", label: "Histórico" },
  { id: "branches", label: "Ramos" },
  { id: "tags", label: "Tags" },
  { id: "remotes", label: "Remotos" },
  { id: "secrets", label: "Segredos" },
];

/** O que a coluna da direita está mostrando. */
type DiffTarget =
  | { kind: "file"; path: string; staged: boolean }
  | { kind: "commit"; id: string }
  | { kind: "compare"; from: string; to: string };

function targetLabel(target: DiffTarget, branch: string | null): string {
  switch (target.kind) {
    case "file":
      return `${target.path} · ${target.staged ? "índice (preparado)" : "disco (sem preparar)"}`;
    case "commit":
      return `commit ${target.id.slice(0, 9)} em relação ao pai`;
    case "compare":
      return `${target.from} → ${target.to}${branch ? ` · comparando no ramo ${branch}` : ""}`;
  }
}

export function GitView({ onNavigate }: { onNavigate: (v: ViewId) => void }) {
  const { projects, settings, pushToast, reloadProjects } = useApp();
  const gitProjects = useMemo(() => projects.filter((p) => p.isGitRepo), [projects]);
  const [projectId, setProjectId] = useState<number | null>(gitProjects[0]?.id ?? null);
  const [tab, setTab] = useState<Tab>("changes");
  const [target, setTarget] = useState<DiffTarget | null>(null);
  const [diffFiles, setDiffFiles] = useState<FileDiff[]>([]);
  const [diffLoading, setDiffLoading] = useState(false);
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [cloning, setCloning] = useState(false);
  const [pullStrategy, setPullStrategy] = useState<PullStrategy>("ff-only");
  const [compareOpen, setCompareOpen] = useState(false);
  const [compareFrom, setCompareFrom] = useState("");
  const [compareTo, setCompareTo] = useState("");
  const [pushBlock, setPushBlock] = useState<SecretFinding[] | null>(null);
  const [publishing, setPublishing] = useState(false);
  const [revertAsk, setRevertAsk] = useState<CommitDetail | null>(null);
  const [resetAsk, setResetAsk] = useState<{ commit: string; subject: string; mode: ResetMode } | null>(null);
  const [acesso, setAcesso] = useState<RemoteAccess | null>(null);
  const [abrindo, setAbrindo] = useState(false);

  const git = useGitRepo(projectId, pushToast);
  const status = git.status;
  const branch = status?.branch ?? null;
  const editor = (settings.git_editor ?? "system") as GitEditor;

  // O veredito de acesso pertence a um repositório: trocar o projeto o invalida.
  useEffect(() => {
    setAcesso(null);
  }, [projectId]);

  useEffect(() => {
    if (gitProjects.length === 0) {
      if (projectId !== null) setProjectId(null);
      return;
    }
    if (projectId === null || !gitProjects.some((p) => p.id === projectId)) {
      setProjectId(gitProjects[0].id);
    }
  }, [gitProjects, projectId]);

  // O diff é sempre lido do git. Quando o status muda (staging, commit, troca de
  // ramo) a consulta é refeita, para a coluna da direita nunca divergir do índice.
  useEffect(() => {
    if (projectId == null || target == null) {
      setDiffFiles([]);
      setDetail(null);
      return;
    }
    let cancelled = false;
    setDiffLoading(true);
    const load = async () => {
      try {
        const files =
          target.kind === "file"
            ? await api.gitDiff({ projectId, path: target.path, staged: target.staged })
            : target.kind === "commit"
              ? await api.gitDiff({ projectId, commit: target.id })
              : await api.gitDiff({ projectId, from: target.from, to: target.to });
        if (cancelled) return;
        setDiffFiles(files);
        if (target.kind === "commit") {
          const found = await api.gitCommitDetail(projectId, target.id);
          if (!cancelled) setDetail(found);
        } else {
          setDetail(null);
        }
      } catch (e) {
        if (cancelled) return;
        setDiffFiles([]);
        setDetail(null);
        pushToast("error", String(e));
      } finally {
        if (!cancelled) setDiffLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
    // status é a assinatura de "alguma coisa mudou no repositório"
  }, [projectId, target, status, pushToast]);

  const openFile = (path: string, staged: boolean) => setTarget({ kind: "file", path, staged });

  const openHistoryCommit = (id: string) => {
    setTarget({ kind: "commit", id });
  };

  const toggleHunks =
    target?.kind === "file"
      ? (path: string, indexes: number[]) => git.stageHunks(path, indexes, target.staged, target.staged)
      : undefined;

  const openInEditor = (path?: string) => {
    if (projectId == null) return;
    api
      .openInEditor(projectId, path ?? null)
      .then(() => pushToast("success", path ? `Abrindo ${path} no ${editor}.` : "Repositório aberto no editor."))
      .catch((e) => pushToast("error", String(e)));
  };

  const runCompare = () => {
    const from = compareFrom.trim();
    const to = compareTo.trim();
    if (from === "" || to === "") return;
    setTarget({ kind: "compare", from, to });
    setCompareOpen(false);
    setTab("history");
  };

  const doRevert = async (detail: CommitDetail) => {
    setRevertAsk(null);
    // merge tem dois pais: o git exige dizer qual deles continua.
    const mainline = detail.commit.parents.length > 1 ? 1 : null;
    await git.revert(detail.commit.id, mainline);
  };

  const doReset = async (ask: { commit: string; mode: ResetMode }) => {
    setResetAsk(null);
    const feito = await git.reset(ask.mode, ask.commit);
    // o commit escolhido pode ter saído do alcance; fechar o diff evita leitura velha
    if (feito) setTarget(null);
  };

  // §8: antes de publicar, o conteúdo do HEAD é verificado. Se houver candidato
  // a segredo no que o remoto ainda não tem, nada é enviado até a pessoa decidir.
  const runPush = async (allowSecrets = false) => {
    if (!allowSecrets) {
      const found = await git.secretCheck("push");
      if (found === null) return;
      const blocked = found.filter((f) => f.severity === "block");
      if (blocked.length > 0) {
        setPushBlock(blocked);
        return;
      }
    }
    setPushBlock(null);
    await git.push({ allowSecrets });
  };

  // Abrir uma pasta que já existe no computador: o seletor nativo devolve o
  // caminho, o backend canoniciza a pasta, sonda se ela tem um repositório e a
  // cadastra. Uma pasta sem `.git` não vira projeto — é desfeita na hora, para a
  // lista continuar mostrando só o que realmente existe.
  const abrirPastaLocal = async () => {
    setAbrindo(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Selecionar a pasta do repositório" });
      if (typeof dir !== "string" || dir.trim() === "") return;
      const novo = await api.addProject(dir, null);
      if (!novo.isGitRepo) {
        await api.deleteProject(novo.id);
        pushToast("error", `A pasta escolhida não é um repositório Git — “${novo.name}” não foi cadastrada.`);
        return;
      }
      await reloadProjects();
      setProjectId(novo.id);
      setTarget(null);
      setTab("changes");
      pushToast("success", `Repositório “${novo.name}” aberto.`);
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setAbrindo(false);
    }
  };

  if (gitProjects.length === 0) {
    return (
      <Card>
        <EmptyState
          icon={<GitBranchIcon className="size-8" />}
          title="Nenhum repositório aberto"
          description="Aponte uma pasta do seu computador que já seja um repositório, clone um repositório novo para uma pasta, ou cadastre a pasta em Projetos."
        >
          <Button variant="primary" disabled={abrindo} onClick={() => void abrirPastaLocal()}>
            {abrindo ? <Spinner className="size-4" /> : <FolderIcon className="size-4" />} Abrir pasta local
          </Button>
          <Button disabled={abrindo} onClick={() => setCloning(true)}>
            <PlusIcon className="size-4" /> Clonar repositório
          </Button>
          <Button disabled={abrindo} onClick={() => onNavigate("projects")}>
            Ir para Projetos
          </Button>
        </EmptyState>
        <CloneDialog open={cloning} onClose={() => setCloning(false)} />
      </Card>
    );
  }

  const project = gitProjects.find((p) => p.id === projectId) ?? null;
  const dirty = status ? status.files.length : 0;
  const ahead = status?.ahead ?? 0;
  const behind = status?.behind ?? 0;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <Card className="p-3">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-56">
            <Field label="Repositório">
              <Select
                value={projectId ?? ""}
                onChange={(e) => {
                  setProjectId(Number(e.target.value));
                  setTarget(null);
                  setTab("changes");
                }}
              >
                {gitProjects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          {branch !== null || status?.detachedAt ? (
            <div className="flex items-end gap-2 pb-1">
              <Badge tone="accent">
                <GitBranchIcon className="size-3" /> {branch ?? `HEAD avulso em ${status?.detachedAt ?? ""}`}
              </Badge>
              {status?.upstream ? (
                <span className="flex items-center gap-1 text-[11px] text-zinc-500 dark:text-zinc-400">
                  {status.upstream}
                  {ahead > 0 ? <Badge tone="success">{ahead} para enviar</Badge> : null}
                  {behind > 0 ? <Badge tone="warning">{behind} para trazer</Badge> : null}
                </span>
              ) : (
                <Badge>sem remoto acompanhante</Badge>
              )}
            </div>
          ) : null}
          <div className="ml-auto flex flex-wrap items-center gap-1.5">
            <Button size="sm" disabled={git.busy !== null} onClick={() => void git.reload()}>
              <RefreshIcon className={`size-3.5 ${git.busy === "Lendo o repositório" ? "animate-spin" : ""}`} /> Atualizar
            </Button>
            <Button size="sm" disabled={git.busy !== null} onClick={() => void git.fetch(null)} title="git fetch --all --prune">
              <CloudIcon className="size-3.5" /> Buscar
            </Button>
            <Select
              aria-label="Estratégia do pull"
              className="w-32 py-1 text-xs"
              value={pullStrategy}
              onChange={(e) => setPullStrategy(e.target.value as PullStrategy)}
            >
              <option value="ff-only">só fast-forward</option>
              <option value="merge">merge</option>
              <option value="rebase">rebase</option>
            </Select>
            <Button size="sm" disabled={git.busy !== null} onClick={() => void git.pull(pullStrategy)} title="git pull">
              <DownloadIcon className="size-3.5" /> Puxar
            </Button>
            <Button
              size="sm"
              disabled={git.busy !== null || !status?.upstream}
              onClick={() => void runPush()}
              title={status?.upstream ? "Enviar os commits para o remoto" : "Cadastre um remoto ou acompanhe um ramo remoto antes de enviar"}
            >
              <UploadIcon className="size-3.5" /> Enviar
            </Button>
            <Button
              size="sm"
              variant="primary"
              disabled={git.busy !== null || git.remotes.length === 0}
              onClick={() => setPublishing(true)}
              title={
                git.remotes.length === 0
                  ? "Cadastre um remoto em Remotos antes de publicar"
                  : "Rodar os comandos do projeto, enviar o ramo e marcar com uma tag — sem precisar de servidor"
              }
            >
              <RocketIcon className="size-3.5" /> Publicar
            </Button>
            <Button
              size="sm"
              disabled={git.busy !== null || git.remotes.length === 0}
              onClick={() => void git.checkRemote().then(setAcesso)}
              title={
                git.remotes.length === 0
                  ? "Cadastre um remoto em Remotos antes de testar o acesso"
                  : "Perguntar ao remoto (git ls-remote) se esta máquina já está liberada para publicar"
              }
            >
              <KeyIcon className="size-3.5" /> Testar acesso
            </Button>
            <Button size="sm" onClick={() => setCloning(true)}>
              <PlusIcon className="size-3.5" /> Clonar
            </Button>
          </div>
        </div>
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-zinc-500 dark:text-zinc-400">
          <span className="truncate font-mono" title={project?.localPath}>
            {project?.localPath}
          </span>
          {dirty > 0 ? <Badge tone="warning">{dirty} alteração(ões)</Badge> : <Badge tone="success">limpo</Badge>}
          {status && status.conflictCount > 0 ? <Badge tone="error">{status.conflictCount} em conflito</Badge> : null}
          <Button size="sm" variant="ghost" disabled={git.busy !== null} onClick={() => openInEditor()}>
            <ExternalLinkIcon className="size-3.5" /> Abrir no editor
          </Button>
          {git.busy ? (
            <span className="flex items-center gap-1.5 text-teal-700 dark:text-teal-300">
              <Spinner className="size-3.5" /> {git.busy}…
            </span>
          ) : null}
        </div>
        {git.error ? (
          <p className="mt-2 rounded-lg bg-red-500/10 px-2.5 py-1.5 text-xs text-red-700 dark:text-red-300">{git.error}</p>
        ) : null}
        {acesso ? (
          <p
            className={`mt-2 flex items-start gap-2 rounded-lg px-2.5 py-1.5 text-xs ${
              acesso.ok
                ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                : "bg-amber-500/10 text-amber-700 dark:text-amber-300"
            }`}
          >
            {acesso.ok ? (
              <CheckIcon className="mt-0.5 size-3.5 shrink-0" />
            ) : (
              <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />
            )}
            <span className="flex-1">
              {acesso.remote}: {acesso.detail}
            </span>
            <button type="button" className="shrink-0 underline decoration-dotted" onClick={() => setAcesso(null)}>
              fechar
            </button>
          </p>
        ) : null}
      </Card>

      {pushBlock !== null ? (
        <Card className="border-red-400/60 bg-red-50/60 p-3 dark:border-red-900/60 dark:bg-red-950/20">
          <p className="flex items-center gap-2 text-xs font-medium text-red-700 dark:text-red-300">
            <AlertTriangleIcon className="size-4" />
            {pushBlock.length} arquivo(s) com possível segredo no que este envio publicaria. Nada foi enviado.
          </p>
          <ul className="mt-1.5 space-y-0.5">
            {pushBlock.slice(0, 6).map((f) => (
              <li key={`${f.path}#${f.reason}`} className="truncate font-mono text-[11px]" title={`${f.path} — ${f.reason}`}>
                {f.path} — {f.reason}
              </li>
            ))}
            {pushBlock.length > 6 ? (
              <li className="text-[11px] text-zinc-600 dark:text-zinc-400">e mais {pushBlock.length - 6} caminho(s)</li>
            ) : null}
          </ul>
          <p className="mt-1.5 text-[11px] text-red-800/80 dark:text-red-200/80">
            Antes de enviar, tire do índice ou revogue a credencial no provedor. Um segredo que já está
            em commits continua no histórico depois do envio — reescrever histórico remoto não é algo
            que esta aplicação faça por conta própria.
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" onClick={() => setPushBlock(null)}>
              Cancelar
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setPushBlock(null);
                setTab("secrets");
              }}
            >
              Ver na aba Segredos
            </Button>
            <Button size="sm" variant="danger" disabled={git.busy !== null} onClick={() => void runPush(true)}>
              <UploadIcon className="size-3.5" /> Enviar assim mesmo
            </Button>
          </div>
        </Card>
      ) : null}

      <div className="flex min-h-0 flex-1 gap-4">
        <div className="flex w-[400px] shrink-0 flex-col gap-2 overflow-y-auto pr-1">
          <div className="flex gap-1">
            {TABS.map(({ id, label }) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
                  tab === id
                    ? "bg-teal-600/10 text-teal-700 dark:bg-teal-400/10 dark:text-teal-300"
                    : "text-zinc-500 hover:bg-zinc-200/60 dark:text-zinc-400 dark:hover:bg-zinc-800/60"
                }`}
              >
                {label}
                {id === "changes" && dirty > 0 ? <span className="ml-1 text-[10px]">({dirty})</span> : null}
              </button>
            ))}
          </div>

          {status === null && git.busy === "Lendo o repositório" ? (
            <div className="flex items-center gap-2 py-8 text-sm text-zinc-500">
              <Spinner /> Lendo o repositório…
            </div>
          ) : null}

          {status !== null && tab === "changes" ? (
            <ChangesPanel git={git} status={status} selected={target?.kind === "file" ? { path: target.path, staged: target.staged } : null} onSelectFile={openFile} />
          ) : null}

          {tab === "history" ? (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <SectionTitle>Histórico de commits</SectionTitle>
                <Button size="sm" variant="ghost" className="ml-auto" disabled={git.busy !== null} onClick={() => setCompareOpen(true)}>
                  Comparar dois pontos
                </Button>
              </div>
              <HistoryPanel
                history={git.history}
                selectedId={target?.kind === "commit" ? target.id : null}
                onSelect={openHistoryCommit}
                busy={git.busy !== null}
                onLoadMore={(limit) => void git.loadHistory(limit)}
              />
            </div>
          ) : null}

          {tab === "branches" ? <BranchesPanel git={git} branches={git.branches} /> : null}
          {tab === "tags" ? <TagsPanel git={git} tags={git.tags} branches={git.branches} /> : null}
          {tab === "remotes" ? <RemotesPanel git={git} remotes={git.remotes} /> : null}
          {tab === "secrets" ? <SecretsPanel git={git} /> : null}
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-2 overflow-y-auto">
          <div className="flex flex-wrap items-center gap-2">
            <SectionTitle>Diff</SectionTitle>
            {target ? (
              <>
                <span className="min-w-0 truncate text-xs text-zinc-500 dark:text-zinc-400" title={targetLabel(target, branch)}>
                  {targetLabel(target, branch)}
                </span>
                <Button size="sm" variant="ghost" className="ml-auto" onClick={() => setTarget(null)} title="Fechar o diff">
                  <XIcon className="size-3.5" />
                </Button>
              </>
            ) : (
              <p className="text-xs text-zinc-400 dark:text-zinc-500">
                Escolha um arquivo, um commit ou compare dois pontos para ver as linhas alteradas.
              </p>
            )}
          </div>

          {detail ? (
            <Card className="p-3">
              <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
                <CommitIcon className="size-4 text-teal-600" />
                {detail.commit.subject}
                <Badge>{detail.commit.shortId}</Badge>
              </p>
              <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">
                {detail.commit.authorName} &lt;{detail.commit.authorEmail}&gt; ·{" "}
                {formatTimestamp(detail.commit.unixDate)} ({formatRelative(detail.commit.unixDate)})
              </p>
              {detail.commit.body ? (
                <p className="mt-2 whitespace-pre-wrap text-xs text-zinc-600 dark:text-zinc-300">{detail.commit.body}</p>
              ) : null}
              <p className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
                <span className="font-mono text-emerald-600 dark:text-emerald-400">+{detail.totalAdditions}</span>
                <span className="font-mono text-red-600 dark:text-red-400">-{detail.totalDeletions}</span>
                <span>{detail.files.length} arquivo(s)</span>
                {detail.commit.parents.length > 1 ? <Badge tone="warning">merge de {detail.commit.parents.length} pais</Badge> : null}
              </p>
              <ul className="mt-2 space-y-0.5">
                {detail.files.map((f) => (
                  <li key={f.path} className="flex items-center gap-2">
                    <Badge tone={f.kind === "adicionado" ? "success" : f.kind === "removido" ? "error" : "accent"}>{f.kind}</Badge>
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px]">{f.path}</span>
                  </li>
                ))}
              </ul>
              <div className="mt-3 flex flex-wrap gap-2 border-t border-zinc-200 pt-3 dark:border-zinc-800">
                <Button
                  size="sm"
                  disabled={git.busy !== null}
                  onClick={() => setRevertAsk(detail)}
                  title="git revert — cria um commit novo que desfaz este"
                >
                  <UndoIcon className="size-3.5" /> Reverter este commit
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={git.busy !== null}
                  onClick={() => setResetAsk({ commit: detail.commit.id, subject: detail.commit.subject, mode: "mixed" })}
                  title="git reset — move a cabeça do ramo de volta para este commit"
                >
                  Voltar para este commit
                </Button>
              </div>
            </Card>
          ) : null}

          {target === null ? (
            <Card className="flex min-h-40 items-center justify-center">
              <EmptyState
                icon={<CommitIcon className="size-8" />}
                title="Nenhuma comparação aberta"
                description="As alterações, o histórico e os commits aparecem à esquerda; o conteúdo do diff vem aqui."
              />
            </Card>
          ) : (
            <DiffView
              files={diffFiles}
              loading={diffLoading}
              emptyHint={
                target.kind === "file"
                  ? "Este arquivo não tem mais diferenças nesse lado — ele foi preparado, descartado ou já entrou em um commit."
                  : "O git não retornou linhas alteradas para essa comparação."
              }
              onToggleHunks={toggleHunks}
              onOpenFile={target.kind === "commit" || target.kind === "compare" ? undefined : (path) => openInEditor(path)}
            />
          )}
        </div>
      </div>

      <CloneDialog open={cloning} onClose={() => setCloning(false)} />

      {projectId !== null ? (
        <PublishDialog git={git} projectId={projectId} open={publishing} onClose={() => setPublishing(false)} />
      ) : null}

      <Confirm
        open={revertAsk !== null}
        title="Reverter este commit"
        message={
          revertAsk
            ? `Será criado um commit novo que desfaz as mudanças de “${revertAsk.commit.subject}” (${revertAsk.commit.shortId}). ` +
              `O histórico não é reescrito: o commit original continua no gráfico e a reversão vai por cima dele.` +
              (revertAsk.commit.parents.length > 1
                ? ` Como é um merge, a reversão segue o pai 1 — o ramo que recebeu a mesclagem.`
                : ``) +
              (dirty > 0
                ? ` Este repositório tem ${dirty} alteração(ões) em andamento; se elas se sobrepuserem ao que a reversão toca, o git recusa e nada é gravado.`
                : ``)
            : ""
        }
        confirmLabel="Criar a reversão"
        busy={git.busy !== null}
        onConfirm={() => revertAsk && void doRevert(revertAsk)}
        onClose={() => setRevertAsk(null)}
      />

      <Confirm
        open={resetAsk !== null}
        title="Voltar a cabeça do ramo"
        message={
          resetAsk
            ? `A cabeça do ramo ${branch ?? "(ramo sem nome)"} está em ${git.history[0]?.commit.shortId ?? "?"} e passaria para “${resetAsk.subject}” (${resetAsk.commit.slice(0, 9)}). ` +
              RESET_HINTS[resetAsk.mode] +
              (resetAsk.mode === "hard" && dirty > 0
                ? ` ATENÇÃO: ${dirty} alteração(ões) em andamento seriam descartadas.`
                : ``) +
              (status?.upstream
                ? ` O ramo tem remoto acompanhante (${status.upstream}): voltar a cabeça para trás cria divergência, e o envio normal seria recusado pelo git. Reescrever o remoto não é algo que esta aplicação faça.`
                : ``)
            : ""
        }
        confirmLabel={resetAsk?.mode === "hard" ? "Voltar e descartar" : "Voltar"}
        busy={git.busy !== null}
        onConfirm={() => resetAsk && void doReset(resetAsk)}
        onClose={() => setResetAsk(null)}
      >
        {resetAsk ? (
          <Field label="Modo" hint="O que acontece com o que está preparado e com o que está em trabalho.">
            <Select
              value={resetAsk.mode}
              onChange={(e) => setResetAsk({ ...resetAsk, mode: e.target.value as ResetMode })}
            >
              {(Object.keys(RESET_LABELS) as ResetMode[]).map((m) => (
                <option key={m} value={m}>
                  {RESET_LABELS[m]}
                </option>
              ))}
            </Select>
          </Field>
        ) : null}
      </Confirm>

      <Modal open={compareOpen} onClose={() => setCompareOpen(false)} title="Comparar dois pontos do histórico">
        <div className="grid gap-3">
          <Field label="Origem" hint="Ramo, tag ou hash. Ex.: main, v1.2.0, 9f3c1ab.">
            <Input value={compareFrom} onChange={(e) => setCompareFrom(e.target.value)} placeholder="main" />
          </Field>
          <Field label="Destino" hint="A comparação mostra o que mudou de um ponto para o outro.">
            <Input value={compareTo} onChange={(e) => setCompareTo(e.target.value)} placeholder={branch ?? "cabeça do ramo"} />
          </Field>
          <div className="flex flex-wrap gap-1.5">
            {git.branches.slice(0, 12).map((b) => (
              <button
                key={`${b.remote ?? "l"}:${b.name}`}
                onClick={() => (setCompareFrom(b.name), setCompareTo(branch ?? b.name))}
                className="rounded-md border border-zinc-200 px-2 py-0.5 text-[11px] transition hover:bg-zinc-100 dark:border-zinc-700 dark:hover:bg-zinc-800"
              >
                {b.name}
              </button>
            ))}
          </div>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setCompareOpen(false)}>Cancelar</Button>
          <Button variant="primary" disabled={compareFrom.trim() === "" || compareTo.trim() === ""} onClick={runCompare}>
            Comparar
          </Button>
        </div>
      </Modal>
    </div>
  );
}
