import { useMemo, useState } from "react";
import type { GitRepo } from "../../lib/useGitRepo";
import type { BranchInfo } from "../../types";
import { formatRelative } from "../../lib/format";
import { GitBranchIcon, PencilIcon, PlusIcon, TrashIcon } from "../Icons";
import { Badge, Button, Card, Checkbox, Confirm, EmptyState, Field, Input, Modal, Select } from "../ui";

function branchRowKey(b: BranchInfo) {
  return `${b.remote ?? "local"}:${b.name}`;
}

/** `origin/main` vira `main`; ramos locais com barra (feature/x) ficam intactos. */
function shortRemoteName(b: BranchInfo) {
  const prefix = `${b.remote}/`;
  return b.remote !== null && b.name.startsWith(prefix) ? b.name.slice(prefix.length) : b.name;
}

export function BranchesPanel({ git, branches }: { git: GitRepo; branches: BranchInfo[] }) {
  const busy = git.busy !== null;
  const locals = useMemo(() => branches.filter((b) => b.remote === null), [branches]);
  const remotes = useMemo(() => branches.filter((b) => b.remote !== null), [branches]);
  const current = locals.find((b) => b.isHead)?.name ?? null;

  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [startPoint, setStartPoint] = useState("");
  const [renaming, setRenaming] = useState<BranchInfo | null>(null);
  const [renameTo, setRenameTo] = useState("");
  const [deleting, setDeleting] = useState<BranchInfo | null>(null);
  const [forceDelete, setForceDelete] = useState(false);
  const [merging, setMerging] = useState<BranchInfo | null>(null);
  const [rebasing, setRebasing] = useState<BranchInfo | null>(null);

  const create = async () => {
    const name = newName.trim();
    if (name === "") return;
    const next = await git.checkout(name, true, startPoint.trim() === "" ? null : startPoint.trim());
    if (next) {
      setCreating(false);
      setNewName("");
      setStartPoint("");
    }
  };

  const rename = async () => {
    if (!renaming || renameTo.trim() === "") return;
    const list = await git.renameBranch(renaming.name, renameTo.trim());
    if (list) setRenaming(null);
  };

  const remove = async () => {
    if (!deleting) return;
    const list = await git.deleteBranch(deleting.name, forceDelete);
    if (list) {
      setDeleting(null);
      setForceDelete(false);
    }
  };

  const runMerge = async () => {
    if (!merging) return;
    const outcome = await git.merge(merging.name);
    if (outcome) setMerging(null);
  };

  const runRebase = async () => {
    if (!rebasing) return;
    const outcome = await git.rebase(rebasing.name);
    if (outcome) setRebasing(null);
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" size="sm" disabled={busy} onClick={() => setCreating(true)}>
          <PlusIcon className="size-3.5" /> Novo ramo
        </Button>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          Criar e trocar de ramo acontecem no repositório local. Nada é enviado ao remoto sem você usar “Enviar”.
        </p>
      </div>

      <Card className="p-2">
        <p className="px-2 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          Ramos locais · {locals.length}
        </p>
        {locals.length === 0 ? (
          <EmptyState
            icon={<GitBranchIcon className="size-7" />}
            title="Nenhum ramo local"
            description="Crie um ramo, ou faça checkout de um ramo remoto para acompanhá-lo localmente."
          />
        ) : null}
        <ul className="space-y-0.5">
          {locals.map((b) => (
            <li
              key={branchRowKey(b)}
              className={`flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg px-2 py-1.5 ${
                b.isHead ? "bg-teal-500/10" : "hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50"
              }`}
            >
              <button
                className="min-w-0 flex-1 text-left"
                disabled={b.isHead || busy}
                onClick={() => void git.checkout(b.name, false)}
                title={b.isHead ? "Ramo atual" : "Trocar para este ramo"}
              >
                <span className="flex items-center gap-1.5">
                  <span className="truncate text-sm font-medium">{b.name}</span>
                  {b.isHead ? <Badge tone="success">atual</Badge> : null}
                  {b.ahead > 0 ? <Badge tone="accent">{b.ahead} à frente</Badge> : null}
                  {b.behind > 0 ? <Badge tone="warning">{b.behind} atrás</Badge> : null}
                </span>
                <span className="mt-0.5 block truncate text-[11px] text-zinc-500 dark:text-zinc-400">
                  {b.lastSubject} · {formatRelative(b.lastUnixDate)}
                </span>
              </button>
              <span className="flex shrink-0 items-center gap-1">
                <Button size="sm" variant="ghost" disabled={busy || current === null} onClick={() => setMerging(b)} title="Mesclar este ramo no ramo atual">
                  Mesclar
                </Button>
                <Button size="sm" variant="ghost" disabled={busy || current === null || b.isHead} onClick={() => setRebasing(b)} title="Rebasear o ramo atual sobre este">
                  Rebasear
                </Button>
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => (setRenaming(b), setRenameTo(b.name))} title="Renomear">
                  <PencilIcon className="size-3.5" />
                </Button>
                <Button size="sm" variant="ghost" disabled={busy || b.isHead} onClick={() => setDeleting(b)} title="Excluir ramo">
                  <TrashIcon className="size-3.5" />
                </Button>
              </span>
            </li>
          ))}
        </ul>
      </Card>

      <Card className="p-2">
        <p className="px-2 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          Ramos remotos · {remotes.length}
        </p>
        {remotes.length === 0 ? (
          <p className="px-2 py-3 text-xs text-zinc-500 dark:text-zinc-400">
            Nenhum ramo remoto conhecido. Use “Buscar” na barra superior para trazer os refs do remoto, ou cadastre um
            remoto na aba Remotos.
          </p>
        ) : null}
        <ul className="space-y-0.5">
          {remotes.map((b) => (
            <li key={branchRowKey(b)} className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg px-2 py-1.5 hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50">
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5">
                  <Badge tone="info">{b.remote}</Badge>
                  <span className="truncate font-mono text-xs">{shortRemoteName(b)}</span>
                </span>
                <span className="mt-0.5 block truncate text-[11px] text-zinc-500 dark:text-zinc-400">
                  {b.lastSubject} · {formatRelative(b.lastUnixDate)}
                </span>
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                title="Criar um ramo local acompanhando este ramo remoto"
                onClick={() => void git.checkout(shortRemoteName(b), false)}
              >
                Acompanhar
              </Button>
            </li>
          ))}
        </ul>
      </Card>

      <Modal open={creating} onClose={() => setCreating(false)} title="Novo ramo">
        <div className="grid gap-3">
          <Field label="Nome do ramo">
            <Input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="feature/cadastro-de-usuarios" />
          </Field>
          <Field label="A partir de (opcional)" hint="Nome de um ramo, tag ou hash. Vazio usa o commit atual.">
            <Select value={startPoint} onChange={(e) => setStartPoint(e.target.value)}>
              <option value="">HEAD ({current ?? "commit atual"})</option>
              {branches
                .filter((b) => b.remote !== null || !b.isHead)
                .map((b) => (
                  <option key={branchRowKey(b)} value={b.name}>
                    {b.name}
                  </option>
                ))}
            </Select>
          </Field>
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setCreating(false)} disabled={busy}>
            Cancelar
          </Button>
          <Button variant="primary" disabled={busy || newName.trim() === ""} loading={busy && git.busy?.startsWith("Criando") === true} onClick={() => void create()}>
            Criar e trocar
          </Button>
        </div>
      </Modal>

      <Modal open={renaming !== null} onClose={() => setRenaming(null)} title={`Renomear ${renaming?.name ?? ""}`}>
        <Field label="Novo nome">
          <Input value={renameTo} onChange={(e) => setRenameTo(e.target.value)} />
        </Field>
        <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
          A renomeação é local. O ramo remoto mantém o nome antigo até você enviar o novo e ajustar o remoto
          manualmente.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={() => setRenaming(null)} disabled={busy}>
            Cancelar
          </Button>
          <Button variant="primary" disabled={busy || renameTo.trim() === "" || renameTo.trim() === renaming?.name} onClick={() => void rename()}>
            Renomear
          </Button>
        </div>
      </Modal>

      <Confirm
        open={deleting !== null}
        title={`Excluir o ramo ${deleting?.name ?? ""}`}
        message={
          forceDelete
            ? "A exclusão forçada descarta commits que não estão em outro ramo. Essa ref local some do histórico; recupera-se apenas pelo reflog."
            : "Excluir o ramo local? Os commits continuam acessíveis pelo histórico enquanto estiverem em outros ramos."
        }
        confirmLabel="Excluir"
        busy={busy}
        onConfirm={() => void remove()}
        onClose={() => {
          setDeleting(null);
          setForceDelete(false);
        }}
      >
        <Checkbox checked={forceDelete} onChange={setForceDelete} label="Forçar (-D), mesmo com commits não mesclados" />
      </Confirm>

      <Confirm
        open={merging !== null}
        title={`Mesclar ${merging?.name ?? ""} em ${current ?? ""}`}
        message="O merge cria um commit de mesclagem no ramo atual. Se houver conflito, a aplicação para e mostra cada arquivo para você decidir o lado."
        confirmLabel="Mesclar"
        busy={busy}
        onConfirm={() => void runMerge()}
        onClose={() => setMerging(null)}
      />

      <Confirm
        open={rebasing !== null}
        title={`Rebasear ${current ?? ""} sobre ${rebasing?.name ?? ""}`}
        message="O rebase reescreve os commits do ramo atual, trocando seus hashes. Se esse ramo já foi enviado a um remoto, o push seguinte será recusado — a aplicação não força histórico por conta própria."
        confirmLabel="Rebasear"
        busy={busy}
        onConfirm={() => void runRebase()}
        onClose={() => setRebasing(null)}
      />
    </div>
  );
}
