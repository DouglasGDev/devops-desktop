import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../lib/api";
import { formatDateTime, formatKilobytes } from "../lib/format";
import type {
  GitHubDeviceFlow,
  GitHubPull,
  GitHubRepo,
  GitHubStatus,
  RemoteInfo,
} from "../types";
import { useApp } from "../store/AppContext";
import {
  AlertTriangleIcon,
  CloudIcon,
  ExternalLinkIcon,
  KeyIcon,
  RefreshIcon,
} from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  Confirm,
  EmptyState,
  Field,
  Input,
  SectionTitle,
  Select,
  Spinner,
  Textarea,
} from "../components/ui";

const DEVELOPERS_URL = "https://github.com/settings/developers";

type Puxada = { loading: boolean; items: GitHubPull[] | null; error: string | null };

export function GitHubView() {
  const { pushToast, projects } = useApp();

  const [status, setStatus] = useState<GitHubStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [savingId, setSavingId] = useState(false);

  const [flow, setFlow] = useState<GitHubDeviceFlow | null>(null);
  const [starting, setStarting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const [repos, setRepos] = useState<GitHubRepo[] | null>(null);
  const [reposLoading, setReposLoading] = useState(false);
  const [reposError, setReposError] = useState<string | null>(null);

  const [pulls, setPulls] = useState<Record<string, Puxada>>({});

  const [novoNome, setNovoNome] = useState("");
  const [novoDescricao, setNovoDescricao] = useState("");
  const [novoPrivado, setNovoPrivado] = useState(true);
  const [confirmando, setConfirmando] = useState(false);
  const [criando, setCriando] = useState(false);

  const [projetoId, setProjetoId] = useState<number | "">("");
  const [repoAlvo, setRepoAlvo] = useState("");
  const [remotosDoProjeto, setRemotosDoProjeto] = useState<RemoteInfo[] | null>(null);
  const [vinculando, setVinculando] = useState(false);
  const [confirmarVinculo, setConfirmarVinculo] = useState<null | { substituir: boolean }>(null);

  const recarregarStatus = useCallback(async () => {
    try {
      setStatus(await api.githubStatus());
    } catch (e) {
      pushToast("error", `Falha ao ler o estado do GitHub: ${String(e)}`);
    }
  }, [pushToast]);

  useEffect(() => {
    void recarregarStatus();
  }, [recarregarStatus]);

  // O intervalo de nova tentativa vem do próprio GitHub; a interface apenas
  // repete a pergunta até a autorização sair (ou expirar).
  useEffect(() => {
    if (flow === null) return;
    const esperados = Math.max(1, flow.interval) * 1000;
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const r = await api.githubConnectPoll();
          if (r.status === "connected") {
            setFlow(null);
            setConnectError(null);
            await recarregarStatus();
            pushToast("info", `Conta GitHub conectada como ${r.login ?? "desconhecida"}.`);
          } else if (r.status === "error") {
            setFlow(null);
            setConnectError(r.message ?? "a autorização não foi concluída");
          }
        } catch (e) {
          setFlow(null);
          setConnectError(String(e));
        }
      })();
    }, esperados);
    return () => window.clearInterval(timer);
  }, [flow, recarregarStatus, pushToast]);

  const lerRepos = useCallback(async () => {
    setReposLoading(true);
    setReposError(null);
    try {
      setRepos(await api.githubRepos());
    } catch (e) {
      setRepos(null);
      setReposError(String(e));
    } finally {
      setReposLoading(false);
    }
  }, []);

  const conectado = status?.loggedIn === true;

  useEffect(() => {
    if (conectado && repos === null && !reposLoading) void lerRepos();
  }, [conectado, repos, reposLoading, lerRepos]);

  const salvarClientId = async () => {
    setSavingId(true);
    try {
      setStatus(await api.githubSetClientId(clientId.trim()));
      setClientId("");
      pushToast("info", "Client ID salvo. Ele não é um segredo: é o identificador público da aplicação.");
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setSavingId(false);
    }
  };

  const iniciarConexao = async () => {
    setStarting(true);
    setConnectError(null);
    try {
      setFlow(await api.githubConnectStart());
    } catch (e) {
      setConnectError(String(e));
    } finally {
      setStarting(false);
    }
  };

  const desconectar = async () => {
    try {
      setStatus(await api.githubDisconnect());
      setRepos(null);
      setPulls({});
      pushToast("info", "Conta GitHub desconectada. O token saiu da memória desta execução.");
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const abrirPulls = async (fullName: string) => {
    if (pulls[fullName]?.items) {
      setPulls((atual) => {
        const copia = { ...atual };
        delete copia[fullName];
        return copia;
      });
      return;
    }
    setPulls((atual) => ({ ...atual, [fullName]: { loading: true, items: null, error: null } }));
    try {
      const items = await api.githubPulls(fullName);
      setPulls((atual) => ({ ...atual, [fullName]: { loading: false, items, error: null } }));
    } catch (e) {
      setPulls((atual) => ({ ...atual, [fullName]: { loading: false, items: null, error: String(e) } }));
    }
  };

  const criarRepositorio = async () => {
    setCriando(true);
    try {
      const criado = await api.githubCreateRepo({
        name: novoNome.trim(),
        private: novoPrivado,
        description: novoDescricao.trim() || null,
      });
      setRepos((atual) => (atual ? [criado, ...atual] : [criado]));
      setConfirmando(false);
      setNovoNome("");
      setNovoDescricao("");
      setRepoAlvo(criado.fullName);
      pushToast("info", `${criado.fullName} criado (${criado.private ? "privado" : "público"}).`);
    } catch (e) {
      setConfirmando(false);
      pushToast("error", String(e));
    } finally {
      setCriando(false);
    }
  };

  // Remoto já existente decide se a operação adiciona ou substitui — sempre
  // depois de confirmação explícita na tela.
  useEffect(() => {
    if (projetoId === "") {
      setRemotosDoProjeto(null);
      return;
    }
    let vivo = true;
    api
      .gitRemotes(projetoId)
      .then((r) => {
        if (vivo) setRemotosDoProjeto(r);
      })
      .catch(() => {
        if (vivo) setRemotosDoProjeto([]);
      });
    return () => {
      vivo = false;
    };
  }, [projetoId]);

  const repoEscolhido = useMemo(
    () => (repos ?? []).find((r) => r.fullName === repoAlvo) ?? null,
    [repos, repoAlvo],
  );

  const originAtual = (remotosDoProjeto ?? []).find((r) => r.name === "origin") ?? null;

  const vincular = async (substituir: boolean) => {
    if (projetoId === "" || repoEscolhido === null) return;
    setVinculando(true);
    setConfirmarVinculo(null);
    try {
      const url = repoEscolhido.cloneUrl;
      const novos = substituir
        ? await api.gitSetRemoteUrl(projetoId, "origin", url)
        : await api.gitAddRemote(projetoId, "origin", url);
      setRemotosDoProjeto(novos);
      pushToast(
        "info",
        `origin de ${repoEscolhido.fullName} vinculado ao projeto. Enviar commits continua sendo ação sua, na seção Git.`,
      );
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setVinculando(false);
    }
  };

  return (
    <div className="max-w-4xl space-y-4">
      <Card className="p-5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SectionTitle>Conta GitHub</SectionTitle>
          <Button size="sm" onClick={() => void recarregarStatus()}>
            <RefreshIcon className="size-3.5" /> Atualizar estado
          </Button>
        </div>
        {status === null ? (
          <p className="flex items-center gap-2 text-sm text-zinc-500">
            <Spinner className="size-3.5" /> Lendo o estado local…
          </p>
        ) : conectado ? (
          <div className="flex flex-wrap items-center gap-3">
            <Badge tone="success">conectado</Badge>
            <span className="text-sm">
              login <span className="font-mono">{status.login ?? "—"}</span>
            </span>
            <Button size="sm" variant="ghost" onClick={() => void desconectar()}>
              Desconectar
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <p className="text-sm text-zinc-600 dark:text-zinc-300">
              A conexão usa o fluxo de dispositivo do OAuth: o aplicativo pede um código, você digita esse código no
              github.com e o token resultante fica <span className="font-medium">somente na memória</span> desta
              execução — nada de token no banco de dados, nos logs ou no disco.
            </p>
            {flow === null ? (
              <Button variant="primary" loading={starting} disabled={!status.hasClientId} onClick={() => void iniciarConexao()}>
                <KeyIcon className="size-4" /> Conectar conta
              </Button>
            ) : (
              <div className="rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
                <p className="text-xs uppercase tracking-wider text-zinc-500">Código de verificação</p>
                <p className="mt-1 font-mono text-2xl font-semibold tracking-widest">{flow.userCode}</p>
                <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
                  Digite este código em {flow.verificationUri} depois de autorizar no navegador. Expira em{" "}
                  {Math.round(flow.expiresIn / 60)} min.
                </p>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    onClick={() =>
                      void openUrl(flow.verificationUri).catch(() => pushToast("info", `Abra manualmente em ${flow.verificationUri}`))
                    }
                  >
                    <ExternalLinkIcon className="size-3.5" /> Abrir a página de autorização
                  </Button>
                  <span className="flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
                    <Spinner className="size-3" /> aguardando você autorizar
                  </span>
                  <Button size="sm" variant="ghost" onClick={() => setFlow(null)}>
                    Cancelar aqui
                  </Button>
                </div>
              </div>
            )}
            {connectError ? (
              <p className="flex items-start gap-2 rounded-lg bg-red-500/10 p-2.5 text-xs text-red-600 dark:text-red-400">
                <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {connectError}
              </p>
            ) : null}
            {status.hasClientId ? null : (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                Conectar exige um Client ID (próximo card). Sem ele o botão fica desabilitado de propósito: nada é
                fingido na interface.
              </p>
            )}
          </div>
        )}
      </Card>

      <Card className="p-5">
        <SectionTitle>Client ID da aplicação OAuth</SectionTitle>
        <Field
          label={status?.hasClientId ? "Novo Client ID (substitui o salvo)" : "Client ID"}
          hint="Crie uma OAuth App em github.com/settings/developers, marque “Enable Device Flow” e cole o Identificador. Não cole token de acesso aqui — o campo aceita apenas o identificador público."
        >
          <div className="flex flex-wrap items-center gap-2">
            <Input
              className="max-w-xs font-mono"
              placeholder="Iv1.b50e9c5a61f47c2b"
              value={clientId}
              spellCheck={false}
              onChange={(e) => setClientId(e.target.value)}
            />
            <Button
              size="sm"
              variant="primary"
              loading={savingId}
              disabled={clientId.trim().length === 0}
              onClick={() => void salvarClientId()}
            >
              Salvar
            </Button>
            <Button size="sm" variant="ghost" onClick={() => void openUrl(DEVELOPERS_URL).catch(() => pushToast("info", `Abra manualmente em ${DEVELOPERS_URL}`))}>
              <ExternalLinkIcon className="size-3.5" /> Criar OAuth App
            </Button>
          </div>
        </Field>
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          {status?.hasClientId
            ? "Há um Client ID salvo. Por segurança o valor completo não volta para a interface; para trocar, cole o novo."
            : "Nenhum Client ID salvo."}
        </p>
      </Card>

      <Card className="p-5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <SectionTitle>Repositórios da conta</SectionTitle>
          <Button size="sm" loading={reposLoading} disabled={!conectado} onClick={() => void lerRepos()}>
            <RefreshIcon className="size-3.5" /> Ler do GitHub
          </Button>
        </div>
        {!conectado ? (
          <EmptyState
            icon={<CloudIcon className="size-8" />}
            title="Nenhum repositório lido"
            description="Conecte a conta acima para listar os repositórios reais desta conta."
          />
        ) : reposError ? (
          <p className="flex items-start gap-2 rounded-lg bg-red-500/10 p-2.5 text-xs text-red-600 dark:text-red-400">
            <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {reposError}
          </p>
        ) : repos === null ? (
          <p className="flex items-center gap-2 text-sm text-zinc-500">
            <Spinner className="size-3.5" /> Consultando api.github.com…
          </p>
        ) : repos.length === 0 ? (
          <EmptyState
            icon={<CloudIcon className="size-8" />}
            title="A conta respondeu com zero repositórios"
            description="Crie um abaixo ou verifique o escopo da autorização."
          />
        ) : (
          <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
            {repos.map((r) => {
              const p = pulls[r.fullName];
              return (
                <li key={r.fullName} className="py-2.5">
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <span className="font-mono text-sm">{r.fullName}</span>
                    <Badge tone={r.private ? "warning" : "accent"}>{r.private ? "privado" : "público"}</Badge>
                    {r.archived ? <Badge tone="neutral">arquivado</Badge> : null}
                    {r.fork ? <Badge tone="neutral">fork</Badge> : null}
                    <span className="text-xs text-zinc-500 dark:text-zinc-400">
                      {r.defaultBranch || "sem branch"} · {r.language ?? "sem linguagem"} · {formatKilobytes(r.sizeKb)}
                    </span>
                    <span className="ml-auto flex flex-wrap items-center gap-1.5">
                      <span className="text-[10px] text-zinc-400 dark:text-zinc-500">
                        {r.pushedAt ? `push ${formatDateTime(r.pushedAt)}` : "sem push registrado"}
                      </span>
                      <Button size="sm" variant="ghost" onClick={() => void abrirPulls(r.fullName)}>
                        {p?.items ? "Fechar PRs" : "Pull requests"}
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => void openUrl(r.htmlUrl).catch(() => pushToast("info", r.htmlUrl))}
                      >
                        <ExternalLinkIcon className="size-3.5" /> Abrir
                      </Button>
                    </span>
                  </div>
                  {p ? (
                    <div className="mt-2 rounded-lg bg-zinc-100/70 p-2.5 text-xs dark:bg-zinc-800/60">
                      {p.loading ? (
                        <span className="flex items-center gap-2 text-zinc-500">
                          <Spinner className="size-3" /> lendo pull requests abertos…
                        </span>
                      ) : p.error ? (
                        <span className="text-red-600 dark:text-red-400">{p.error}</span>
                      ) : p.items && p.items.length > 0 ? (
                        <ul className="space-y-1.5">
                          {p.items.map((pr) => (
                            <PRRow key={pr.number} pr={pr} />
                          ))}
                        </ul>
                      ) : (
                        <span className="text-zinc-500">Nenhum pull request aberto neste repositório.</span>
                      )}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </Card>

      <Card className="p-5">
        <SectionTitle>Vincular um projeto local a um repositório</SectionTitle>
        <div className="flex flex-wrap items-end gap-3">
          <Field label="Projeto cadastrado">
            <Select
              className="min-w-44"
              value={projetoId === "" ? "" : String(projetoId)}
              onChange={(e) => {
                const v = e.target.value;
                setProjetoId(v === "" ? "" : Number(v));
              }}
            >
              <option value="">selecione…</option>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Repositório do GitHub">
            <Select
              className="min-w-52"
              value={repoAlvo}
              onChange={(e) => setRepoAlvo(e.target.value)}
              disabled={repos === null || repos.length === 0}
            >
              <option value="">selecione…</option>
              {repos?.map((r) => (
                <option key={r.fullName} value={r.fullName}>
                  {r.fullName}
                </option>
              ))}
            </Select>
          </Field>
          <Button
            variant="primary"
            disabled={projetoId === "" || repoEscolhido === null || vinculando}
            loading={vinculando}
            onClick={() => setConfirmarVinculo({ substituir: originAtual !== null })}
          >
            Vincular como origin
          </Button>
        </div>
        {projetoId !== "" ? (
          <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
            {originAtual
              ? `Este projeto já tem origin (${originAtual.fetchUrl ?? "sem URL de busca"}). Vincular substitui a URL.`
              : "Este projeto ainda não tem um remoto chamado origin."}
          </p>
        ) : null}
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          A ação grava apenas a configuração local <span className="font-mono">remote.origin.url</span>. Nenhum commit,
          push ou criação de branch é feito por conta própria.
        </p>
      </Card>

      <Card className="p-5">
        <SectionTitle>Criar repositório novo</SectionTitle>
        <div className="space-y-3">
          <Field label="Nome">
            <Input
              className="max-w-sm font-mono"
              placeholder="devops-desktop"
              value={novoNome}
              spellCheck={false}
              onChange={(e) => setNovoNome(e.target.value)}
            />
          </Field>
          <Field label="Descrição (opcional)">
            <Textarea
              className="max-w-xl"
              rows={2}
              value={novoDescricao}
              onChange={(e) => setNovoDescricao(e.target.value)}
            />
          </Field>
          <Checkbox
            checked={novoPrivado}
            onChange={setNovoPrivado}
            label="Privado (desmarcado, o repositório nasce público e qualquer pessoa lê o código)"
          />
          <Button
            variant="primary"
            disabled={!conectado || novoNome.trim().length === 0}
            onClick={() => setConfirmando(true)}
          >
            Criar no GitHub
          </Button>
        </div>
      </Card>

      <p className="px-1 text-[11px] leading-relaxed text-zinc-500 dark:text-zinc-400">
        Toda a lista acima vem de uma chamada autenticada a api.github.com feita agora. Nada é pré-carregado,
        cacheado de outra conta ou inventado: se o GitHub não respondeu, a tela mostra o erro da resposta em vez de
        um valor de exemplo. O token fica na memória do processo e é descartado ao fechar o aplicativo ou ao
        desconectar.
      </p>

      <Confirm
        open={confirmando}
        busy={criando}
        title="Criar repositório no GitHub"
        message={
          `Vai criar ${novoNome.trim()} como repositório ${novoPrivado ? "privado" : "público"} na conta conectada. ` +
          "Isso é visível para outras pessoas se for público e não é desfeito sozinho."
        }
        confirmLabel="Criar"
        onConfirm={() => void criarRepositorio()}
        onClose={() => setConfirmando(false)}
      />

      <Confirm
        open={confirmarVinculo !== null}
        busy={vinculando}
        title={confirmarVinculo?.substituir ? "Substituir a URL do origin" : "Adicionar o remoto origin"}
        message={
          confirmarVinculo?.substituir
            ? `A URL do origin do projeto selecionado passa a ser ${repoEscolhido?.cloneUrl ?? ""}.`
            : `Será criado o remoto origin apontando para ${repoEscolhido?.cloneUrl ?? ""}.`
        }
        confirmLabel={confirmarVinculo?.substituir ? "Substituir" : "Adicionar"}
        onConfirm={() => void vincular(confirmarVinculo?.substituir === true)}
        onClose={() => setConfirmarVinculo(null)}
      />
    </div>
  );
}

function PRRow({ pr }: { pr: GitHubPull }) {
  return (
    <li className="flex flex-wrap items-center gap-2">
      <Badge tone={pr.draft ? "neutral" : "info"}>#{pr.number}</Badge>
      <span className="text-zinc-700 dark:text-zinc-200">{pr.title}</span>
      <span className="text-zinc-500">
        {pr.headRef} → {pr.baseRef}
      </span>
      {pr.author ? <span className="text-zinc-400">@{pr.author}</span> : null}
      {pr.updatedAt ? <span className="text-zinc-400">{formatDateTime(pr.updatedAt)}</span> : null}
      <Button
        size="sm"
        variant="ghost"
        className="ml-auto"
        onClick={() => void openUrl(pr.htmlUrl).catch(() => undefined)}
      >
        <ExternalLinkIcon className="size-3" /> abrir
      </Button>
    </li>
  );
}
