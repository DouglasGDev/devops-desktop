import { useMemo, useState } from "react";
import type { GitRepo } from "../../lib/useGitRepo";
import type { BranchInfo, TagInfo } from "../../types";
import { PlusIcon, TagIcon, TrashIcon } from "../Icons";
import { Badge, Button, Card, Confirm, EmptyState, Field, Input, Modal, Select, Textarea } from "../ui";

export function TagsPanel({
  git,
  tags,
  branches,
}: {
  git: GitRepo;
  tags: TagInfo[];
  branches: BranchInfo[];
}) {
  const busy = git.busy !== null;
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [target, setTarget] = useState("");
  const [message, setMessage] = useState("");
  const [deleting, setDeleting] = useState<TagInfo | null>(null);

  const heads = useMemo(() => branches.filter((b) => b.isHead), [branches]);

  const create = async () => {
    const trimmed = name.trim();
    if (trimmed === "") return;
    const list = await git.createTag(trimmed, target.trim() === "" ? null : target.trim(), message.trim() === "" ? null : message.trim());
    if (list) {
      setCreating(false);
      setName("");
      setMessage("");
    }
  };

  const remove = async () => {
    if (!deleting) return;
    const list = await git.deleteTag(deleting.name);
    if (list) setDeleting(null);
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" disabled={busy} onClick={() => setCreating(true)}>
          <PlusIcon className="size-3.5" /> Nova tag
        </Button>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          Com mensagem a tag é anotada (vira um objeto com autor e data). Tags não são enviadas ao remoto até você usar
          “Enviar”.
        </p>
      </div>

      <Card className="p-2">
        <p className="px-2 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          Tags · {tags.length}
        </p>
        {tags.length === 0 ? (
          <EmptyState
            icon={<TagIcon className="size-7" />}
            title="Nenhuma tag"
            description="Marque um release ou um ponto de deploy para poder voltar a ele depois."
          >
            <Button size="sm" variant="primary" onClick={() => setCreating(true)}>
              <PlusIcon className="size-3.5" /> Criar a primeira tag
            </Button>
          </EmptyState>
        ) : null}
        <ul className="space-y-0.5">
          {tags.map((t) => (
            <li key={t.name} className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg px-2 py-1.5 hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50">
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <Badge tone="violet">{t.name}</Badge>
                  <span className="font-mono text-[11px] text-zinc-400 dark:text-zinc-500">{t.target}</span>
                  {t.message ? <Badge tone="accent">anotada</Badge> : null}
                </span>
                {t.message ? (
                  <span className="mt-0.5 block truncate text-[11px] text-zinc-500 dark:text-zinc-400">{t.message}</span>
                ) : null}
              </span>
              <Button size="sm" variant="ghost" disabled={busy} title="Excluir a tag local" onClick={() => setDeleting(t)}>
                <TrashIcon className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      </Card>

      <Modal open={creating} onClose={() => setCreating(false)} title="Nova tag">
        <div className="grid gap-3">
          <Field label="Nome" hint="Ex.: v1.4.0 ou release-2026-10.">
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="v1.0.0" />
          </Field>
          <Field label="Apontar para" hint="Vazio usa o commit atual do ramo.">
            <Select value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="">HEAD{heads.length > 0 ? ` (${heads[0].name})` : ""}</option>
              {branches.map((b) => (
                <option key={`${b.remote ?? "l"}:${b.name}`} value={b.name}>
                  {b.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Mensagem (opcional)" hint="Sem mensagem a tag apenas aponta para o commit.">
            <Textarea rows={2} value={message} onChange={(e) => setMessage(e.target.value)} />
          </Field>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setCreating(false)} disabled={busy}>
            Cancelar
          </Button>
          <Button variant="primary" disabled={busy || name.trim() === ""} onClick={() => void create()}>
            Criar tag
          </Button>
        </div>
      </Modal>

      <Confirm
        open={deleting !== null}
        title={`Excluir a tag ${deleting?.name ?? ""}`}
        message="A exclusão remove apenas a referência local. Se a tag já foi enviada a um remoto, ela continua lá até você excluí-la no remoto."
        confirmLabel="Excluir"
        busy={busy}
        onConfirm={() => void remove()}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}
