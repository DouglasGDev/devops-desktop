import { useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import type { DeployProfile, DeployProfileInput, DeployStrategy, Project } from "../../types";
import { Button, Checkbox, Field, Modal, Select, STRATEGY_HINTS, STRATEGY_LABELS, Textarea, Input } from "../ui";

const STRATEGIES: DeployStrategy[] = ["sftp", "git", "artifacts", "script", "docker"];

function toLines(value: string): string[] {
  return value
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");
}

function fromLines(items: string[]): string {
  return items.join("\n");
}

type Draft = {
  name: string;
  serverId: number;
  strategy: DeployStrategy;
  gitBranch: string;
  remotePath: string;
  includePaths: string;
  excludePaths: string;
  prepareCommands: string;
  buildCommands: string;
  restartCommands: string;
  healthChecks: string;
  rollbackCommands: string;
  testCommands: string;
  secretKeys: string;
  backupEnabled: boolean;
  releasesEnabled: boolean;
  deleteExtras: boolean;
  stepTimeoutSecs: number;
  isDefault: boolean;
};

function draftFrom(profile: DeployProfile | null, servers: { id: number }[]): Draft {
  if (profile) {
    return {
      name: profile.name,
      serverId: profile.serverId,
      strategy: profile.strategy,
      gitBranch: profile.gitBranch ?? "",
      remotePath: profile.remotePath,
      includePaths: fromLines(profile.includePaths),
      excludePaths: fromLines(profile.excludePaths),
      prepareCommands: fromLines(profile.prepareCommands),
      buildCommands: fromLines(profile.buildCommands),
      restartCommands: fromLines(profile.restartCommands),
      healthChecks: fromLines(profile.healthChecks),
      rollbackCommands: fromLines(profile.rollbackCommands),
      testCommands: fromLines(profile.testCommands),
      secretKeys: fromLines(profile.secretKeys),
      backupEnabled: profile.backupEnabled,
      releasesEnabled: profile.releasesEnabled,
      deleteExtras: profile.deleteExtras,
      stepTimeoutSecs: profile.stepTimeoutSecs,
      isDefault: profile.isDefault,
    };
  }
  return {
    name: "",
    serverId: servers[0]?.id ?? 0,
    strategy: "sftp",
    gitBranch: "",
    remotePath: "",
    includePaths: "",
    excludePaths: "node_modules\n.git\ntarget\ndist/**/*.map",
    prepareCommands: "",
    buildCommands: "",
    restartCommands: "",
    healthChecks: "",
    rollbackCommands: "",
    testCommands: "",
    secretKeys: "",
    backupEnabled: true,
    releasesEnabled: false,
    deleteExtras: false,
    stepTimeoutSecs: 600,
    isDefault: false,
  };
}

/**
 * Editor de um modelo de deploy. Salvar não executa nada: a publicação só
 * acontece pelo motor de deploy, a partir de um modelo já confirmado aqui.
 */
export function ProfileForm({
  project,
  profile,
  onClose,
  onSaved,
}: {
  project: Project;
  profile: DeployProfile | null;
  onClose: () => void;
  onSaved: (saved: DeployProfile) => void;
}) {
  const { servers, pushToast } = useApp();
  const [draft, setDraft] = useState<Draft>(() => draftFrom(profile, servers));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const enviar = async () => {
    if (servers.length === 0) {
      setError("cadastre um servidor SSH antes de criar o modelo");
      return;
    }
    if (!draft.name.trim()) {
      setError("dê um nome ao modelo (ex.: produção, homologação)");
      return;
    }
    if (!draft.remotePath.trim().startsWith("/")) {
      setError("o diretório remoto precisa ser um caminho absoluto, como /var/www/site");
      return;
    }
    const input: DeployProfileInput = {
      id: profile?.id ?? null,
      projectId: project.id,
      name: draft.name.trim(),
      serverId: draft.serverId,
      strategy: draft.strategy,
      gitBranch: draft.gitBranch.trim() === "" ? null : draft.gitBranch.trim(),
      remotePath: draft.remotePath.trim(),
      includePaths: toLines(draft.includePaths),
      excludePaths: toLines(draft.excludePaths),
      prepareCommands: toLines(draft.prepareCommands),
      buildCommands: toLines(draft.buildCommands),
      restartCommands: toLines(draft.restartCommands),
      healthChecks: toLines(draft.healthChecks),
      rollbackCommands: toLines(draft.rollbackCommands),
      testCommands: toLines(draft.testCommands),
      secretKeys: toLines(draft.secretKeys),
      backupEnabled: draft.backupEnabled,
      releasesEnabled: draft.releasesEnabled,
      deleteExtras: draft.deleteExtras,
      stepTimeoutSecs: draft.stepTimeoutSecs,
      isDefault: draft.isDefault,
    };
    setSaving(true);
    setError(null);
    try {
      const saved = await api.saveDeployProfile(input);
      pushToast("success", `Modelo “${saved.name}” salvo para ${saved.serverName}.`);
      onSaved(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const lista = (key: keyof Draft, label: string, hint: string, placeholder: string) => (
    <Field label={label} hint={hint}>
      <Textarea
        rows={3}
        spellCheck={false}
        className="font-mono text-xs"
        placeholder={placeholder}
        value={String(draft[key] ?? "")}
        onChange={(e) => set(key, e.target.value as Draft[typeof key])}
      />
    </Field>
  );

  return (
    <Modal
      open
      onClose={onClose}
      title={profile ? `Editar modelo “${profile.name}”` : `Novo modelo de deploy — ${project.name}`}
      width="max-w-2xl"
    >
      <div className="max-h-[65vh] space-y-3 overflow-y-auto pr-1">
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="Nome do modelo">
            <Input value={draft.name} onChange={(e) => set("name", e.target.value)} placeholder="produção" />
          </Field>
          <Field label="Servidor de destino">
            <Select value={draft.serverId} onChange={(e) => set("serverId", Number(e.target.value))}>
              {servers.length === 0 ? <option value={0}>Nenhum servidor cadastrado</option> : null}
              {servers.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} · {s.host}
                </option>
              ))}
            </Select>
          </Field>
        </div>

        <Field label="Estratégia de publicação" hint={STRATEGY_HINTS[draft.strategy]}>
          <Select value={draft.strategy} onChange={(e) => set("strategy", e.target.value as DeployStrategy)}>
            {STRATEGIES.map((s) => (
              <option key={s} value={s}>
                {STRATEGY_LABELS[s]}
              </option>
            ))}
          </Select>
        </Field>

        <div className="grid gap-3 sm:grid-cols-2">
          <Field
            label="Diretório remoto do projeto"
            hint="Caminho absoluto no servidor. É para onde os arquivos vão."
          >
            <Input
              spellCheck={false}
              className="font-mono text-xs"
              value={draft.remotePath}
              onChange={(e) => set("remotePath", e.target.value)}
              placeholder="/var/www/site"
            />
          </Field>
          {draft.strategy === "git" ? (
            <Field label="Branch de origem" hint="O servidor faz checkout desta branch no repositório.">
              <Input
                spellCheck={false}
                className="font-mono text-xs"
                value={draft.gitBranch}
                onChange={(e) => set("gitBranch", e.target.value)}
                placeholder={project.gitBranch ?? "main"}
              />
            </Field>
          ) : (
            <Field label="Tempo limite por etapa (segundos)" hint="Entre 10 e 7200. Etapas que excederem o limite são interrompidas.">
              <Input
                type="number"
                min={10}
                max={7200}
                value={draft.stepTimeoutSecs}
                onChange={(e) => set("stepTimeoutSecs", Number(e.target.value) || 0)}
              />
            </Field>
          )}
        </div>

        {draft.strategy === "git" ? (
          <Field label="Tempo limite por etapa (segundos)" hint="Entre 10 e 7200. Etapas que excederem o limite são interrompidas.">
            <Input
              type="number"
              min={10}
              max={7200}
              value={draft.stepTimeoutSecs}
              onChange={(e) => set("stepTimeoutSecs", Number(e.target.value) || 0)}
            />
          </Field>
        ) : null}

        <div className="grid gap-3 sm:grid-cols-2">
          {lista(
            "includePaths",
            "Arquivos e pastas a enviar",
            "Um item por linha. Vazio = todo o projeto, menos o que estiver em “ignorar”.",
            "dist\npublic",
          )}
          {lista(
            "excludePaths",
            "Arquivos e pastas ignorados",
            "Um item por linha. Aceita padrões como **/*.map e .env*.",
            "node_modules\n.env",
          )}
        </div>

        {lista(
          "prepareCommands",
          "Comandos de preparação",
          "Executados no servidor antes do envio. Um comando por linha.",
          "mkdir -p /var/www/site",
        )}
        {lista(
          "testCommands",
          "Comandos de teste",
          "Executados no computador antes de qualquer mudança no servidor (etapa 6). Sem itens, a etapa é registrada como não executada.",
          "npm run test\nnpm run lint",
        )}
        {lista(
          "buildCommands",
          "Comandos de build",
          "Executados no computador local, antes de enviar qualquer coisa.",
          "npm ci && npm run build",
        )}
        {lista(
          "restartCommands",
          "Comandos de reinicialização",
          "Executados no servidor depois do envio (recarregar serviço, limpar cache).",
          "sudo systemctl reload caddy",
        )}
        {lista(
          "healthChecks",
          "Testes de saúde",
          "Comandos de leitura executados no servidor ao final. Qualquer falha marca o deploy como fracassado.",
          "curl -fsS http://127.0.0.1/healthz",
        )}
        {lista(
          "rollbackCommands",
          "Comandos de rollback",
          "Usados pela ação de retorno ao estado anterior.",
          "ln -sfn /var/www/releases/anterior /var/www/site/current",
        )}

        {lista(
          "secretKeys",
          "Variáveis e segredos (somente os nomes)",
          "Os valores são pedidos na hora do deploy, usados uma vez e nunca gravados no banco nem nos logs.",
          "SENTRY_TOKEN\nDATABASE_URL",
        )}

        <div className="space-y-2 rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
          <Checkbox
            checked={draft.backupEnabled}
            onChange={(v) => set("backupEnabled", v)}
            label="Criar backup do diretório atual antes de publicar"
          />
          <Checkbox
            checked={draft.releasesEnabled}
            onChange={(v) => set("releasesEnabled", v)}
            label="Publicar em releases separadas e trocar o symlink ativo (rollback mais seguro)"
          />
          <Checkbox
            checked={draft.deleteExtras}
            onChange={(v) => set("deleteExtras", v)}
            label={
              <span>
                Apagar no destino os arquivos que não existem na origem
                <span className="block text-[11px] text-red-600 dark:text-red-400">
                  Operação destrutiva: só marque se você tem certeza de que o destino não guarda nada além do
                  conteúdo deste projeto.
                </span>
              </span>
            }
          />
          <Checkbox checked={draft.isDefault} onChange={(v) => set("isDefault", v)} label="Usar este modelo como padrão do projeto" />
        </div>

        {error ? (
          <p className="rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">{error}</p>
        ) : null}
      </div>

      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onClose} disabled={saving}>
          Cancelar
        </Button>
        <Button variant="primary" loading={saving} onClick={() => void enviar()}>
          Salvar modelo
        </Button>
      </div>
    </Modal>
  );
}
