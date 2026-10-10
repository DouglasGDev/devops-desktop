import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { formatDateTime } from "../lib/format";
import type { Project } from "../types";
import { useApp } from "../store/AppContext";
import { FolderIcon, GitBranchIcon, PencilIcon, ChevronDownIcon, ChevronRightIcon, PlusIcon, RefreshIcon, TrashIcon } from "../components/Icons";
import { ProjectProfiles } from "../components/deploy/ProjectProfiles";
import { Badge, Button, Card, Confirm, EmptyState, Field, Input, Modal, Select } from "../components/ui";

export function ProjectsView() {
  const { projects, servers, reloadProjects, pushToast } = useApp();
  const [adding, setAdding] = useState(false);
  const [addBusy, setAddBusy] = useState(false);
  const [editing, setEditing] = useState<Project | null>(null);
  const [editName, setEditName] = useState("");
  const [editServer, setEditServer] = useState<string>("");
  const [deleting, setDeleting] = useState<Project | null>(null);
  const [refreshingId, setRefreshingId] = useState<number | null>(null);
  const [profilesFor, setProfilesFor] = useState<number | null>(null);

  const serverNameById = useMemo(() => {
    const map = new Map<number, string>();
    for (const s of servers) map.set(s.id, s.name);
    return map;
  }, [servers]);

  const pickFolder = async () => {
    setAddBusy(true);
    try {
      const dir = await open({ directory: true, multiple: false, title: "Selecionar pasta do projeto" });
      if (typeof dir !== "string" || dir.trim() === "") return;
      const project = await api.addProject(dir, null);
      pushToast("success", `Projeto “${project.name}” adicionado${project.isGitRepo ? ` (branch ${project.gitBranch ?? "?"})` : " (sem Git detectado)"}.`);
      await reloadProjects();
      setAdding(false);
    } catch (e) {
      pushToast("error", `Falha ao adicionar projeto: ${String(e)}`);
    } finally {
      setAddBusy(false);
    }
  };

  const openEdit = (p: Project) => {
    setEditing(p);
    setEditName(p.name);
    setEditServer(p.defaultServerId === null ? "" : String(p.defaultServerId));
  };

  const saveEdit = async () => {
    if (!editing) return;
    if (!editName.trim()) {
      pushToast("error", "Informe um nome para o projeto.");
      return;
    }
    try {
      await api.updateProject(editing.id, editName.trim(), editServer === "" ? null : Number(editServer));
      pushToast("success", "Projeto atualizado.");
      setEditing(null);
      await reloadProjects();
    } catch (e) {
      pushToast("error", `Falha ao salvar: ${String(e)}`);
    }
  };

  const remove = async () => {
    if (!deleting) return;
    try {
      const modelos = await api.deleteProject(deleting.id);
      pushToast(
        "info",
        modelos > 0
          ? `Projeto “${deleting.name}” removido do cadastro, junto com ${modelos} modelo(s) de deploy. Nenhum arquivo local foi apagado.`
          : `Projeto “${deleting.name}” removido do cadastro. Nenhum arquivo local foi apagado.`,
      );
      setDeleting(null);
      await reloadProjects();
    } catch (e) {
      pushToast("error", `Falha ao remover: ${String(e)}`);
    }
  };

  const refresh = async (p: Project) => {
    setRefreshingId(p.id);
    try {
      const updated = await api.refreshProject(p.id);
      pushToast(
        updated.isGitRepo ? "success" : "info",
        updated.isGitRepo
          ? `“${updated.name}”: branch ${updated.gitBranch ?? "?"}${updated.gitRemote ? ` · ${updated.gitRemote}` : ""}.`
          : `“${updated.name}” continua sem repositório Git.`,
      );
      await reloadProjects();
    } catch (e) {
      pushToast("error", `Falha ao atualizar: ${String(e)}`);
    } finally {
      setRefreshingId(null);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Button variant="primary" onClick={() => (setAdding(true), void pickFolder())}>
          <PlusIcon /> Adicionar projeto
        </Button>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          A detecção de Git usa o repositório local existente — nada é clonado ou alterado nesta etapa.
        </p>
      </div>

      {projects.length === 0 ? (
        <Card>
          <EmptyState
            icon={<FolderIcon className="size-8" />}
            title="Nenhum projeto cadastrado"
            description="Adicione uma pasta local do seu computador. A aplicação detecta automaticamente se ela contém um repositório Git, a branch atual e o remote."
          >
            <Button variant="primary" onClick={() => (setAdding(true), void pickFolder())}>
              <PlusIcon /> Selecionar pasta
            </Button>
          </EmptyState>
        </Card>
      ) : (
        <Card className="p-4">
          <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
            {projects.map((p) => (
              <li key={p.id} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2.5">
                <div className="min-w-0 flex-1">
                  <p className="flex items-center gap-2 text-sm font-medium">
                    {p.name}
                    {p.isGitRepo ? (
                      <Badge tone="accent">
                        <GitBranchIcon className="size-3" /> {p.gitBranch ?? "sem branch"}
                      </Badge>
                    ) : (
                      <Badge>Sem Git</Badge>
                    )}
                  </p>
                  <p className="truncate font-mono text-xs text-zinc-500 dark:text-zinc-400" title={p.localPath}>
                    {p.localPath}
                  </p>
                  {p.gitRemote ? (
                    <p className="truncate font-mono text-[11px] text-zinc-400 dark:text-zinc-500">remote: {p.gitRemote}</p>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {p.defaultServerId !== null ? (
                    <Badge tone="info">Deploy: {serverNameById.get(p.defaultServerId) ?? `#${p.defaultServerId}`}</Badge>
                  ) : null}
                  <span className="hidden text-[11px] text-zinc-400 lg:inline">{formatDateTime(p.updatedAt)}</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    title="Modelos de deploy deste projeto"
                    onClick={() => setProfilesFor(profilesFor === p.id ? null : p.id)}
                  >
                    {profilesFor === p.id ? (
                      <ChevronDownIcon className="size-3.5" />
                    ) : (
                      <ChevronRightIcon className="size-3.5" />
                    )}
                    Modelos de deploy
                  </Button>
                  <Button size="sm" loading={refreshingId === p.id} onClick={() => void refresh(p)} title="Atualizar detecção de Git">
                    <RefreshIcon className="size-3.5" />
                  </Button>
                  <Button size="sm" variant="ghost" title="Editar" onClick={() => openEdit(p)}>
                    <PencilIcon className="size-3.5" />
                  </Button>
                  <Button size="sm" variant="ghost" title="Remover do cadastro" onClick={() => setDeleting(p)}>
                    <TrashIcon className="size-3.5" />
                  </Button>
                </div>
                {profilesFor === p.id ? (
                  <div className="w-full border-t border-zinc-100 pt-3 dark:border-zinc-800">
                    <ProjectProfiles project={p} />
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Modal open={adding} onClose={() => setAdding(false)} title="Adicionar projeto">
        <p className="text-sm text-zinc-600 dark:text-zinc-300">Selecione a pasta do projeto no seu computador.</p>
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          A pasta deve existir e ser legível. A aplicação registra apenas o caminho no banco local e lê os metadados do Git
          quando houver.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setAdding(false)} disabled={addBusy}>
            Cancelar
          </Button>
          <Button variant="primary" loading={addBusy} onClick={() => void pickFolder()}>
            <FolderIcon /> Escolher pasta…
          </Button>
        </div>
      </Modal>

      <Modal open={editing !== null} onClose={() => setEditing(null)} title={`Editar “${editing?.name ?? ""}”`}>
        {editing ? (
          <>
            <div className="grid gap-3">
              <Field label="Nome">
                <Input value={editName} onChange={(e) => setEditName(e.target.value)} />
              </Field>
              <Field label="Servidor padrão de deploy (opcional)">
                <Select value={editServer} onChange={(e) => setEditServer(e.target.value)}>
                  <option value="">Nenhum</option>
                  {servers.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name} ({s.host})
                    </option>
                  ))}
                </Select>
              </Field>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <Button onClick={() => setEditing(null)}>Cancelar</Button>
              <Button variant="primary" onClick={() => void saveEdit()}>
                Salvar
              </Button>
            </div>
          </>
        ) : null}
      </Modal>

      <Confirm
        open={deleting !== null}
        title="Remover projeto"
        message={`Remover “${deleting?.name}” do cadastro? Os modelos de deploy dele e o histórico dessas publicações saem junto. Os arquivos em ${deleting?.localPath} não são afetados.`}
        confirmLabel="Remover"
        onConfirm={() => void remove()}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}
