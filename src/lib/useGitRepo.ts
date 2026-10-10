import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import type {
  BranchInfo,
  GitActionOutcome,
  GitIdentity,
  GitOperation,
  GraphRow,
  PullStrategy,
  RemoteAccess,
  RemoteInfo,
  RepoStatus,
  ResetMode,
  SecretFinding,
  StashEntry,
  TagInfo,
} from "../types";

export type GitNotify = (kind: "success" | "error" | "info", message: string) => void;

const HISTORY_LIMIT = 200;

/** Como a barra de status nomeia cada operação que o Git pode ter parada. */
const NOME_OPERACAO: Record<GitOperation, string> = {
  merge: "o merge",
  rebase: "o rebase",
  "cherry-pick": "o cherry-pick",
  revert: "a reversão",
};

export interface GitRepo {
  status: RepoStatus | null;
  identity: GitIdentity | null;
  branches: BranchInfo[];
  tags: TagInfo[];
  remotes: RemoteInfo[];
  history: GraphRow[];
  /** Trabalhos guardados (`git stash`), na ordem do git. */
  stash: StashEntry[];
  /** Rótulo da operação em andamento, exibido na barra de status. */
  busy: string | null;
  error: string | null;
  reload: () => Promise<void>;
  loadHistory: (limit?: number) => Promise<GraphRow[] | null>;
  refreshStatus: () => Promise<RepoStatus | null>;
  stage: (paths: string[]) => Promise<RepoStatus | null>;
  unstage: (paths: string[]) => Promise<RepoStatus | null>;
  stageHunks: (path: string, indexes: number[], staged: boolean, reverse: boolean) => Promise<number | null>;
  discard: (path: string) => Promise<RepoStatus | null>;
  commit: (message: string, amend: boolean, allowSecrets?: boolean) => Promise<string | null>;
  secretCheck: (scope?: "index" | "push") => Promise<SecretFinding[] | null>;
  scanSecrets: () => Promise<SecretFinding[] | null>;
  untrackFile: (path: string, gitignore?: boolean) => Promise<RepoStatus | null>;
  checkout: (name: string, create: boolean, startPoint?: string | null) => Promise<RepoStatus | null>;
  renameBranch: (from: string, to: string) => Promise<BranchInfo[] | null>;
  deleteBranch: (name: string, force: boolean) => Promise<BranchInfo[] | null>;
  merge: (branch: string) => Promise<GitActionOutcome | null>;
  rebase: (upstream: string) => Promise<GitActionOutcome | null>;
  /** Cancela a operação que o status reporta — o kind vem de `status.operation`. */
  abort: (kind: GitOperation) => Promise<RepoStatus | null>;
  continuePending: (kind: GitOperation) => Promise<GitActionOutcome | null>;
  skipPending: (kind: GitOperation) => Promise<GitActionOutcome | null>;
  resolveConflict: (path: string, side: "ours" | "theirs") => Promise<RepoStatus | null>;
  revert: (commitId: string, mainline?: number | null) => Promise<GitActionOutcome | null>;
  cherryPick: (commitId: string, mainline?: number | null) => Promise<GitActionOutcome | null>;
  /** Grava o trabalho em andamento na lista do git e limpa a pasta. */
  stashPush: (message: string | null, includeUntracked: boolean) => Promise<GitActionOutcome | null>;
  /** `pop` também retira da lista; em conflito o git mantém a entrada. */
  stashApply: (index: number, pop: boolean) => Promise<GitActionOutcome | null>;
  stashDrop: (index: number) => Promise<GitActionOutcome | null>;
  reloadStash: () => Promise<StashEntry[] | null>;
  reset: (mode: ResetMode, target: string) => Promise<GitActionOutcome | null>;
  fetch: (remote?: string | null) => Promise<GitActionOutcome | null>;
  pull: (strategy: PullStrategy) => Promise<GitActionOutcome | null>;
  push: (params: {
    remote?: string | null;
    branch?: string | null;
    setUpstream?: boolean;
    allowSecrets?: boolean;
  }) => Promise<GitActionOutcome | null>;
  publish: (params: {
    remote?: string | null;
    tag?: string | null;
    commands: string[];
    allowSecrets?: boolean;
  }) => Promise<GitActionOutcome | null>;
  unpublishTag: (name: string, remote?: string | null) => Promise<GitActionOutcome | null>;
  /** `git ls-remote` com prazo: diz se esta máquina consegue publicar. */
  checkRemote: (remote?: string | null) => Promise<RemoteAccess | null>;
  createTag: (name: string, target?: string | null, message?: string | null) => Promise<TagInfo[] | null>;
  deleteTag: (name: string) => Promise<TagInfo[] | null>;
  addRemote: (name: string, url: string) => Promise<RemoteInfo[] | null>;
  setRemoteUrl: (name: string, url: string) => Promise<RemoteInfo[] | null>;
  removeRemote: (name: string) => Promise<RemoteInfo[] | null>;
}

