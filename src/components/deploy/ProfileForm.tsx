import { useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import type { DeployProfile, DeployProfileInput, DeployStrategy, Project } from "../../types";
import { Button, Checkbox, Field, Modal, Select, STRATEGY_HINTS, STRATEGY_LABELS, Textarea, Input } from "../ui";

const STRATEGIES: DeployStrategy[] = ["sftp", "git", "artifacts", "script", "docker"];
/** Estratégias que funcionam sem servidor: nada de SFTP, Git remoto ou Docker remoto. */
const STRATEGIES_LOCAIS: DeployStrategy[] = ["artifacts", "script"];
/** Valor sentinela do <Select>: o servidor "zero" é a máquina local. */
const MAQUINA_LOCAL = 0;
/** No modo sem servidor a estratégia é a mesma, mas o destino muda o texto. */
const STRATEGY_HINTS_LOCAIS: Record<string, string> = {
  artifacts: "Compila nesta máquina e copia apenas os artefatos listados. Nem o node_modules nem o código-fonte vão para o destino.",
  script: "Executa a sequência de comandos que você configurou nesta máquina, com log e código de saída de cada etapa.",
};

/** O mesmo critério do motor: caminho absoluto de pasta, sem "..". */
function isLocalAbsolutePath(path: string): boolean {
  const raw = path.trim();
  if (raw === "" || raw.length > 512 || raw.includes("\0") || raw.includes("..") || raw.includes("\n") || raw.includes("\r"))
    return false;
  const unificado = raw.replace(/\\/g, "/");
  if (unificado.startsWith("//")) return unificado.split("/").filter((p) => p !== "").length >= 2;
  if (unificado.startsWith("/")) return unificado.replace(/^\/+|\/+$/g, "") !== "";
  return /^[A-Za-z]:\/.+/.test(unificado);
}

function isRemoteAbsolutePath(path: string): boolean {
  const raw = path.trim();
  return raw.startsWith("/") && !raw.includes("\n") && !raw.includes("\r");
}

/** Caminho comparável: barras à frente, sem repetir nem sobrar separador. */
function normPath(path: string): string {
  return path
    .trim()
    .replace(/\\/g, "/")
    .replace(/\/{2,}/g, "/")
    .replace(/\/+$/, "");
}

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
  /** `null` é o modo sem servidor: publica nesta máquina. */
  serverId: number | null;
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
    // Sem servidor cadastrado, o único destino possível é esta máquina.
    serverId: servers[0]?.id ?? null,
    strategy: servers[0] ? "sftp" : "artifacts",
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

  const localMode = draft.serverId === null;
  const estrategias = localMode ? STRATEGIES_LOCAIS : STRATEGIES;

  /** Trocar o destino troca também o que faz sentido publicar ali. */
  const escolherServidor = (id: number) => {
    setDraft((current) => {
      const serverId = id === MAQUINA_LOCAL ? null : id;
      const semHost = serverId === null;
      return {
        ...current,
        serverId,
        strategy: semHost && !STRATEGIES_LOCAIS.includes(current.strategy) ? "artifacts" : current.strategy,
        // release por symlink é um mecanismo do servidor
        releasesEnabled: semHost ? false : current.releasesEnabled,
      };
    });
  };

  /** Trocar de estratégia traz a branch do projeto quando o Git precisa dela. */
  const escolherEstrategia = (strategy: DeployStrategy) =>
    setDraft((current) => ({
      ...current,
      strategy,
      gitBranch:
        strategy === "git" && current.gitBranch.trim() === "" ? project.gitBranch ?? "main" : current.gitBranch,
    }));

  const enviar = async () => {
    if (!draft.name.trim()) {
      setError("dê um nome ao modelo (ex.: produção, homologação)");
      return;
    }
    const destino = draft.remotePath.trim();
    if (localMode) {
      if (!isLocalAbsolutePath(destino)) {
        setError("a pasta de destino precisa ser um caminho absoluto nesta máquina, como D:\\publicados\\site");
        return;
      }
      const alvo = normPath(destino);
      const origem = normPath(project.localPath);
      if (origem !== "" && (alvo === origem || alvo.startsWith(`${origem}/`))) {
        setError("a pasta de destino não pode ficar dentro da pasta do projeto — escolha uma pasta separada");
        return;
      }
    } else if (!isRemoteAbsolutePath(destino)) {
      setError("o diretório remoto precisa ser um caminho absoluto, como /var/www/site");
      return;
    }
    if (draft.strategy === "script" && toLines(draft.prepareCommands).length === 0) {
      setError("um deploy por script precisa de pelo menos um comando de preparação");
      return;
    }
    if (draft.strategy === "git" && draft.gitBranch.trim() === "") {
      setError("escolha a branch que o servidor vai publicar");
      return;
    }
    if (
      draft.strategy === "docker" &&
      toLines(draft.buildCommands).length === 0 &&
      toLines(draft.restartCommands).length === 0
    ) {
      setError("um deploy com Docker precisa de comandos de build ou de reinicialização dos containers");
      return;
    }
    if (draft.stepTimeoutSecs < 10 || draft.stepTimeoutSecs > 7200) {
      setError("o tempo limite por etapa fica entre 10 e 7200 segundos");
      return;
    }
    const input: DeployProfileInput = {
      id: profile?.id ?? null,
      projectId: project.id,
      name: draft.name.trim(),
      serverId: draft.serverId,
      strategy: draft.strategy,
      gitBranch: draft.gitBranch.trim() === "" ? null : draft.gitBranch.trim(),
      remotePath: destino,
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
      releasesEnabled: localMode ? false : draft.releasesEnabled,
      deleteExtras: draft.deleteExtras,
      stepTimeoutSecs: draft.stepTimeoutSecs,
      isDefault: draft.isDefault,
    };
    setSaving(true);
    setError(null);
    try {
      const saved = await api.saveDeployProfile(input);
      pushToast(
        "success",
        `Modelo “${saved.name}” salvo${saved.serverName ? ` para ${saved.serverName}` : " para esta máquina"}.`,
      );
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
          <Field label="Destino">
            <Select
              value={draft.serverId ?? MAQUINA_LOCAL}
              onChange={(e) => escolherServidor(Number(e.target.value))}
            >
              <option value={MAQUINA_LOCAL}>Nesta máquina (sem SSH)</option>
              {servers.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} · {s.host}
                </option>
              ))}
            </Select>
          </Field>
        </div>

        <Field label="Estratégia de publicação" hint={(localMode ? STRATEGY_HINTS_LOCAIS : STRATEGY_HINTS)[draft.strategy]}>
          <Select value={draft.strategy} onChange={(e) => escolherEstrategia(e.target.value as DeployStrategy)}>
            {estrategias.map((s) => (
              <option key={s} value={s}>
                {STRATEGY_LABELS[s]}
              </option>
            ))}
          </Select>
        </Field>

        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={localMode ? "Pasta de destino nesta máquina" : "Diretório remoto do projeto"}>
            <Input
              spellCheck={false}
              className="font-mono text-xs"
              value={draft.remotePath}
              onChange={(e) => set("remotePath", e.target.value)}
              placeholder={localMode ? "D:/publicados/site" : "/var/www/site"}
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
          localMode
            ? "Executados nesta máquina antes da cópia. Um comando por linha."
            : "Executados no servidor antes do envio. Um comando por linha.",
          localMode ? "if not exist \"D:\\publicados\\site\" mkdir \"D:\\publicados\\site\"" : "mkdir -p /var/www/site",
        )}
        {lista(
          "testCommands",
          "Comandos de teste",
          localMode
            ? "Executados nesta máquina antes de qualquer mudança no destino (etapa 6). Sem itens, a etapa é registrada como não executada."
            : "Executados no computador antes de qualquer mudança no servidor (etapa 6). Sem itens, a etapa é registrada como não executada.",
          "npm run test\nnpm run lint",
        )}
        {lista(
          "buildCommands",
          "Comandos de build",
          localMode
            ? "Executados nesta máquina, antes de copiar qualquer coisa."
            : "Executados no computador local, antes de enviar qualquer coisa.",
          "npm ci && npm run build",
        )}
        {lista(
          "restartCommands",
          "Comandos de reinicialização",
          localMode
            ? "Executados nesta máquina depois da cópia (recarregar serviço, limpar cache)."
            : "Executados no servidor depois do envio (recarregar serviço, limpar cache).",
          localMode ? "iisreset /restart" : "sudo systemctl reload caddy",
        )}
        {lista(
          "healthChecks",
          "Testes de saúde",
          localMode
            ? "Comandos de leitura executados nesta máquina ao final. Qualquer falha marca o deploy como fracassado."
            : "Comandos de leitura executados no servidor ao final. Qualquer falha marca o deploy como fracassado.",
          "curl -fsS http://127.0.0.1/healthz",
        )}
        {lista(
          "rollbackCommands",
          "Comandos de rollback",
          "Usados pela ação de retorno ao estado anterior.",
          localMode ? "copy /Y D:\\publicados\\guarda\\* D:\\publicados\\site" : "ln -sfn /var/www/releases/anterior /var/www/site/current",
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
            label={
              localMode
                ? "Guardar o que já está no destino antes de sobrescrever (ponto de recuperação)"
                : "Criar backup do diretório atual antes de publicar"
            }
          />
          {!localMode && (
            <Checkbox
              checked={draft.releasesEnabled}
              onChange={(v) => set("releasesEnabled", v)}
              label="Publicar em releases separadas e trocar o symlink ativo (rollback mais seguro)"
            />
          )}
          <Checkbox
            checked={draft.deleteExtras}
            onChange={(v) => set("deleteExtras", v)}
            label={
              <span>
                Apagar no destino os arquivos que não existem na origem
                <span className="block text-[11px] text-red-600 dark:text-red-400">
                  {draft.deleteExtras && !draft.backupEnabled && localMode
                    ? "Sem ponto de recuperação: os arquivos removidos do destino não voltam."
                    : "Operação destrutiva: só marque se você tem certeza de que o destino não guarda nada além do conteúdo deste projeto."}
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
