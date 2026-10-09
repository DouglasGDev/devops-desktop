import { useMemo, useState } from "react";
import type { GitRepo } from "../../lib/useGitRepo";
import type { RepoStatus, SecretFinding, StatusFile } from "../../types";
import { AlertTriangleIcon, CheckIcon, PlusIcon, RefreshIcon, TrashIcon, XIcon } from "../Icons";
import { Badge, Button, Card, Checkbox, Confirm, EmptyState, Field, Textarea } from "../ui";

const KIND_TONES: Record<string, "success" | "error" | "warning" | "accent" | "neutral"> = {
  adicionado: "success",
  modificado: "accent",
  renomeado: "warning",
  copiado: "warning",
  removido: "error",
  "tipo alterado": "warning",
  "não rastreado": "neutral",
  conflito: "error",
};

function FileRow({
  file,
  group,
  selected,
  busy,
  onSelect,
  onStage,
  onUnstage,
  onDiscard,
  onResolve,
}: {
  file: StatusFile;
  /** de onde a linha veio: índice, worktree ou conflito */
  group: "index" | "worktree" | "conflict";
  selected: boolean;
  busy: boolean;
  onSelect: () => void;
  onStage: () => void;
  onUnstage: () => void;
  onDiscard: () => void;
  onResolve: (side: "ours" | "theirs") => void;
}) {
  const tone = KIND_TONES[file.kind] ?? "neutral";
  return (
    <li
      className={`flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg px-2 py-1.5 ${
        selected ? "bg-teal-500/10" : "hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50"
      }`}
    >
      <button onClick={onSelect} className="min-w-0 flex-1 text-left" title="Ver o diff linha a linha">
        <span className="block truncate font-mono text-xs" title={file.path}>
          {file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
        </span>
        <span className="flex items-center gap-1.5 pt-0.5">
          <Badge tone={tone}>{file.kind}</Badge>
          <span className="font-mono text-[10px] text-zinc-400 dark:text-zinc-500">
            {file.indexCode}
            {file.worktreeCode}
          </span>
        </span>
      </button>
      <span className="flex shrink-0 items-center gap-1">
        {group === "conflict" ? (
          <>
            <Button size="sm" disabled={busy} onClick={() => onResolve("ours")} title="Ficar com a versão do seu ramo">
              Meu lado
            </Button>
            <Button
              size="sm"
              disabled={busy}
              onClick={() => onResolve("theirs")}
              title="Ficar com a versão que está entrando"
            >
              Lado deles
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onStage} title="Marcar como resolvido (git add)">
              <CheckIcon className="size-3.5" />
            </Button>
          </>
        ) : group === "index" ? (
          <Button size="sm" variant="ghost" disabled={busy} onClick={onUnstage} title="Tirar do staging">
            <XIcon className="size-3.5" />
          </Button>
        ) : (
          <>
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={onDiscard}
              title="Descartar as alterações do arquivo"
            >
              <TrashIcon className="size-3.5" />
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onStage} title="Preparar para o commit">
              <PlusIcon className="size-3.5" />
            </Button>
          </>
        )}
      </span>
    </li>
  );
}

function Section({
  title,
  hint,
  files,
  group,
  selectedPath,
  busy,
  onSelect,
  onStage,
  onUnstage,
  onDiscard,
  onResolve,
  bulk,
}: {
  title: string;
  hint: string;
  files: StatusFile[];
  group: "index" | "worktree" | "conflict";
  selectedPath: string | null;
  busy: boolean;
  onSelect: (file: StatusFile) => void;
  onStage: (paths: string[]) => void;
  onUnstage: (paths: string[]) => void;
  onDiscard: (file: StatusFile) => void;
  onResolve: (file: StatusFile, side: "ours" | "theirs") => void;
  bulk: { label: string; title: string; onClick: () => void };
}) {
  return (
    <div>
      <div className="flex items-center gap-2 px-2 pb-1 pt-2">
        <p className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
          {title} · {files.length}
        </p>
        <Button size="sm" variant="ghost" className="ml-auto" disabled={busy} onClick={bulk.onClick} title={bulk.title}>
          {bulk.label}
        </Button>
      </div>
      <p className="px-2 pb-1.5 text-[11px] text-zinc-400 dark:text-zinc-500">{hint}</p>
      <ul className="space-y-0.5">
        {files.map((file) => (
          <FileRow
            key={`${group}:${file.path}`}
            file={file}
            group={group}
            selected={group !== "conflict" && selectedPath === file.path}
            busy={busy}
            onSelect={() => onSelect(file)}
            onStage={() => onStage([file.path])}
            onUnstage={() => onUnstage([file.path])}
            onDiscard={() => onDiscard(file)}
            onResolve={(side) => onResolve(file, side)}
          />
        ))}
      </ul>
    </div>
  );
}