/**
 * Estado do repositório Git de um projeto cadastrado.
 *
 * Toda chamada parte do `projectId`; o caminho do repositório é resolvido no
 * backend a partir do banco local, então a interface nunca escolhe em qual pasta
 * executar. Depois de cada mutação a fatia correspondente é recarregada com o
 * git — o painel mostra o resultado real, nunca uma suposição otimista.
 */
export function useGitRepo(projectId: number | null, notify: GitNotify): GitRepo {
  const [status, setStatus] = useState<RepoStatus | null>(null);
  const [identity, setIdentity] = useState<GitIdentity | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [tags, setTags] = useState<TagInfo[]>([]);
  const [remotes, setRemotes] = useState<RemoteInfo[]>([]);
  const [history, setHistory] = useState<GraphRow[]>([]);
  const [stash, setStash] = useState<StashEntry[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  /** Roda a operação com o seletor ocupado e o erro visível; `null` = falhou. */
  const guard = useCallback(
    async <T,>(label: string, fn: (id: number) => Promise<T>): Promise<T | null> => {
      if (projectId == null) {
        setError("Selecione um projeto com repositório Git.");
        return null;
      }
      setBusy(label);
      setError(null);
      try {
        return await fn(projectId);
      } catch (e) {
        const message = String(e);
        if (alive.current) {
          setError(message);
          notify("error", message);
        }
        return null;
      } finally {
        if (alive.current) setBusy(null);
      }
    },
    [notify, projectId],
  );

  const refreshStatus = useCallback(
    () =>
      guard("Atualizando as alterações", (id) => api.gitStatus(id)).then((next) => {
        if (next && alive.current) setStatus(next);
        return next;
      }),
    [guard],
  );

  const loadHistory = useCallback(
    (limit = HISTORY_LIMIT) =>
      guard("Lendo o histórico", (id) => api.gitGraph(id, limit, true)).then((rows) => {
        if (rows && alive.current) setHistory(rows);
        return rows;
      }),
    [guard],
  );

  const loadBranches = useCallback(
    () =>
      guard("Atualizando os ramos", (id) => api.gitBranches(id)).then((list) => {
        if (list && alive.current) setBranches(list);
        return list;
      }),
    [guard],
  );

  const loadStash = useCallback(
    () =>
      guard("Lendo os trabalhos guardados", (id) => api.gitStashList(id)).then((list) => {
        if (list && alive.current) setStash(list);
        return list;
      }),
    [guard],
  );

  const reload = useCallback(async () => {
    if (projectId == null) {
      setStatus(null);
      setIdentity(null);
      setBranches([]);
      setTags([]);
      setRemotes([]);
      setHistory([]);
      setStash([]);
      return;
    }
    const snapshot = await guard("Lendo o repositório", (id) =>
      Promise.all([
        api.gitStatus(id),
        api.gitIdentity(id),
        api.gitBranches(id),
        api.gitTags(id),
        api.gitRemotes(id),
        api.gitGraph(id, HISTORY_LIMIT, true),
        api.gitStashList(id),
      ]),
    );
    if (!snapshot || !alive.current) return;
    const [st, id, br, tg, rm, hist, guardados] = snapshot;
    setStatus(st);
    setIdentity(id);
    setBranches(br);
    setTags(tg);
    setRemotes(rm);
    setHistory(hist);
    setStash(guardados);
  }, [guard, projectId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const statusAction = useCallback(
    (label: string, fn: (id: number) => Promise<RepoStatus>) =>
      guard(label, fn).then((next) => {
        if (next && alive.current) setStatus(next);
        return next;
      }),
    [guard],
  );

  /** Operações que podem deixar o repositório em merge/rebase: recarregam tudo. */
  const reloadAfter = useCallback(
    (label: string, fn: (id: number) => Promise<GitActionOutcome>) =>
      guard(label, fn).then(async (result) => {
        if (result == null) return null;
        await reload();
        if (alive.current && result.output) {
          notify("info", result.output.split("\n").slice(0, 3).join(" · "));
        }
        return result;
      }),
    [guard, notify, reload],
  );

  const stage = useCallback(
    (paths: string[]) => statusAction("Preparando arquivos para o commit", (id) => api.gitStage(id, paths)),
    [statusAction],
  );

  const unstage = useCallback(
    (paths: string[]) => statusAction("Removendo arquivos do staging", (id) => api.gitUnstage(id, paths)),
    [statusAction],
  );

  const stageHunks = useCallback(
    (path: string, indexes: number[], staged: boolean, reverse: boolean) =>
      guard(
        reverse ? "Removendo trechos do staging" : "Preparando trechos para o commit",
        (id) => api.gitStageHunks({ projectId: id, path, indexes, staged, reverse }),
      ).then(async (count) => {
        if (count == null) return null;
        await refreshStatus();
        if (alive.current) {
          notify(
            "success",
            reverse
              ? `${count} trecho(s) saíram do staging de ${path}.`
              : `${count} trecho(s) preparados em ${path}.`,
          );
        }
        return count;
      }),
    [guard, notify, refreshStatus],
  );

  const discard = useCallback(
    (path: string) =>
      guard("Descartando alterações", (id) => api.gitDiscard(id, path)).then(() => refreshStatus()),
    [guard, refreshStatus],
  );

  const commit = useCallback(
    (message: string, amend: boolean, allowSecrets = false) =>
      guard(amend ? "Regravando o último commit" : "Criando o commit", (id) =>
        api.gitCommit(id, message, amend, allowSecrets),
      ).then(async (hash) => {
        if (hash == null) return null;
        await reload();
        if (alive.current) notify("success", amend ? "Último commit regravado." : "Commit criado.");
        return hash;
      }),
    [guard, notify, reload],
  );

  const secretCheck = useCallback(
    (scope: "index" | "push" = "index") =>
      guard(scope === "push" ? "Checando o que o envio publicaria" : "Checando o conteúdo do índice", (id) =>
        api.gitSecretCheck(id, scope),
      ),
    [guard],
  );

  const scanSecrets = useCallback(
    () => guard("Varrendo a pasta do projeto", (id) => api.scanProjectSecrets(id)),
    [guard],
  );

  const untrackFile = useCallback(
    (path: string, gitignore = true) =>
      guard(`Tirando ${path} do índice`, (id) => api.gitUntrackFile(id, path, gitignore)).then((next) => {
        if (next && alive.current) setStatus(next);
        return next;
      }),
    [guard],
  );

  const checkout = useCallback(
    (name: string, create: boolean, startPoint?: string | null) =>
      statusAction(create ? `Criando o ramo ${name}` : `Trocando para ${name}`, (id) =>
        api.gitCheckout(id, name, create, startPoint ?? null),
      ).then(async (next) => {
        if (next == null) return null;
        await loadBranches();
        await loadHistory();
        return next;
      }),
    [loadBranches, loadHistory, statusAction],
  );

  const renameBranch = useCallback(
    (from: string, to: string) =>
      guard(`Renomeando ${from} para ${to}`, (id) => api.gitRenameBranch(id, from, to)).then((list) => {
        if (list && alive.current) setBranches(list);
        return list;
      }),
    [guard],
  );

  const deleteBranch = useCallback(
    (name: string, force: boolean) =>
      guard(`Excluindo o ramo ${name}`, (id) => api.gitDeleteBranch(id, name, force)).then((list) => {
        if (list && alive.current) setBranches(list);
        return list;
      }),
    [guard],
  );

  const merge = useCallback(
    (branch: string) => reloadAfter(`Mesclando ${branch}`, (id) => api.gitMerge(id, branch)),
    [reloadAfter],
  );

  const rebase = useCallback(
    (upstream: string) => reloadAfter(`Rebaseando sobre ${upstream}`, (id) => api.gitRebase(id, upstream)),
    [reloadAfter],
  );

  const abort = useCallback(
    (kind: GitOperation) => statusAction(`Cancelando ${NOME_OPERACAO[kind]}`, (id) => api.gitAbort(id, kind)),
    [statusAction],
  );

  const continuePending = useCallback(
    (kind: GitOperation) =>
      reloadAfter(`Concluindo ${NOME_OPERACAO[kind]}`, (id) => api.gitContinuePending(id, kind)),
    [reloadAfter],
  );

  const skipPending = useCallback(
    (kind: GitOperation) => reloadAfter(`Pulando o passo de ${kind}`, (id) => api.gitSkipPending(id, kind)),
    [reloadAfter],
  );

  const resolveConflict = useCallback(
    (path: string, side: "ours" | "theirs") =>
      statusAction(`Resolvendo conflito em ${path}`, (id) => api.gitResolveConflict(id, path, side)),
    [statusAction],
  );

  /** Desfaz um commit gravando outro no lugar — nada do histórico é reescrito. */
  const revert = useCallback(
    (commitId: string, mainline?: number | null) =>
      reloadAfter(`Revertendo ${commitId.slice(0, 9)}`, (id) => api.gitRevert(id, commitId, mainline ?? null)),
    [reloadAfter],
  );

  /** Reproduz aqui um commit de outro ramo, gravando um commit novo. O ramo de
   * origem fica como está e nada é enviado ao remoto. */
  const cherryPick = useCallback(
    (commitId: string, mainline?: number | null) =>
      reloadAfter(`Reproduzindo ${commitId.slice(0, 9)}`, (id) => api.gitCherryPick(id, commitId, mainline ?? null)),
    [reloadAfter],
  );

  const stashPush = useCallback(
    (message: string | null, includeUntracked: boolean) =>
      reloadAfter("Guardando o trabalho", (id) => api.gitStashPush(id, message, includeUntracked)),
    [reloadAfter],
  );

  const stashApply = useCallback(
    (index: number, pop: boolean) =>
      reloadAfter(pop ? "Aplicando e tirando da lista" : "Aplicando o trabalho guardado", (id) =>
        api.gitStashApply(id, index, pop),
      ),
    [reloadAfter],
  );

  /** Excluir o guardado perde aquele trabalho: a interface confirma antes. */
  const stashDrop = useCallback(
    (index: number) => reloadAfter(`Excluindo o guardado ${index}`, (id) => api.gitStashDrop(id, index)),
    [reloadAfter],
  );

  /** Move a cabeça do ramo. `hard` descarta trabalho, por isso chega aqui só
   * depois da confirmação da interface. */  const reset = useCallback(
    (mode: ResetMode, target: string) =>
      reloadAfter(`Movendo a cabeça do ramo (--${mode})`, (id) => api.gitReset(id, mode, target)),
    [reloadAfter],
  );

  const fetch = useCallback(
    (remote?: string | null) =>
      reloadAfter(remote ? `Buscando de ${remote}` : "Buscando de todos os remotos", (id) =>
        api.gitFetch(id, remote ?? null),
      ),
    [reloadAfter],
  );

  const pull = useCallback(
    (strategy: PullStrategy) => reloadAfter(`Puxando com ${strategy}`, (id) => api.gitPull(id, strategy)),
    [reloadAfter],
  );

  const push = useCallback(
    (params: {
      remote?: string | null;
      branch?: string | null;
      setUpstream?: boolean;
      allowSecrets?: boolean;
    }) => reloadAfter("Enviando commits", (id) => api.gitPush({ ...params, projectId: id })),
    [reloadAfter],
  );

  /** Publicação sem servidor: comandos locais, envio do ramo e tag. O desfecho
   * volta para a janela de publicação mostrar a saída real de cada etapa. */
  const publish = useCallback(
    (params: {
      remote?: string | null;
      tag?: string | null;
      commands: string[];
      allowSecrets?: boolean;
    }) => reloadAfter("Publicando via git", (id) => api.gitPublish({ ...params, projectId: id })),
    [reloadAfter],
  );

  /** Retira a tag do remoto; a local e o histórico permanecem. */
  const unpublishTag = useCallback(
    (name: string, remote?: string | null) =>
      reloadAfter(`Retirando a tag ${name} do remoto`, (id) => api.gitUnpublishTag(id, name, remote ?? null)),
    [reloadAfter],
  );

  const checkRemote = useCallback(
    (remote?: string | null) =>
      guard("Testando o acesso ao remoto", (id) => api.gitCheckRemote(id, remote ?? null)).then((r) => {
        if (r && alive.current) notify(r.ok ? "success" : "error", `${r.remote}: ${r.detail}`);
        return r;
      }),
    [guard, notify],
  );

  const createTag = useCallback(
    (name: string, target?: string | null, message?: string | null) =>
      guard(`Criando a tag ${name}`, (id) => api.gitCreateTag(id, name, target ?? null, message ?? null)).then(
        (list) => {
          if (list && alive.current) setTags(list);
          return list;
        },
      ),
    [guard],
  );

  const deleteTag = useCallback(
    (name: string) =>
      guard(`Excluindo a tag ${name}`, (id) => api.gitDeleteTag(id, name)).then((list) => {
        if (list && alive.current) setTags(list);
        return list;
      }),
    [guard],
  );

  const addRemote = useCallback(
    (name: string, url: string) =>
      guard(`Adicionando o remoto ${name}`, (id) => api.gitAddRemote(id, name, url)).then((list) => {
        if (list && alive.current) setRemotes(list);
        return list;
      }),
    [guard],
  );

  const setRemoteUrl = useCallback(
    (name: string, url: string) =>
      guard(`Atualizando a URL de ${name}`, (id) => api.gitSetRemoteUrl(id, name, url)).then((list) => {
        if (list && alive.current) setRemotes(list);
        return list;
      }),
    [guard],
  );

  const removeRemote = useCallback(
    (name: string) =>
      guard(`Removendo o remoto ${name}`, (id) => api.gitRemoveRemote(id, name)).then((list) => {
        if (list && alive.current) setRemotes(list);
        return list;
      }),
    [guard],
  );

  return {
    status,
    identity,
    branches,
    tags,
    remotes,
    history,
    stash,
    busy,
    error,
    reload,
    loadHistory,
    reloadStash: loadStash,
    refreshStatus,
    stage,
    unstage,
    stageHunks,
    discard,
    commit,
    secretCheck,
    scanSecrets,
    untrackFile,
    checkout,
    renameBranch,
    deleteBranch,
    merge,
    rebase,
    abort,
    continuePending,
    skipPending,
    resolveConflict,
    revert,
    cherryPick,
    stashPush,
    stashApply,
    stashDrop,
    reset,
    fetch,
    pull,
    push,
    publish,
    unpublishTag,
    checkRemote,
    createTag,
    deleteTag,
    addRemote,
    setRemoteUrl,
    removeRemote,
  };
}
