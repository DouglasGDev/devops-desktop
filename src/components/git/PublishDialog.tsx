import { useEffect, useRef, useState } from "react";
import { api } from "../../lib/api";
import type { GitRepo } from "../../lib/useGitRepo";
import { useApp } from "../../store/AppContext";
import type { SecretFinding } from "../../types";
import { AlertTriangleIcon, RocketIcon } from "../Icons";
import { Badge, Button, Field, Input, Modal, Select, Textarea } from "../ui";

/** Os comandos de uma publicação ficam salvos por projeto nesta chave. */
const cmdsKey = (projectId: number) => `git_publish_cmds:${projectId}`;

function comandosSalvos(raw: string | undefined): string[] | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    return parsed.filter((c): c is string => typeof c === "string");
  } catch {
    return null;
  }
}

/**
 * Publicação por Git, sem servidor: roda os comandos locais na pasta do
 * projeto, envia o ramo e, se pedida, cria e envia a tag. Cada etapa é executada
 * de verdade — a janela mostra a saída que o git e os comandos devolveram.
 */
export function PublishDialog({
  git,
  projectId,
  open,
  onClose,
}: {
  git: GitRepo;
  projectId: number;
  open: boolean;
  onClose: () => void;
}) {
  const { settings, pushToast } = useApp();
  const [remote, setRemote] = useState("");
  const [tag, setTag] = useState("");
  const [commands, setCommands] = useState("");
  const [blocked, setBlocked] = useState<SecretFinding[] | null>(null);
  const [log, setLog] = useState<string | null>(null);
  const seeded = useRef<string | null>(null);
  const busy = git.busy !== null;
  const status = git.status;

  useEffect(() => {
    if (!open) {
      seeded.current = null;
      return;
    }
    const marca = `${projectId}`;
    if (seeded.current === marca) return;
    seeded.current = marca;
    setRemote("");
    setTag("");
    setBlocked(null);
    setLog(null);
    setCommands(comandosSalvos(settings[cmdsKey(projectId)])?.join("\n") ?? "");
  }, [open, projectId, settings]);

  const linhas = commands
    .split("\n")
    .map((c) => c.trim())
    .filter((c) => c !== "");

  const publicar = async (allowSecrets: boolean) => {
    if (!allowSecrets) {
      const encontrados = await git.secretCheck("push");
      if (encontrados === null) return;
      const graves = encontrados.filter((f) => f.severity === "block");
      if (graves.length > 0) {
        setBlocked(graves);
        return;
      }
    }
    setBlocked(null);
    try {
      await api.setSetting(cmdsKey(projectId), JSON.stringify(linhas));
    } catch (e) {
      pushToast("error", `Os comandos desta publicação não foram lembrados: ${String(e)}`);
    }
    const resultado = await git.publish({
      remote: remote.trim() === "" ? null : remote.trim(),
      tag: tag.trim() === "" ? null : tag.trim(),
      commands: linhas,
      allowSecrets,
    });
    if (resultado?.ok) setLog(resultado.output === "" ? "Publicado." : resultado.output);
  };

  return (
    <Modal open={open} onClose={onClose} title="Publicar via git" width="max-w-2xl">
      <div className="grid gap-3">
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          Nada aqui depende de servidor: o que é publicado é o que o git envia para o remoto, depois de os
          comandos abaixo passarem na pasta do projeto.
        </p>

        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="Remoto" hint={git.remotes.map((r) => r.name).join(", ")}>
            <Select value={remote} onChange={(e) => setRemote(e.target.value)}>
              <option value="">padrão (origin, se existir)</option>
              {git.remotes.map((r) => (
                <option key={r.name} value={r.name}>
                  {r.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Tag (opcional)" hint="Vazia: publica só o ramo. Informada: cria no commit atual e envia junto.">
            <Input value={tag} onChange={(e) => setTag(e.target.value)} placeholder="v1.0.0" />
          </Field>
        </div>

        <Field
          label="Comandos antes de publicar"
          hint="Um por linha, executados na pasta do projeto. Se um falhar, nada é enviado. Ficam salvos para a próxima publicação."
        >
          <Textarea
            rows={4}
            value={commands}
            onChange={(e) => setCommands(e.target.value)}
            placeholder={"npm test\nnpm run build"}
            className="font-mono text-xs"
          />
        </Field>

        <div className="flex flex-wrap items-center gap-2 text-[11px] text-zinc-500 dark:text-zinc-400">
          <Badge tone={status?.upstream ? "success" : "neutral"}>
            {status?.upstream ? `ramo acompanhando ${status.upstream}` : "ramo sem rastreio: o envio cria a referência no remoto"}
          </Badge>
          {(status?.behind ?? 0) > 0 ? (
            <Badge tone="warning">{status?.behind} commit(s) no remoto que você ainda não trouxe — o git vai recusar o envio</Badge>
          ) : null}
          {(status?.files.length ?? 0) > 0 ? (
            <Badge tone="info">{status?.files.length} alteração(ões) sem commit não serão publicadas</Badge>
          ) : null}
        </div>

        {blocked !== null ? (
          <div className="rounded-lg border border-red-400/60 bg-red-50/60 p-2.5 dark:border-red-900/60 dark:bg-red-950/20">
            <p className="flex items-center gap-2 text-xs font-medium text-red-700 dark:text-red-300">
              <AlertTriangleIcon className="size-4" />
              {blocked.length} arquivo(s) com possível segredo no que esta publicação traria. Nada foi enviado.
            </p>
            <ul className="mt-1.5 space-y-0.5">
              {blocked.slice(0, 6).map((f) => (
                <li key={`${f.path}#${f.reason}`} className="truncate font-mono text-[11px]" title={`${f.path} — ${f.reason}`}>
                  {f.path} — {f.reason}
                </li>
              ))}
            </ul>
            <p className="mt-1.5 text-[11px] text-red-800/80 dark:text-red-200/80">
              Publicar um segredo leva ele para o remoto, onde fica no histórico mesmo se você desfizer depois.
            </p>
          </div>
        ) : null}

        {log !== null ? (
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
              Saída da publicação
            </p>
            <pre className="mt-1 max-h-56 overflow-auto rounded-lg bg-zinc-950/90 p-2.5 font-mono text-[11px] leading-relaxed text-zinc-200">
              {log}
            </pre>
          </div>
        ) : null}
      </div>

      <div className="mt-5 flex flex-wrap justify-end gap-2">
        <Button onClick={onClose} disabled={busy}>
          {log !== null ? "Fechar" : "Cancelar"}
        </Button>
        {blocked !== null ? (
          <Button variant="danger" disabled={busy} onClick={() => void publicar(true)}>
            <RocketIcon className="size-3.5" /> Publicar assim mesmo
          </Button>
        ) : null}
        <Button variant="primary" disabled={busy} onClick={() => void publicar(false)}>
          <RocketIcon className="size-3.5" /> {busy ? "Publicando…" : "Publicar"}
        </Button>
      </div>
    </Modal>
  );
}