export function ChangesPanel({
  git,
  status,
  selected,
  onSelectFile,
}: {
  git: GitRepo;
  status: RepoStatus;
  selected: { path: string; staged: boolean } | null;
  onSelectFile: (path: string, staged: boolean) => void;
}) {
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  const [confirmAmend, setConfirmAmend] = useState(false);
  const [discardTarget, setDiscardTarget] = useState<StatusFile | null>(null);
  const [committing, setCommitting] = useState(false);
  const [secretBlock, setSecretBlock] = useState<SecretFinding[] | null>(null);
  const busy = git.busy !== null || committing;

  const stagedFiles = useMemo(() => status.files.filter((f) => f.staged && !f.conflict), [status.files]);
  const worktreeFiles = useMemo(
    () => status.files.filter((f) => !f.staged && !f.conflict && (f.unstaged || f.untracked)),
    [status.files],
  );
  const conflictFiles = useMemo(() => status.files.filter((f) => f.conflict), [status.files]);
  const inRebase = status.branch === null && status.detachedAt !== null;

  const runCommit = async (allowSecrets = false) => {
    setCommitting(true);
    try {
      if (!allowSecrets) {
        const found = await git.secretCheck("index");
        if (found === null) return;
        const blocks = found.filter((f) => f.severity === "block");
        if (blocks.length > 0) {
          setSecretBlock(blocks);
          return;
        }
      }
      const hash = await git.commit(message, amend, allowSecrets);
      setSecretBlock(null);
      if (hash !== null) {
        setMessage("");
        setAmend(false);
        setConfirmAmend(false);
      }
    } finally {
      setCommitting(false);
    }
  };

  // Depois de tirar um arquivo do índice a lista é lida de novo: o que o Git
  // mostra é o índice atual, não o que vimos antes do clique.
  const untrackBlocked = async (path: string) => {
    const next = await git.untrackFile(path);
    if (next === null) return;
    const found = await git.secretCheck("index");
    setSecretBlock(found === null ? null : found.filter((f) => f.severity === "block"));
  };

  return (
    <div className="space-y-3">
      {conflictFiles.length > 0 ? (
        <Card className="border-amber-300/60 bg-amber-50/60 p-3 dark:border-amber-900/60 dark:bg-amber-950/20">
          <p className="flex items-center gap-2 text-xs font-medium text-amber-800 dark:text-amber-300">
            <AlertTriangleIcon className="size-4" />
            {conflictFiles.length} arquivo(s) em conflito — {inRebase ? "rebase" : "merge"} em andamento.
          </p>
          <p className="mt-1 text-[11px] text-amber-800/80 dark:text-amber-300/80">
            Escolha o lado de cada arquivo, marque como resolvido e faça o commit. Para desistir sem deixar nada
            aplicado, cancele a operação — nada é enviado a um remoto por aqui.
          </p>
          <div className="mt-2 flex gap-2">
            <Button
              size="sm"
              variant="danger"
              disabled={busy}
              onClick={() => void git.abort(inRebase ? "rebase" : "merge")}
            >
              Cancelar {inRebase ? "o rebase" : "o merge"}
            </Button>
            {inRebase ? (
              <Button size="sm" disabled={busy} onClick={() => void git.continueRebase()}>
                Continuar o rebase
              </Button>
            ) : null}
          </div>
        </Card>
      ) : null}

      {status.files.length === 0 ? (
        <Card>
          <EmptyState
            icon={<CheckIcon className="size-8" />}
            title="Árvore de trabalho limpa"
            description="Nenhuma alteração pendente neste repositório. Edite arquivos e volte aqui, ou busque mudanças do remoto."
          >
            <Button size="sm" disabled={busy} onClick={() => void git.reload()}>
              <RefreshIcon className="size-3.5" /> Verificar de novo
            </Button>
          </EmptyState>
        </Card>
      ) : (
        <Card className="p-2">
          {stagedFiles.length > 0 ? (
            <Section
              title="Para commit (índice)"
              hint="O que entra no próximo commit. Clique no arquivo para revisar o diff preparado, hunk por hunk."
              files={stagedFiles}
              group="index"
              selectedPath={selected?.staged ? selected.path : null}
              busy={busy}
              onSelect={(f) => onSelectFile(f.path, true)}
              onStage={(paths) => void git.stage(paths)}
              onUnstage={(paths) => void git.unstage(paths)}
              onDiscard={(f) => setDiscardTarget(f)}
              onResolve={(f, side) => void git.resolveConflict(f.path, side)}
              bulk={{
                label: "Tirar tudo",
                title: "Remover todos os arquivos do staging",
                onClick: () => void git.unstage(stagedFiles.map((f) => f.path)),
              }}
            />
          ) : null}
          {worktreeFiles.length > 0 ? (
            <Section
              title="Alterações não preparadas"
              hint="Mudanças no disco fora do índice. O “+” prepara o arquivo inteiro; para escolher trechos individuais abra o diff e use “Preparar trecho”."
              files={worktreeFiles}
              group="worktree"
              selectedPath={selected && !selected.staged ? selected.path : null}
              busy={busy}
              onSelect={(f) => onSelectFile(f.path, false)}
              onStage={(paths) => void git.stage(paths)}
              onUnstage={(paths) => void git.unstage(paths)}
              onDiscard={(f) => setDiscardTarget(f)}
              onResolve={(f, side) => void git.resolveConflict(f.path, side)}
              bulk={{
                label: "Preparar tudo",
                title: "Adicionar todos os arquivos ao staging",
                onClick: () => void git.stage(worktreeFiles.map((f) => f.path)),
              }}
            />
          ) : null}
          {conflictFiles.length > 0 ? (
            <Section
              title="Conflitos"
              hint="Cada arquivo precisa de uma decisão antes do commit."
              files={conflictFiles}
              group="conflict"
              selectedPath={null}
              busy={busy}
              onSelect={(f) => onSelectFile(f.path, true)}
              onStage={(paths) => void git.stage(paths)}
              onUnstage={(paths) => void git.unstage(paths)}
              onDiscard={(f) => setDiscardTarget(f)}
              onResolve={(f, side) => void git.resolveConflict(f.path, side)}
              bulk={{
                label: "Marcar todos como resolvidos",
                title: "Adicionar todos os arquivos conflitantes ao índice",
                onClick: () => void git.stage(conflictFiles.map((f) => f.path)),
              }}
            />
          ) : null}
        </Card>
      )}

      {secretBlock !== null ? (
        <Card className="border-red-400/60 bg-red-50/60 p-3 dark:border-red-900/60 dark:bg-red-950/20">
          <p className="flex items-center gap-2 text-xs font-medium text-red-700 dark:text-red-300">
            <AlertTriangleIcon className="size-4" />
            {secretBlock.length} arquivo(s) com possível segredo no que este commit incluiria.
          </p>
          <p className="mt-1 text-[11px] text-red-800/80 dark:text-red-200/80">
            Nada foi commitado. O julgamento foi feito no conteúdo que está no índice do Git — não no
            arquivo do disco. Tirar do índice mantém a cópia local; se o arquivo já entrou em commits
            anteriores ele continua no histórico, e reescrever histórico remoto é decisão sua, feita
            linha a linha no terminal.
          </p>
          <ul className="mt-2 space-y-1.5">
            {secretBlock.map((f) => (
              <li key={`${f.path}#${f.reason}`} className="flex items-start gap-2">
                <div className="min-w-0 flex-1">
                  <p className="truncate font-mono text-[11px]" title={f.path}>
                    {f.path}
                  </p>
                  <p className="text-[11px] text-zinc-600 dark:text-zinc-400">
                    {f.reason} — {f.suggestion}
                  </p>
                </div>
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => void untrackBlocked(f.path)}>
                  <XIcon className="size-3.5" /> Tirar do índice
                </Button>
              </li>
            ))}
          </ul>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" onClick={() => setSecretBlock(null)}>
              Cancelar
            </Button>
            <Button size="sm" variant="danger" disabled={busy} onClick={() => void runCommit(true)}>
              Commitar assim mesmo
            </Button>
          </div>
        </Card>
      ) : null}

      <Card className="p-3">
        <Field label="Mensagem do commit">
          <Textarea
            rows={3}
            value={message}
            placeholder="Resumo em uma linha, e o motivo no corpo."
            onChange={(e) => setMessage(e.target.value)}
          />
        </Field>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <Checkbox checked={amend} onChange={setAmend} label="Regravar o último commit (amend)" />
          <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
            {git.identity?.userName
              ? `Autor: ${git.identity.userName}${git.identity.userEmail ? ` <${git.identity.userEmail}>` : ""}`
              : "O git ainda não tem user.name definido para este repositório."}
          </p>
          <Button
            variant="primary"
            className="ml-auto"
            disabled={busy || message.trim() === "" || (stagedFiles.length === 0 && !amend)}
            loading={committing}
            onClick={() => (amend ? setConfirmAmend(true) : void runCommit())}
            title={stagedFiles.length === 0 && !amend ? "Prepare ao menos um arquivo primeiro" : undefined}
          >
            <CheckIcon className="size-4" /> {amend ? "Regravar commit" : "Commitar"}
          </Button>
        </div>
      </Card>

      <Confirm
        open={discardTarget !== null}
        title="Descartar alterações"
        message={`Voltar ${discardTarget?.path} para a versão do último commit? As alterações locais desse arquivo são perdidas pelo git — não há como desfazer por aqui.`}
        confirmLabel="Descartar"
        busy={busy}
        onConfirm={async () => {
          if (!discardTarget) return;
          await git.discard(discardTarget.path);
          setDiscardTarget(null);
        }}
        onClose={() => setDiscardTarget(null)}
      />

      <Confirm
        open={confirmAmend}
        title="Regravar o último commit"
        message="O amend reescreve o commit mais recente do histórico local. Se ele já foi enviado a um remoto, o próximo push será recusado — a aplicação não força nada por conta própria. Continuar?"
        confirmLabel="Regravar"
        busy={busy}
        onConfirm={() => void runCommit()}
        onClose={() => setConfirmAmend(false)}
      />
    </div>
  );
}
