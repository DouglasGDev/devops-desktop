import { useState } from "react";
import type { GitRepo } from "../../lib/useGitRepo";
import type { RemoteInfo } from "../../types";
import { CloudIcon, ExternalLinkIcon, PencilIcon, PlusIcon, TrashIcon } from "../Icons";
import { Badge, Button, Card, Confirm, EmptyState, Field, Input, Modal } from "../ui";

export function RemotesPanel({ git, remotes }: { git: GitRepo; remotes: RemoteInfo[] }) {
  const busy = git.busy !== null;
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [editing, setEditing] = useState<RemoteInfo | null>(null);
  const [editUrl, setEditUrl] = useState("");
  const [removing, setRemoving] = useState<RemoteInfo | null>(null);

  const add = async () => {
    const list = await git.addRemote(name.trim(), url.trim());
    if (list) {
      setAdding(false);
      setName("");
      setUrl("");
    }
  };

  const saveUrl = async () => {
    if (!editing) return;
    const list = await git.setRemoteUrl(editing.name, editUrl.trim());
    if (list) setEditing(null);
  };

  const remove = async () => {
    if (!removing) return;
    const list = await git.removeRemote(removing.name);
    if (list) setRemoving(null);
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" disabled={busy} onClick={() => setAdding(true)}>
          <PlusIcon className="size-3.5" /> Adicionar remoto
        </Button>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          URLs com usuário e senha/token embutidos são redigidas na interface e no histórico — a credencial continua
          sendo tratada pelo seu gerenciador de credenciais do git.
        </p>
      </div>

      <Card className="p-2">
        <p className="px-2 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          Remotos · {remotes.length}
        </p>
        {remotes.length === 0 ? (
          <EmptyState
            icon={<ExternalLinkIcon className="size-7" />}
            title="Nenhum remoto configurado"
            description="Sem remoto a aplicação só trabalha localmente: nada pode ser buscado ou enviado."
          >
            <Button size="sm" variant="primary" onClick={() => setAdding(true)}>
              <PlusIcon className="size-3.5" /> Adicionar origin
            </Button>
          </EmptyState>
        ) : null}
        <ul className="space-y-1">
          {remotes.map((r) => (
            <li key={r.name} className="rounded-lg px-2 py-1.5 hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <Badge tone="info">{r.name}</Badge>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400" title={r.fetchUrl ?? ""}>
                  {r.fetchUrl ?? "sem URL de busca"}
                </span>
                <span className="flex shrink-0 items-center gap-1">
                  <Button size="sm" variant="ghost" disabled={busy} title="Buscar refs deste remoto" onClick={() => void git.fetch(r.name)}>
                    <CloudIcon className="size-3.5" /> Buscar
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    title="Alterar a URL"
                    onClick={() => (setEditing(r), setEditUrl(r.fetchUrl ?? ""))}
                  >
                    <PencilIcon className="size-3.5" />
                  </Button>
                  <Button size="sm" variant="ghost" disabled={busy} title="Remover o remoto" onClick={() => setRemoving(r)}>
                    <TrashIcon className="size-3.5" />
                  </Button>
                </span>
              </div>
              {r.pushUrl !== null && r.pushUrl !== r.fetchUrl ? (
                <p className="mt-0.5 truncate pl-1 font-mono text-[11px] text-zinc-400 dark:text-zinc-500">
                  push: {r.pushUrl}
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      </Card>

      <Modal open={adding} onClose={() => setAdding(false)} title="Adicionar remoto">
        <div className="grid gap-3">
          <Field label="Nome" hint="O nome convencional é origin.">
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="origin" />
          </Field>
          <Field label="URL" hint="https://…, ssh://… ou usuário@host:caminho.">
            <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://github.com/usuario/repositorio.git" />
          </Field>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setAdding(false)} disabled={busy}>
            Cancelar
          </Button>
          <Button variant="primary" disabled={busy || name.trim() === "" || url.trim() === ""} onClick={() => void add()}>
            Adicionar
          </Button>
        </div>
      </Modal>

      <Modal open={editing !== null} onClose={() => setEditing(null)} title={`URL do remoto ${editing?.name ?? ""}`}>
        <Field label="Nova URL">
          <Input value={editUrl} onChange={(e) => setEditUrl(e.target.value)} />
        </Field>
        <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
          A URL atual é lida de <span className="font-mono">git remote get-url</span>; a alteração vale para busca e
          envio.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setEditing(null)} disabled={busy}>
            Cancelar
          </Button>
          <Button variant="primary" disabled={busy || editUrl.trim() === ""} onClick={() => void saveUrl()}>
            Salvar
          </Button>
        </div>
      </Modal>

      <Confirm
        open={removing !== null}
        title={`Remover o remoto ${removing?.name ?? ""}`}
        message="Isso apaga só a referência local ao remoto; os commits que já foram enviados continuam lá. Para reenviá-los será preciso cadastrar a URL de novo."
        confirmLabel="Remover"
        busy={busy}
        onConfirm={() => void remove()}
        onClose={() => setRemoving(null)}
      />
    </div>
  );
}
