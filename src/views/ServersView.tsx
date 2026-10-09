import { useMemo, useState } from "react";
import { api } from "../lib/api";
import { ERROR_KIND_LABELS, formatDateTime, formatDuration } from "../lib/format";
import type { Environment, ImportSshHost, Server, ServerInput, SshDiscoveryReport } from "../types";
import { useApp } from "../store/AppContext";
import { AlertTriangleIcon, KeyIcon, PencilIcon, PlusIcon, RefreshIcon, SearchIcon, ServerIcon, TrashIcon } from "../components/Icons";
import {
  AUTH_LABELS,
  Badge,
  Button,
  Card,
  Confirm,
  EmptyState,
  ENV_LABELS,
  ENV_TONES,
  Field,
  Input,
  Modal,
  Select,
  StatusDot,
  Textarea,
} from "../components/ui";

const ENV_ORDER: Environment[] = ["dev", "staging", "prod", "database", "docker", "other"];

const EMPTY_INPUT: ServerInput = {
  name: "",
  host: "",
  port: 22,
  username: "",
  authMethod: "unknown",
  identityFile: null,
  sshAlias: null,
  environment: "dev",
  groupName: null,
  notes: null,
};

export function ServersView() {
  const { servers, reloadServers, pushToast } = useApp();
  const [filter, setFilter] = useState("");
  const [editing, setEditing] = useState<{ server: Server | null; input: ServerInput } | null>(null);
  const [deleting, setDeleting] = useState<Server | null>(null);
  const [testingId, setTestingId] = useState<number | null>(null);
  const [testDetail, setTestDetail] = useState<{ serverId: number; ok: boolean; message: string; durationMs: number } | null>(null);
  const [importOpen, setImportOpen] = useState(false);

  const grouped = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? servers.filter(
          (s) =>
            s.name.toLowerCase().includes(q) ||
            s.host.toLowerCase().includes(q) ||
            s.username.toLowerCase().includes(q) ||
            (s.groupName ?? "").toLowerCase().includes(q),
        )
      : servers;
    const map = new Map<Environment, Server[]>();
    for (const env of ENV_ORDER) map.set(env, []);
    for (const s of list) {
      const arr = map.get(s.environment);
      if (arr) arr.push(s);
    }
    return ENV_ORDER.map((env) => ({ env, items: (map.get(env) as Server[]).sort((a, b) => a.name.localeCompare(b.name, "pt-BR")) })).filter(
      (g) => g.items.length > 0,
    );
  }, [servers, filter]);

  const openAdd = () => setEditing({ server: null, input: { ...EMPTY_INPUT } });

  const openEdit = (s: Server) =>
    setEditing({
      server: s,
      input: {
        name: s.name,
        host: s.host,
        port: s.port,
        username: s.username,
        authMethod: s.authMethod,
        identityFile: s.identityFile,
        sshAlias: s.sshAlias,
        environment: s.environment,
        groupName: s.groupName,
        notes: s.notes,
      },
    });

  const save = async () => {
    if (!editing) return;
    const { server, input } = editing;
    if (!input.name.trim() || !input.host.trim() || !input.username.trim()) {
      pushToast("error", "Preencha nome, host e usuário.");
      return;
    }
    try {
      if (server) {
        await api.updateServer(server.id, input);
        pushToast("success", `Servidor “${input.name}” atualizado.`);
      } else {
        await api.addServer(input);
        pushToast("success", `Servidor “${input.name}” cadastrado.`);
      }
      setEditing(null);
      await reloadServers();
    } catch (e) {
      pushToast("error", `Falha ao salvar: ${String(e)}`);
    }
  };

  const remove = async () => {
    if (!deleting) return;
    try {
      await api.deleteServer(deleting.id);
      pushToast("info", `Servidor “${deleting.name}” removido.`);
      setDeleting(null);
      await reloadServers();
    } catch (e) {
      pushToast("error", `Falha ao remover: ${String(e)}`);
    }
  };

  const test = async (s: Server) => {
    setTestingId(s.id);
    setTestDetail(null);
    try {
      const result = await api.testConnection(s.id);
      setTestDetail({ serverId: s.id, ok: result.ok, message: result.message, durationMs: result.durationMs });
      if (result.ok) pushToast("success", `Conexão OK com “${s.name}” (${formatDuration(result.durationMs)}).`);
      else
        pushToast(
          "error",
          `${s.name}: ${ERROR_KIND_LABELS[result.errorKind ?? "other"] ?? result.errorKind ?? "Falha"} — ${result.message}`,
        );
      await reloadServers();
    } catch (e) {
      pushToast("error", `Falha ao testar “${s.name}”: ${String(e)}`);
    } finally {
      setTestingId(null);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" onClick={openAdd}>
          <PlusIcon /> Adicionar servidor
        </Button>
        <Button onClick={() => setImportOpen(true)}>
          <KeyIcon /> Importar de ~/.ssh/config
        </Button>
        <div className="relative ml-auto w-full max-w-xs">
          <SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-zinc-400" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filtrar por nome, host, usuário…"
            className="w-full rounded-lg border border-zinc-200 bg-white py-1.5 pl-8 pr-3 text-sm outline-none focus:ring-2 focus:ring-teal-500/40 dark:border-zinc-800 dark:bg-zinc-900"
          />
        </div>
      </div>

      {servers.length === 0 ? (
        <Card>
          <EmptyState
            icon={<ServerIcon className="size-8" />}
            title="Nenhum servidor cadastrado"
            description="Importe automaticamente os acessos existentes no seu ~/.ssh/config ou cadastre um servidor manualmente. As chaves originais nunca são movidas ou copiadas."
          >
            <Button variant="primary" onClick={() => setImportOpen(true)}>
              <KeyIcon /> Importar de ~/.ssh/config
            </Button>
            <Button onClick={openAdd}>
              <PlusIcon /> Cadastro manual
            </Button>
          </EmptyState>
        </Card>
      ) : (
        grouped.map(({ env, items }) => (
          <Card key={env} className="p-4">
            <div className="mb-2 flex items-center gap-2">
              <Badge tone={ENV_TONES[env]}>{ENV_LABELS[env]}</Badge>
              <span className="text-xs text-zinc-400 dark:text-zinc-500">{items.length} servidor(es)</span>
              {env === "prod" ? (
                <Badge tone="error">
                  <AlertTriangleIcon className="size-3" /> Operações exigem confirmação extra
                </Badge>
              ) : null}
            </div>
            <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
              {items.map((s) => (
                <li key={s.id} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2.5">
                  <StatusDot status={s.lastStatus} />
                  <div className="min-w-0">
                    <p className="text-sm font-medium">
                      {s.name}
                      {s.sshAlias ? <span className="ml-1.5 font-mono text-[11px] text-zinc-400">({s.sshAlias})</span> : null}
                      {s.groupName ? <span className="ml-1.5 text-[11px] text-zinc-400">· {s.groupName}</span> : null}
                    </p>
                    <p className="truncate font-mono text-xs text-zinc-500 dark:text-zinc-400">
                      {s.username}@{s.host}:{s.port} · {AUTH_LABELS[s.authMethod]}
                      {s.identityFile ? ` · ${s.identityFile}` : ""}
                    </p>
                  </div>
                  <div className="ml-auto flex shrink-0 items-center gap-1.5">
                    {s.lastStatus ? (
                      <Badge tone={s.lastStatus === "ok" ? "success" : "error"}>
                        {s.lastStatus === "ok" ? "Conectável" : "Falha"}
                      </Badge>
                    ) : null}
                    <span className="hidden text-[11px] text-zinc-400 md:inline">
                      {s.lastCheckedAt ? `testado ${formatDateTime(s.lastCheckedAt)}` : "nunca testado"}
                    </span>
                    <Button size="sm" loading={testingId === s.id} onClick={() => void test(s)}>
                      Testar
                    </Button>
                    <Button size="sm" variant="ghost" title="Editar" onClick={() => openEdit(s)}>
                      <PencilIcon className="size-3.5" />
                    </Button>
                    <Button size="sm" variant="ghost" title="Remover" onClick={() => setDeleting(s)}>
                      <TrashIcon className="size-3.5" />
                    </Button>
                  </div>
                  {testDetail && testDetail.serverId === s.id ? (
                    <p
                      className={`w-full rounded-lg px-3 py-2 text-xs ${
                        testDetail.ok
                          ? "bg-emerald-50 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300"
                          : "bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300"
                      }`}
                    >
                      {testDetail.ok ? "Conexão estabelecida" : "Falha"} em {formatDuration(testDetail.durationMs)} — {testDetail.message}
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          </Card>
        ))
      )}

      <ServerFormModal editing={editing} onChange={(input) => setEditing((prev) => (prev ? { ...prev, input } : prev))} onClose={() => setEditing(null)} onSave={() => void save()} />

      <Confirm
        open={deleting !== null}
        title="Remover servidor"
        message={`Remover “${deleting?.name}” do cadastro? A configuração original em ~/.ssh não é alterada.`}
        confirmLabel="Remover"
        onConfirm={() => void remove()}
        onClose={() => setDeleting(null)}
      />

      <ImportModal open={importOpen} onClose={() => setImportOpen(false)} onImported={reloadServers} />
    </div>
  );
}

function ServerFormModal({
  editing,
  onChange,
  onClose,
  onSave,
}: {
  editing: { server: Server | null; input: ServerInput } | null;
  onChange: (input: ServerInput) => void;
  onClose: () => void;
  onSave: () => void;
}) {
  if (!editing) return null;
  const { server, input } = editing;
  const set = <K extends keyof ServerInput>(key: K, value: ServerInput[K]) => onChange({ ...input, [key]: value });

  return (
    <Modal open onClose={onClose} title={server ? `Editar “${server.name}”` : "Adicionar servidor"}>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Nome">
          <Input value={input.name} onChange={(e) => set("name", e.target.value)} placeholder="ex.: VPS produção" />
        </Field>
        <Field label="Host / IP">
          <Input value={input.host} onChange={(e) => set("host", e.target.value)} placeholder="ex.: 203.0.113.10" />
        </Field>
        <Field label="Usuário">
          <Input value={input.username} onChange={(e) => set("username", e.target.value)} placeholder="ex.: root ou deploy" />
        </Field>
        <Field label="Porta">
          <Input
            type="number"
            min={1}
            max={65535}
            value={input.port}
            onChange={(e) => set("port", Number(e.target.value) || 22)}
          />
        </Field>
        <Field
          label="Método de autenticação"
          hint={
            input.authMethod === "password"
              ? "A senha é pedida a cada operação, usada só em memória e nunca gravada. Prefira chave para automatizar deploys."
              : undefined
          }
        >
          <Select value={input.authMethod} onChange={(e) => set("authMethod", e.target.value as ServerInput["authMethod"])}>
            <option value="unknown">Não informado (usa padrão do OpenSSH)</option>
            <option value="key">Chave privada</option>
            <option value="agent">SSH Agent</option>
            <option value="password">Senha (pedida a cada operação)</option>
          </Select>
        </Field>
        <Field label="Arquivo de identidade (opcional)" hint="Apenas a referência ao caminho é guardada; a chave nunca é copiada.">
          <Input
            value={input.identityFile ?? ""}
            onChange={(e) => set("identityFile", e.target.value.trim() === "" ? null : e.target.value.trim())}
            placeholder="ex.: ~/.ssh/id_ed25519"
          />
        </Field>
        <Field label="Alias do ~/.ssh/config (opcional)">
          <Input
            value={input.sshAlias ?? ""}
            onChange={(e) => set("sshAlias", e.target.value.trim() === "" ? null : e.target.value.trim())}
            placeholder="ex.: meu-servidor"
          />
        </Field>
        <Field label="Ambiente">
          <Select value={input.environment} onChange={(e) => set("environment", e.target.value as Environment)}>
            {ENV_ORDER.map((env) => (
              <option key={env} value={env}>
                {ENV_LABELS[env]}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="Grupo (opcional)">
          <Input
            value={input.groupName ?? ""}
            onChange={(e) => set("groupName", e.target.value.trim() === "" ? null : e.target.value.trim())}
            placeholder="ex.: Empresa X"
          />
        </Field>
        <div className="sm:col-span-2">
          <Field label="Notas (opcional)">
            <Textarea rows={2} value={input.notes ?? ""} onChange={(e) => set("notes", e.target.value.trim() === "" ? null : e.target.value.trim())} />
          </Field>
        </div>
      </div>
      <div className="mt-5 flex justify-end gap-2">
        <Button onClick={onClose}>Cancelar</Button>
        <Button variant="primary" onClick={onSave}>
          {server ? "Salvar alterações" : "Cadastrar"}
        </Button>
      </div>
    </Modal>
  );
}

function ImportModal({ open, onClose, onImported }: { open: boolean; onClose: () => void; onImported: () => Promise<void> }) {
  const { pushToast } = useApp();
  const [report, setReport] = useState<SshDiscoveryReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  const [selected, setSelected] = useState<Map<string, Environment>>(new Map());
  const [defaultEnv, setDefaultEnv] = useState<Environment>("dev");

  const discover = async () => {
    setLoading(true);
    setReport(null);
    try {
      const r = await api.discoverSsh();
      setReport(r);
      const map = new Map<string, Environment>();
      for (const h of r.hosts) if (h.isConcrete) map.set(h.alias, defaultEnv);
      setSelected(map);
    } catch (e) {
      pushToast("error", `Falha na descoberta: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  };

  const importSelected = async () => {
    if (!report) return;
    const entries: ImportSshHost[] = [];
    for (const h of report.hosts) {
      const env = selected.get(h.alias);
      if (!env || !h.isConcrete) continue;
      entries.push({
        alias: h.alias,
        host: h.hostName ?? h.alias,
        user: h.user ?? "",
        port: h.port ?? 22,
        identityFile: h.identityFiles[0] ?? null,
        environment: env,
      });
    }
    if (entries.length === 0) {
      pushToast("info", "Nenhum host selecionado para importar.");
      return;
    }
    setImporting(true);
    try {
      const created = await api.importSshHosts(entries);
      pushToast("success", `${created.length} servidor(es) importado(s).`);
      await onImported();
      onClose();
    } catch (e) {
      pushToast("error", `Falha ao importar: ${String(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const toggle = (alias: string, env: Environment | null) => {
    setSelected((prev) => {
      const next = new Map(prev);
      if (env === null) next.delete(alias);
      else next.set(alias, env);
      return next;
    });
  };

  const concrete = report ? report.hosts.filter((h) => h.isConcrete) : [];
  const selectedCount = concrete.filter((h) => selected.has(h.alias)).length;

  return (
    <Modal open={open} onClose={importing ? () => {} : onClose} title="Importar acessos do OpenSSH" width="max-w-3xl">
      <p className="text-sm text-zinc-600 dark:text-zinc-300">
        A leitura é feita apenas nos arquivos de configuração existentes. Nada é movido, modificado ou copiado — as chaves
        permanecem exatamente onde estão.
      </p>

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button loading={loading} onClick={() => void discover()}>
          <RefreshIcon /> {report ? "Atualizar detecção" : "Detectar acessos"}
        </Button>
        {report ? (
          <>
            <span className="text-xs text-zinc-500 dark:text-zinc-400">
              {concrete.length} host(s) encontrados · {selectedCount} selecionado(s)
            </span>
            <div className="ml-auto flex items-center gap-2">
              <span className="text-xs text-zinc-500 dark:text-zinc-400">Ambiente padrão:</span>
              <Select className="w-40 py-1 text-xs" value={defaultEnv} onChange={(e) => setDefaultEnv(e.target.value as Environment)}>
                {ENV_ORDER.map((env) => (
                  <option key={env} value={env}>
                    {ENV_LABELS[env]}
                  </option>
                ))}
              </Select>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  const map = new Map<string, Environment>();
                  for (const h of concrete) map.set(h.alias, defaultEnv);
                  setSelected(map);
                }}
              >
                Selecionar todos
              </Button>
            </div>
          </>
        ) : null}
      </div>

      {report ? (
        <>
          {report.warnings.length > 0 ? (
            <div className="mt-3 space-y-1 rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-900/50 dark:bg-amber-950/30 dark:text-amber-300">
              {report.warnings.map((w, i) => (
                <p key={i}>• {w}</p>
              ))}
            </div>
          ) : null}

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <div className="rounded-lg border border-zinc-200 p-3 text-xs dark:border-zinc-800">
              <p className="mb-1 font-semibold">Arquivos de configuração</p>
              {report.configFiles.length === 0 ? (
                <p className="text-zinc-500">Nenhum arquivo encontrado.</p>
              ) : (
                <ul className="space-y-0.5 font-mono text-[11px] text-zinc-600 dark:text-zinc-400">
                  {report.configFiles.map((f) => (
                    <li key={f} className="truncate">
                      {f}
                    </li>
                  ))}
                </ul>
              )}
              {report.knownHosts ? (
                <p className="mt-2 text-zinc-500 dark:text-zinc-400">
                  known_hosts: {report.knownHosts.total} entrada(s), {report.knownHosts.hashed} com hash
                </p>
              ) : (
                <p className="mt-2 text-zinc-500 dark:text-zinc-400">known_hosts não encontrado.</p>
              )}
            </div>
            <div className="rounded-lg border border-zinc-200 p-3 text-xs dark:border-zinc-800">
              <p className="mb-1 font-semibold">Agente SSH</p>
              {report.agent.available ? (
                <p className="text-zinc-600 dark:text-zinc-400">
                  Em execução · {report.agent.keys.length} chave(s) carregada(s)
                </p>
              ) : (
                <p className="text-zinc-500 dark:text-zinc-400">{report.agent.note ?? "Agente não disponível."}</p>
              )}
              <p className="mt-2 font-semibold">Chaves em ~/.ssh</p>
              <p className="text-zinc-600 dark:text-zinc-400">
                {report.privateKeys.length} privada(s) · {report.publicKeys.length} pública(s)
              </p>
            </div>
          </div>

          <div className="mt-3 max-h-72 overflow-y-auto rounded-lg border border-zinc-200 dark:border-zinc-800">
            {concrete.length === 0 ? (
              <p className="p-3 text-sm text-zinc-500 dark:text-zinc-400">
                Nenhum host utilizável encontrado (hosts com curingas, como “Host *”, são ignorados).
              </p>
            ) : (
              <table className="w-full text-sm">
                <tbody className="divide-y divide-zinc-100 dark:divide-zinc-800">
                  {concrete.map((h) => {
                    const checked = selected.has(h.alias);
                    return (
                      <tr key={h.alias} className={checked ? "bg-teal-50/50 dark:bg-teal-950/20" : ""}>
                        <td className="w-8 px-3">
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={(e) => toggle(h.alias, e.target.checked ? selected.get(h.alias) ?? defaultEnv : null)}
                            className="size-4 accent-teal-600"
                          />
                        </td>
                        <td className="py-2 pr-3">
                          <p className="font-mono text-xs font-medium">{h.alias}</p>
                          <p className="font-mono text-[11px] text-zinc-500 dark:text-zinc-400">
                            {(h.user ?? "") ? `${h.user}@` : ""}
                            {h.hostName ?? "?"}
                            {h.port && h.port !== 22 ? `:${h.port}` : ""}
                            {h.identityFiles.length > 0 ? ` · ${h.identityFiles[0]}` : ""}
                          </p>
                        </td>
                        <td className="w-44 px-3">
                          <Select
                            className="py-1 text-xs"
                            value={selected.get(h.alias) ?? defaultEnv}
                            disabled={!checked}
                            onChange={(e) => toggle(h.alias, e.target.value as Environment)}
                          >
                            {ENV_ORDER.map((env) => (
                              <option key={env} value={env}>
                                {ENV_LABELS[env]}
                              </option>
                            ))}
                          </Select>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>

          <div className="mt-4 flex justify-end gap-2">
            <Button onClick={onClose} disabled={importing}>
              Cancelar
            </Button>
            <Button variant="primary" loading={importing} onClick={() => void importSelected()}>
              Importar {selectedCount > 0 ? `(${selectedCount})` : ""}
            </Button>
          </div>
        </>
      ) : loading ? (
        <p className="mt-6 text-center text-sm text-zinc-500">Lendo configurações do OpenSSH…</p>
      ) : null}
    </Modal>
  );
}
