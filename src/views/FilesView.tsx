import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, onTransferProgress } from "../lib/api";
import {
  formatBytes,
  formatDateTime,
  formatEta,
  formatRate,
  joinRemotePath,
  parentRemotePath,
  remoteBasename,
} from "../lib/format";
import { useApp } from "../store/AppContext";
import type { RemoteEntry, Server, TransferInfo, TransferProgress, ViewId } from "../types";
import {
  AlertTriangleIcon,
  FolderIcon,
  PencilIcon,
  PlusIcon,
  RefreshIcon,
  TrashIcon,
  UploadIcon,
  XIcon,
} from "../components/Icons";
import {
  AUTH_LABELS,
  Badge,
  Button,
  Card,
  Confirm,
  EmptyState,
  ENV_LABELS,
  ENV_TONES,
  Field,
  Input,
  Modal,
  Select,
  Textarea,
} from "../components/ui";

type Editor = {
  path: string;
  content: string;
  saving: boolean;
};

type PendingOp =
  | { kind: "delete"; entry: RemoteEntry }
  | { kind: "rename"; entry: RemoteEntry }
  | { kind: "chmod"; entry: RemoteEntry }
  | { kind: "mkdir" }
  | null;

type TransferRow = {
  id: number;
  kind: "upload" | "download";
  remotePath: string;
  localPath: string;
  serverName: string;
  startedAtLabel: string;
  fileName: string;
  pct: number;
  transferred: number;
  total: number | null;
  rate: number | null;
  eta: number | null;
  done: boolean;
  ok: boolean;
  error: string | null;
};

function defaultStartPath(server: Server | null): string {
  // Um caminho inicial razoável: home do usuário não é determinável sem
  // uma chamada ssh; usamos "/" — o servidor aceita ls em qualquer raiz.
  void server;
  return "/";
}

