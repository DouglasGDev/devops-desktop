import { useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../../lib/api";
import { formatAgo, formatKilobytes } from "../../lib/format";
import { useApp } from "../../store/AppContext";
import type { GitHubRepo, RemoteInfo, ViewId } from "../../types";
import { CloneDialog } from "../git/CloneDialog";
import { CloudIcon, ExternalLinkIcon, FolderIcon, GitBranchIcon, TerminalIcon } from "../Icons";
import { Badge, Button, Card, Confirm, EmptyState, Field, Select, SectionTitle, Spinner } from "../ui";

/**
 * A capa que liga o repositório remoto aos diretórios desta máquina: o que o
 * GitHub devolve sobre o repo, quais projetos cadastrados apontam para ele e a
 * única escrita local que a tela faz — a URL do origin.
 */
export function RepoOverview({ repo, onNavigate }: { repo: GitHubRepo; onNavigate: (v: ViewId) => void }) {
  const { projects, pushToast } = useApp();

  const [clonando, setClonando] = useState<string | null>(null);
  const [projetoId, setProjetoId] = useState<number | "">("");
  const [remotos, setRemotos] = useState<RemoteInfo[] | null>(null);
  const [vinculando, setVinculando] = useState(false);
  const [confirmar, setConfirmar] = useState<null | { substituir: boolean }>(null);

  const repositorios = useMemo(() => projects.filter((p) => p.isGitRepo), [projects]);

  // Um projeto aponta para este repo quando o remoto cadastrado carrega o mesmo
  // `dono/repositorio`; a leitura vem do git local, não de suposição.
  const daqui = useMemo(
    () => projects.filter((p) => (p.gitRemote ?? "").toLowerCase().includes(repo.fullName.toLowerCase())),
    [projects, repo.fullName],
  );

  useEffect(() => {
    if (projetoId === "") {
      setRemotos(null);
      return;
    }
    let vivo = true;
    api
      .gitRemotes(projetoId)
      .then((r) => {
        if (vivo) setRemotos(r);
      })
      .catch(() => {
        if (vivo) setRemotos([]);
      });
    return () => {
      vivo = false;
    };
  }, [projetoId]);

  const origin = (remotos ?? []).find((r) => r.name === "origin") ?? null;

  const vincular = async (substituir: boolean) => {
    if (projetoId === "") return;
    setVinculando(true);
    setConfirmar(null);
    try {
      const novos = substituir
        ? await api.gitSetRemoteUrl(projetoId, "origin", repo.cloneUrl)
        : await api.gitAddRemote(projetoId, "origin", repo.cloneUrl);
      setRemotos(novos);
      pushToast("info", `origin de ${repo.fullName} vinculado. Enviar commits é ação sua, na seção Git.`);
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setVinculando(false);
    }
  };

  return (
    <div className="space-y-3">
      <Card className="p-4">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0">
            <p className="flex flex-wrap items-center gap-2 font-mono text-sm">
              {repo.fullName}
              <Badge tone={repo.private ? "warning" : "accent"}>{repo.private ? "privado" : "público"}</Badge>
              {repo.archived ? <Badge tone="neutral">arquivado</Badge> : null}
              {repo.fork ? <Badge tone="neutral">fork</Badge> : null}
            </p>
            <p className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-zinc-500 dark:text-zinc-400">
              <span className="flex items-center gap-1">
                <GitBranchIcon className="size-3" /> {repo.defaultBranch || "sem branch"}
              </span>
              <span>{repo.language ?? "sem linguagem detectada"}</span>
              <span>{formatKilobytes(repo.sizeKb)}</span>
              <span>{repo.pushedAt ? `último push ${formatAgo(repo.pushedAt)}` : "sem push registrado"}</span>
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <Button size="sm" onClick={() => setClonando(repo.cloneUrl)}>
              <FolderIcon className="size-3.5" /> Clonar para pasta local
            </Button>
            <Button size="sm" variant="secondary" onClick={() => setClonando(repo.sshUrl)}>
              <TerminalIcon className="size-3.5" /> Clonar via SSH
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                void openUrl(repo.htmlUrl).catch(() => pushToast("error", `Abra manualmente em ${repo.htmlUrl}`))
              }
            >
              <ExternalLinkIcon className="size-3.5" /> Site
            </Button>
          </div>
        </div>
      </Card>

      <Card className="p-4">
        <div className="mb-2 flex items-center gap-2">
          <CloudIcon className="size-4 text-teal-500" />
          <SectionTitle>Esta máquina</SectionTitle>
          <Badge tone={daqui.length > 0 ? "success" : "neutral"}>{daqui.length}</Badge>
        </div>
        {daqui.length === 0 ? (
          <EmptyState
            icon={<FolderIcon className="size-8" />}
            title={`Nenhum projeto cadastrado aponta para ${repo.fullName}`}
            description="Clone o repositório, ou abra a pasta que já existe no disco; o vínculo fica na seção Projetos."
          >
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" variant="primary" onClick={() => setClonando(repo.cloneUrl)}>
                <FolderIcon className="size-3.5" /> Clonar agora
              </Button>
              <Button size="sm" onClick={() => onNavigate("git")}>
                <GitBranchIcon className="size-3.5" /> Abrir pasta local
              </Button>
            </div>
          </EmptyState>
        ) : (
          <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
            {daqui.map((p) => (
              <li key={p.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
                <span className="text-sm">{p.name}</span>
                <span className="font-mono text-[11px] text-zinc-500 dark:text-zinc-400">{p.gitBranch ?? "fora de branch"}</span>
                <span className="ml-auto flex items-center gap-1.5">
                  <Button size="sm" variant="ghost" onClick={() => onNavigate("git")}>
                    <GitBranchIcon className="size-3.5" /> Abrir na seção Git
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card className="p-4">
        <SectionTitle>Vincular um projeto a este repositório</SectionTitle>
        <div className="mt-2 flex flex-wrap items-end gap-2">
          <Field label="Projeto cadastrado">
            <Select
              className="min-w-52"
              value={projetoId === "" ? "" : String(projetoId)}
              onChange={(e) => setProjetoId(e.target.value === "" ? "" : Number(e.target.value))}
            >
              <option value="">selecione…</option>
              {repositorios.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </Select>
          </Field>
          <Button
            variant="primary"
            disabled={projetoId === "" || vinculando}
            loading={vinculando}
            onClick={() => setConfirmar({ substituir: origin !== null })}
          >
            Vincular como origin
          </Button>
        </div>
        {projetoId !== "" ? (
          <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
            {remotos === null ? (
              <span className="flex items-center gap-2">
                <Spinner className="size-3" /> lendo os remotos deste projeto…
              </span>
            ) : origin ? (
              <>Este projeto já tem origin ({origin.fetchUrl ?? "sem URL de busca"}). Vincular substitui a URL.</>
            ) : (
              <>Este projeto ainda não tem um remoto chamado origin.</>
            )}
          </p>
        ) : null}
        {repositorios.length === 0 ? (
          <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
            Nenhum repositório git está cadastrado nesta máquina ainda.
          </p>
        ) : null}
      </Card>

      {clonando !== null ? (
        <CloneDialog open initialUrl={clonando} onClose={() => setClonando(null)} />
      ) : null}

      <Confirm
        open={confirmar !== null}
        busy={vinculando}
        title={confirmar?.substituir ? "Substituir a URL do origin" : "Adicionar o remoto origin"}
        message={
          confirmar?.substituir
            ? `A URL do origin do projeto selecionado passa a ser ${repo.cloneUrl}.`
            : `Será criado o remoto origin apontando para ${repo.cloneUrl}.`
        }
        confirmLabel={confirmar?.substituir ? "Substituir" : "Adicionar"}
        onConfirm={() => void vincular(confirmar?.substituir === true)}
        onClose={() => setConfirmar(null)}
      >
        <p className="text-[11px] text-zinc-500 dark:text-zinc-400">
          A ação grava apenas <span className="font-mono">remote.origin.url</span> na configuração local do projeto.
          Nenhum commit, push ou branch é feito por conta própria.
        </p>
      </Confirm>
    </div>
  );
}
