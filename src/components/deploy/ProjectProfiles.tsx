import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import type { DeployProfile, Project } from "../../types";
import { TrashIcon, PencilIcon, PlusIcon, StarIcon } from "../Icons";
import { Badge, Button, Confirm, DESTINO_LOCAL, ENV_TONES, Spinner, STRATEGY_LABELS } from "../ui";
import { ProfileForm } from "./ProfileForm";

type Editing = { kind: "new" } | { kind: "existing"; profile: DeployProfile } | null;

/** Modelos de deploy de um projeto: quem publica, para onde e como. */
export function ProjectProfiles({ project }: { project: Project }) {
  const { pushToast } = useApp();
  const [profiles, setProfiles] = useState<DeployProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Editing>(null);
  const [deleting, setDeleting] = useState<DeployProfile | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setProfiles(await api.listDeployProfiles(project.id));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [project.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const markDefault = async (profile: DeployProfile) => {
    try {
      await api.setDefaultDeployProfile(profile.id);
      await load();
    } catch (e) {
      pushToast("error", `Falha ao definir o padrão: ${String(e)}`);
    }
  };

  const remove = async () => {
    if (!deleting) return;
    try {
      await api.deleteDeployProfile(deleting.id);
      pushToast("info", `Modelo “${deleting.name}” removido. Nenhum arquivo do destino foi tocado.`);
      setDeleting(null);
      await load();
    } catch (e) {
      pushToast("error", `Falha ao remover o modelo: ${String(e)}`);
    }
  };

  return (
    <div className="space-y-2">
      {loading ? (
        <p className="flex items-center gap-2 py-2 text-xs text-zinc-500">
          <Spinner className="size-3.5" /> Carregando modelos de deploy…
        </p>
      ) : null}
      {error ? <p className="rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">{error}</p> : null}

      {!loading && profiles.length === 0 ? (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          Nenhum modelo ainda. Cada modelo define o destino — servidor SSH ou pasta desta máquina —, a estratégia e os
          comandos.
        </p>
      ) : null}

      <ul className="space-y-1.5">
        {profiles.map((p) => (
          <li
            key={p.id}
            className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-zinc-200 px-3 py-2 dark:border-zinc-800"
          >
            <div className="min-w-0 flex-1">
              <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
                {p.name}
                {p.isDefault ? (
                  <Badge tone="accent">
                    <StarIcon className="size-3" /> padrão
                  </Badge>
                ) : null}
                <Badge>{STRATEGY_LABELS[p.strategy] ?? p.strategy}</Badge>
                <Badge tone={ENV_TONES[p.serverEnvironment ?? "other"] ?? "neutral"}>
                  {p.serverName ?? DESTINO_LOCAL}
                </Badge>
              </p>
              <p className="truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400" title={p.remotePath}>
                {p.remotePath}
                {p.strategy === "git" && p.gitBranch ? ` · branch ${p.gitBranch}` : ""}
              </p>
              <p className="text-[11px] text-zinc-400 dark:text-zinc-500">
                {p.buildCommands.length} build · {p.testCommands.length} teste(s) · {p.restartCommands.length}{" "}
                reinicialização · {p.healthChecks.length} teste(s) de saúde · {p.secretKeys.length} variável(is)
                {p.backupEnabled ? " · backup" : ""}
                {p.releasesEnabled ? " · releases+symlink" : ""}
                {p.deleteExtras ? " · apaga excedentes" : ""}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {!p.isDefault ? (
                <Button size="sm" variant="ghost" title="Tornar o modelo padrão deste projeto" onClick={() => void markDefault(p)}>
                  <StarIcon className="size-3.5" />
                </Button>
              ) : null}
              <Button
                size="sm"
                variant="ghost"
                title="Editar modelo"
                onClick={() => setEditing({ kind: "existing", profile: p })}
              >
                <PencilIcon className="size-3.5" />
              </Button>
              <Button size="sm" variant="ghost" title="Remover modelo" onClick={() => setDeleting(p)}>
                <TrashIcon className="size-3.5" />
              </Button>
            </div>
          </li>
        ))}
      </ul>

      <Button size="sm" onClick={() => setEditing({ kind: "new" })}>
        <PlusIcon className="size-3.5" /> Novo modelo de deploy
      </Button>

      {editing ? (
        <ProfileForm
          project={project}
          profile={editing.kind === "existing" ? editing.profile : null}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void load();
          }}
        />
      ) : null}

      <Confirm
        open={deleting !== null}
        title="Remover modelo de deploy"
        message={`Remover o modelo “${deleting?.name}”? O histórico de deploys já executados continua no registro de auditoria, e nenhum arquivo do destino é alterado.`}
        confirmLabel="Remover"
        onConfirm={() => void remove()}
        onClose={() => setDeleting(null)}
      />
    </div>
  );
}