export function FilesView({ onNavigate }: { onNavigate: (v: ViewId) => void }) {
  const { servers, settings, pushToast } = useApp();
  const confirmDestructive = settings.confirm_destructive !== "false";

  const [serverId, setServerId] = useState<number | null>(servers[0]?.id ?? null);
  const [path, setPath] = useState<string>("/");
  const [pathDraft, setPathDraft] = useState<string>("/");
  const [entries, setEntries] = useState<RemoteEntry[]>([]);
  const [selected, setSelected] = useState<RemoteEntry | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingOp>(null);
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [transfers, setTransfers] = useState<Map<number, TransferRow>>(new Map());
  const transfersRef = useRef(transfers);
  transfersRef.current = transfers;

  const server = useMemo(() => servers.find((s) => s.id === serverId) ?? null, [servers, serverId]);

  useEffect(() => {
    if (serverId == null && servers.length > 0) {
      setServerId(servers[0].id);
      setPath(defaultStartPath(servers[0]));
      setPathDraft(defaultStartPath(servers[0]));
    }
    if (serverId != null && !servers.some((s) => s.id === serverId)) {
      setServerId(servers[0]?.id ?? null);
      setEntries([]);
    }
  }, [servers, serverId]);

  const refresh = useCallback(
    async (dir?: string) => {
      if (serverId == null) return;
      const target = dir ?? path;
      setLoading(true);
      setError(null);
      try {
        const list = await api.sftpList(serverId, target);
        setEntries(list);
        setSelected(null);
        setPath(target);
        setPathDraft(target);
      } catch (e) {
        const msg = String(e);
        setError(msg);
        pushToast("error", `Falha ao listar: ${msg}`);
      } finally {
        setLoading(false);
      }
    },
    [serverId, path, pushToast],
  );

  useEffect(() => {
    if (serverId != null && entries.length === 0 && !loading && !error) {
      void refresh(defaultStartPath(server));
    }
    // executa apenas na primeira seleção de servidor
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverId]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    onTransferProgress((p: TransferProgress) => {
      setTransfers((prev) => {
        const next = new Map(prev);
        const existing = next.get(p.transferId);
        const base: TransferRow = existing ?? {
          id: p.transferId,
          kind: "upload",
          remotePath: "",
          localPath: "",
          serverName: "",
          startedAtLabel: new Date().toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }),
          fileName: p.fileName,
          pct: p.pct,
          transferred: p.transferred,
          total: p.total,
          rate: p.rate,
          eta: p.eta,
          done: p.done,
          ok: p.ok,
          error: p.error,
        };
        next.set(p.transferId, {
          ...base,
          fileName: p.done ? base.fileName : p.fileName || base.fileName,
          pct: p.done ? (p.ok ? 100 : base.pct) : p.pct,
          transferred: p.done ? base.transferred : p.transferred,
          total: p.total ?? base.total,
          rate: p.rate ?? base.rate,
          eta: p.eta ?? base.eta,
          done: p.done,
          ok: p.ok,
          error: p.error,
        });
        return next;
      });
      if (p.done) {
        if (p.ok) pushToast("success", "Transferência concluída.");
        else if (p.error) pushToast("error", p.error);
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [pushToast]);

  useEffect(() => {
    api
      .transfersList()
      .then((actives: TransferInfo[]) => {
        if (actives.length === 0) return;
        setTransfers((prev) => {
          const next = new Map(prev);
          for (const a of actives) {
            if (next.has(a.id)) continue;
            next.set(a.id, {
              id: a.id,
              kind: a.kind,
              remotePath: a.remotePath,
              localPath: a.localPath,
              serverName: a.serverName,
              startedAtLabel: new Date().toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }),
              fileName: remoteBasename(a.kind === "upload" ? a.remotePath : a.localPath),
              pct: 0,
              transferred: 0,
              total: null,
              rate: null,
              eta: null,
              done: false,
              ok: false,
              error: null,
            });
          }
          return next;
        });
      })
      .catch(() => {});
  }, []);

  const enterDir = (entry: RemoteEntry) => {
    if (!entry.isDir) return;
    void refresh(entry.path);
  };

  const goUp = () => {
    const parent = parentRemotePath(path);
    if (parent != null) void refresh(parent);
  };

  const onOpenFile = async (entry: RemoteEntry) => {
    if (entry.isDir) {
      enterDir(entry);
      return;
    }
    if (serverId == null) return;
    // só abre arquivos de texto até 4 MB; binário vai pelo caminho de download
    const size = entry.size ?? 0;
    if (size > 4_000_000) {
      pushToast("info", "Arquivo grande demais para edição — use Baixar.");
      return;
    }
    try {
      const content = await api.sftpReadText(serverId, entry.path);
      setEditor({ path: entry.path, content, saving: false });
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const saveEditor = async () => {
    if (!editor || serverId == null) return;
    setEditor({ ...editor, saving: true });
    try {
      await api.sftpWriteText(serverId, editor.path, editor.content);
      pushToast("success", "Arquivo salvo.");
      setEditor(null);
      void refresh();
    } catch (e) {
      pushToast("error", String(e));
      setEditor((prev) => (prev ? { ...prev, saving: false } : prev));
    }
  };

  const uploadFiles = async () => {
    if (serverId == null) return;
    const picked = await open({ multiple: true, directory: false, title: "Selecione arquivos para enviar" });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    for (const localPath of list) {
      const name = localPath.replace(/\\/g, "/").split("/").pop() ?? "";
      if (!name) continue;
      const remote = joinRemotePath(path, name);
      try {
        const id = await api.transferStart({ serverId, kind: "upload", remotePath: remote, localPath, recursive: false });
        setTransfers((prev) => {
          const next = new Map(prev);
          next.set(id, {
            id,
            kind: "upload",
            remotePath: remote,
            localPath,
            serverName: server?.name ?? "",
            startedAtLabel: new Date().toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }),
            fileName: name,
            pct: 0,
            transferred: 0,
            total: null,
            rate: null,
            eta: null,
            done: false,
            ok: false,
            error: null,
          });
          return next;
        });
      } catch (e) {
        pushToast("error", String(e));
      }
    }
  };

  const downloadFile = async (entry: RemoteEntry) => {
    if (serverId == null) return;
    const picked = await open({ directory: true, title: "Escolha onde salvar" });
    if (!picked) return;
    const dir = (Array.isArray(picked) ? picked[0] : picked).replace(/\\/g, "/");
    const localPath = `${dir.endsWith("/") ? dir : `${dir}/`}${entry.name}`;
    try {
      const id = await api.transferStart({
        serverId,
        kind: "download",
        remotePath: entry.path,
        localPath,
        recursive: false,
      });
      setTransfers((prev) => {
        const next = new Map(prev);
        next.set(id, {
          id,
          kind: "download",
          remotePath: entry.path,
          localPath,
          serverName: server?.name ?? "",
          startedAtLabel: new Date().toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }),
          fileName: entry.name,
          pct: 0,
          transferred: 0,
          total: null,
          rate: null,
          eta: null,
          done: false,
          ok: false,
          error: null,
        });
        return next;
      });
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const applyPending = async (value: string) => {
    if (serverId == null || !pending) return;
    setBusy(true);
    try {
      if (pending.kind === "mkdir") {
        const target = joinRemotePath(path, value.trim());
        await api.sftpMkdir(serverId, target);
        pushToast("success", "Diretório criado.");
      } else if (pending.kind === "rename") {
        const target = joinRemotePath(parentRemotePath(pending.entry.path) ?? path, value.trim());
        await api.sftpRename(serverId, pending.entry.path, target);
        pushToast("success", "Renomeado.");
      } else if (pending.kind === "chmod") {
        await api.sftpChmod(serverId, pending.entry.path, value.trim());
        pushToast("success", "Permissão alterada.");
      } else if (pending.kind === "delete") {
        if (pending.entry.isDir) await api.sftpRemoveDir(serverId, pending.entry.path);
        else await api.sftpRemove(serverId, pending.entry.path);
        pushToast("success", "Removido.");
      }
      setPending(null);
      void refresh();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const onConfirmDelete = async (entry: RemoteEntry) => {
    if (serverId == null) return;
    setBusy(true);
    try {
      if (entry.isDir) await api.sftpRemoveDir(serverId, entry.path);
      else await api.sftpRemove(serverId, entry.path);
      pushToast("success", "Removido.");
      void refresh();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const requestDelete = (entry: RemoteEntry) => {
    if (!confirmDestructive) {
      void onConfirmDelete(entry);
    } else {
      setPending({ kind: "delete", entry });
    }
  };

  const transferList = useMemo(() => Array.from(transfers.values()).sort((a, b) => b.id - a.id), [transfers]);

  if (servers.length === 0) {
    return (
      <EmptyState
        icon={<FolderIcon className="size-8" />}
        title="Nenhum servidor cadastrado"
        description="Cadastre um servidor para explorar os arquivos remotos por SFTP."
      >
        <Button variant="primary" onClick={() => onNavigate("servers")}>
          Ir para Servidores
        </Button>
      </EmptyState>
    );
  }

  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-52">
            <Field label="Servidor">
              <Select
                value={serverId ?? ""}
                onChange={(e) => {
                  const id = Number(e.target.value);
                  setServerId(id);
                  setEntries([]);
                  setError(null);
                  const s = servers.find((x) => x.id === id);
                  if (s) {
                    setPath(defaultStartPath(s));
                    setPathDraft(defaultStartPath(s));
                  }
                }}
              >
                {servers.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name} · {s.host}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          {server ? <Badge tone={ENV_TONES[server.environment] ?? "neutral"}>{ENV_LABELS[server.environment] ?? server.environment}</Badge> : null}
          <div className="flex-1 min-w-64">
            <Field label="Caminho remoto">
              <Input
                value={pathDraft}
                onChange={(e) => setPathDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void refresh(pathDraft.trim() || "/");
                }}
                placeholder="/srv/app"
              />
            </Field>
          </div>
          <Button size="sm" onClick={goUp} disabled={path === "/" || loading}>
            <UploadIcon className="size-3.5 rotate-180" /> Subir
          </Button>
          <Button size="sm" onClick={() => void refresh()} disabled={loading}>
            <RefreshIcon className={`size-3.5 ${loading ? "animate-spin" : ""}`} /> Atualizar
          </Button>
          <Button size="sm" variant="primary" onClick={uploadFiles} disabled={serverId == null}>
            <UploadIcon className="size-3.5" /> Enviar arquivos
          </Button>
          <Button size="sm" onClick={() => setPending({ kind: "mkdir" })} disabled={serverId == null}>
            <PlusIcon className="size-3.5" /> Novo diretório
          </Button>
        </div>
        {server ? (
          <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
            Autenticação: {AUTH_LABELS[server.authMethod]} · verificação de host estrita (known_hosts)
            {server.authMethod === "password" ? " · a senha é pedida a cada operação" : ""}
          </p>
        ) : null}
      </Card>

      {error ? (
        <Card className="border-red-300/60 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-200">
          <div className="flex items-start gap-2">
            <AlertTriangleIcon className="mt-0.5 size-4 shrink-0" />
            <div className="flex-1">
              <p className="font-medium">Não foi possível listar o diretório</p>
              <p className="mt-1 text-xs opacity-90">{error}</p>
            </div>
            <button onClick={() => setError(null)} className="rounded p-1 hover:bg-black/5 dark:hover:bg-white/5" aria-label="Fechar">
              <XIcon className="size-4" />
            </button>
          </div>
        </Card>
      ) : null}

      <Card className="overflow-hidden">
        <div className="flex items-center justify-between border-b border-zinc-200 px-3 py-2 text-xs text-zinc-500 dark:border-zinc-800 dark:text-zinc-400">
          <span className="font-mono truncate px-1">{path}</span>
          <span>
            {entries.length} entrada(s) {selected ? `· selecionada: ${selected.name}` : ""}
          </span>
        </div>
        {entries.length === 0 && !loading ? (
          <EmptyState icon={<FolderIcon className="size-8" />} title="Diretório vazio" description="Envie arquivos com o botão acima ou crie um diretório." />
        ) : (
          <table className="w-full text-sm">
            <thead className="bg-zinc-50 text-[11px] uppercase tracking-wider text-zinc-500 dark:bg-zinc-900/60 dark:text-zinc-400">
              <tr>
                <th className="px-3 py-2 text-left font-semibold">Nome</th>
                <th className="px-3 py-2 text-right font-semibold">Tamanho</th>
                <th className="px-3 py-2 text-left font-semibold">Dono</th>
                <th className="px-3 py-2 text-left font-semibold">Permissões</th>
                <th className="px-3 py-2 text-left font-semibold">Modificado</th>
                <th className="px-3 py-2 text-right font-semibold">Ações</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => {
                const isSelected = selected?.path === e.path;
                return (
                  <tr
                    key={e.path}
                    onClick={() => setSelected(e)}
                    onDoubleClick={() => void onOpenFile(e)}
                    className={`cursor-pointer border-t border-zinc-100 dark:border-zinc-800 ${
                      isSelected ? "bg-teal-50/60 dark:bg-teal-950/30" : "hover:bg-zinc-50 dark:hover:bg-zinc-900/40"
                    }`}
                  >
                    <td className="px-3 py-2">
                      <div className="flex items-center gap-2">
                        <FolderIcon className={`size-4 shrink-0 ${e.isDir ? "text-amber-500" : e.isSymlink ? "text-sky-500" : "text-zinc-400"}`} />
                        <span className="truncate font-medium">{e.name}</span>
                        {e.isSymlink ? <Badge tone="info">link</Badge> : null}
                      </div>
                    </td>
                    <td className="px-3 py-2 text-right font-mono text-xs">{e.isDir ? "—" : formatBytes(e.size)}</td>
                    <td className="px-3 py-2 font-mono text-xs">{e.owner ?? "—"}</td>
                    <td className="px-3 py-2 font-mono text-xs">{e.perms ?? "—"}</td>
                    <td className="px-3 py-2 text-xs text-zinc-500 dark:text-zinc-400">{e.modified ? formatDateTime(e.modified) : "—"}</td>
                    <td className="px-3 py-2">
                      <div className="flex justify-end gap-1">
                        {!e.isDir && !e.isSymlink ? (
                          <Button size="sm" variant="ghost" onClick={(ev) => (ev.stopPropagation(), void onOpenFile(e))}>
                            <PencilIcon className="size-3.5" /> Editar
                          </Button>
                        ) : null}
                        {!e.isDir ? (
                          <Button size="sm" variant="ghost" onClick={(ev) => (ev.stopPropagation(), void downloadFile(e))}>
                            <UploadIcon className="size-3.5 rotate-180" /> Baixar
                          </Button>
                        ) : null}
                        <Button size="sm" variant="ghost" onClick={(ev) => (ev.stopPropagation(), setPending({ kind: "rename", entry: e }))}>
                          Renomear
                        </Button>
                        {!e.isDir ? (
                          <Button size="sm" variant="ghost" onClick={(ev) => (ev.stopPropagation(), setPending({ kind: "chmod", entry: e }))}>
                            chmod
                          </Button>
                        ) : null}
                        <Button size="sm" variant="danger" onClick={(ev) => (ev.stopPropagation(), requestDelete(e))}>
                          <TrashIcon className="size-3.5" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Card>

      <Card className="overflow-hidden">
        <div className="flex items-center justify-between border-b border-zinc-200 px-3 py-2 dark:border-zinc-800">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400">Transferências</h3>
          <span className="text-[11px] text-zinc-500">{transferList.length} ativa(s) / finalizadas nesta sessão</span>
        </div>
        {transferList.length === 0 ? (
          <p className="px-3 py-4 text-xs text-zinc-500 dark:text-zinc-400">Nenhuma transferência em andamento.</p>
        ) : (
          <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
            {transferList.map((t) => (
              <li key={t.id} className="px-3 py-2">
                <div className="flex items-center justify-between gap-3 text-xs">
                  <div className="min-w-0 flex-1">
                    <p className="truncate">
                      <Badge tone={t.kind === "upload" ? "accent" : "info"}>{t.kind === "upload" ? "↑ envio" : "↓ recebimento"}</Badge>{" "}
                      <span className="ml-1 font-medium">{t.fileName || remoteBasename(t.kind === "upload" ? t.remotePath : t.localPath)}</span>
                      {t.serverName ? <span className="ml-1 text-zinc-500">· {t.serverName}</span> : null}
                    </p>
                    <p className="mt-0.5 truncate font-mono text-[10px] text-zinc-500">
                      {t.kind === "upload" ? `${t.localPath} → ${t.remotePath}` : `${t.remotePath} → ${t.localPath}`}
                    </p>
                  </div>
                  <div className="shrink-0 text-right">
                    {t.done ? (
                      t.ok ? (
                        <Badge tone="success">concluída</Badge>
                      ) : (
                        <Badge tone="error">{t.error ?? "falhou"}</Badge>
                      )
                    ) : (
                      <>
                        <p className="font-mono">
                          {formatBytes(t.transferred)} / {formatBytes(t.total)} · {t.pct.toFixed(1)}%
                        </p>
                        <p className="text-[10px] text-zinc-500">
                          {formatRate(t.rate)} · resta {formatEta(t.eta)} · {t.startedAtLabel}
                        </p>
                      </>
                    )}
                  </div>
                  {!t.done ? (
                    <Button size="sm" variant="ghost" onClick={() => void api.transferCancel(t.id).catch((e) => pushToast("error", String(e)))}>
                      Cancelar
                    </Button>
                  ) : null}
                </div>
                {!t.done ? (
                  <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
                    <div
                      className="h-full rounded-full bg-teal-500 transition-[width] duration-300"
                      style={{ width: `${Math.max(0, Math.min(100, t.pct))}%` }}
                    />
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </Card>

      <PendingModal pending={pending} busy={busy} onClose={() => setPending(null)} onApply={applyPending} />

      <Modal open={editor != null} onClose={() => setEditor(null)} title={editor ? `Editar ${remoteBasename(editor.path)}` : ""} width="max-w-3xl">
        {editor ? (
          <div className="flex h-[70vh] flex-col gap-3">
            <Textarea
              value={editor.content}
              onChange={(e) => setEditor({ ...editor, content: e.target.value })}
              className="h-full font-mono text-xs"
              spellCheck={false}
            />
            <div className="flex justify-end gap-2">
              <Button onClick={() => setEditor(null)} disabled={editor.saving}>
                Cancelar
              </Button>
              <Button variant="primary" loading={editor.saving} onClick={() => void saveEditor()}>
                Salvar via SFTP
              </Button>
            </div>
          </div>
        ) : null}
      </Modal>

      <Confirm
        open={pending?.kind === "delete"}
        busy={busy}
        title={pending?.kind === "delete" ? (pending.entry.isDir ? "Remover diretório?" : "Remover arquivo?") : ""}
        message={
          pending?.kind === "delete"
            ? `Esta ação apaga "${pending.entry.name}" no servidor ${server?.name ?? ""}. A remoção é imediata e irreversível.`
            : ""
        }
        confirmLabel="Remover"
        onClose={() => setPending(null)}
        onConfirm={() => {
          if (pending?.kind === "delete") void onConfirmDelete(pending.entry);
          setPending(null);
        }}
      />
    </div>
  );
}

function PendingModal({
  pending,
  busy,
  onClose,
  onApply,
}: {
  pending: PendingOp;
  busy: boolean;
  onClose: () => void;
  onApply: (value: string) => Promise<void>;
}) {
  const [value, setValue] = useState("");
  useEffect(() => {
    if (!pending) return;
    if (pending.kind === "rename") setValue(pending.entry.name);
    else if (pending.kind === "chmod") setValue(pending.entry.perms ? permsToMode(pending.entry.perms) : "0644");
    else setValue("");
  }, [pending]);

  if (!pending || pending.kind === "delete") return null;

  const title = pending.kind === "mkdir" ? "Novo diretório" : pending.kind === "rename" ? "Renomear" : "Alterar permissão";
  const label = pending.kind === "mkdir" ? "Nome" : pending.kind === "rename" ? "Novo nome" : "Modo (4 dígitos, ex.: 0644)";
  const placeholder = pending.kind === "chmod" ? "0644" : pending.kind === "mkdir" ? "public" : "novo-nome";

  return (
    <Modal open onClose={busy ? () => {} : onClose} title={title} width="max-w-md">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void onApply(value);
        }}
        className="space-y-3"
      >
        {pending.kind !== "mkdir" ? (
          <p className="truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400">{pending.entry.path}</p>
        ) : null}
        <Field label={label}>
          <Input autoFocus value={value} onChange={(e) => setValue(e.target.value)} placeholder={placeholder} />
        </Field>
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={onClose} disabled={busy}>
            Cancelar
          </Button>
          <Button type="submit" variant="primary" loading={busy} disabled={!value.trim()}>
            Aplicar
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function permsToMode(perms: string): string {
  const body = perms.length === 11 ? perms.slice(1) : perms.slice(0, 9);
  const chunk = (s: string) => (s[0] === "r" ? 4 : 0) + (s[1] === "w" ? 2 : 0) + (s[2] === "x" || s[2] === "s" || s[2] === "t" ? 1 : 0);
  const rwx = body.slice(0, 9);
  return `0${chunk(rwx.slice(0, 3))}${chunk(rwx.slice(3, 6))}${chunk(rwx.slice(6, 9))}`;
}
