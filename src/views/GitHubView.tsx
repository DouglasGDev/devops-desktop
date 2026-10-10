import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../lib/api";
import { chaveDoRemoto } from "../lib/repo";
import { formatAgo } from "../lib/format";
import type {
  GitHubDeviceFlow,
  GitHubInbox,
  GitHubRepo,
  GitHubSearchItem,
  GitHubStatus,
  Project,
  ViewId,
} from "../types";
import { useApp } from "../store/AppContext";
import { useMapearPasta } from "../lib/useMapearPasta";
import { GitView } from "./GitView";
import { BandejaPanel } from "../components/github/BandejaPanel";
import { ChecksPanel } from "../components/github/ChecksPanel";
import { IssuePanel } from "../components/github/IssuePanel";
import { PullRequestPanel } from "../components/github/PullRequestPanel";
import { RepoOverview } from "../components/github/RepoOverview";
import {
  AlertTriangleIcon,
  ChecksIcon,
  CloudIcon,
  ExternalLinkIcon,
  FolderIcon,
  GitBranchIcon,
  InboxIcon,
  IssueIcon,
  KeyIcon,
  PlusIcon,
  RefreshIcon,
  SearchIcon,
  SettingsIcon,
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
  Modal,
  SectionTitle,
  Spinner,
  Textarea,
} from "../components/ui";

const DEVELOPERS_URL = "https://github.com/settings/developers";

type Aba = "visao" | "pulls" | "issues" | "acoes" | "local";

const ABAS: { id: Aba; nome: string; icone: typeof GitBranchIcon }[] = [
  { id: "visao", nome: "Visão geral", icone: CloudIcon },
  { id: "pulls", nome: "Pull requests", icone: GitBranchIcon },
  { id: "issues", nome: "Issues", icone: IssueIcon },
  { id: "acoes", nome: "Verificações", icone: ChecksIcon },
  { id: "local", nome: "Local", icone: FolderIcon },
];

export function GitHubView({ onNavigate }: { onNavigate: (v: ViewId) => void }) {
  const { projects, pushToast } = useApp();
  const { mapear, mapeando } = useMapearPasta();

  const [status, setStatus] = useState<GitHubStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [savingId, setSavingId] = useState(false);
  const [mostrarConfig, setMostrarConfig] = useState(false);

  const [flow, setFlow] = useState<GitHubDeviceFlow | null>(null);
  const [starting, setStarting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);

  const [repos, setRepos] = useState<GitHubRepo[] | null>(null);
  const [reposLoading, setReposLoading] = useState(false);
  const [reposError, setReposError] = useState<string | null>(null);
  const [filtro, setFiltro] = useState("");

  const [inbox, setInbox] = useState<GitHubInbox | null>(null);
  const [inboxLoading, setInboxLoading] = useState(false);
  const [inboxErro, setInboxErro] = useState<string | null>(null);

  const [repoSel, setRepoSel] = useState("");
  const [aba, setAba] = useState<Aba>("visao");
  const [prAlvo, setPrAlvo] = useState<number | null>(null);
  const [issueAlvo, setIssueAlvo] = useState<number | null>(null);

  const [novoAberto, setNovoAberto] = useState(false);
  const [novoNome, setNovoNome] = useState("");
  const [novoDescricao, setNovoDescricao] = useState("");
  const [novoPrivado, setNovoPrivado] = useState(true);
  const [confirmando, setConfirmando] = useState(false);
  const [criando, setCriando] = useState(false);

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
            pushToast(
              r.login ? "info" : "error",
              r.login
                ? `Conta GitHub conectada como ${r.login}.`
                : `O GitHub aceitou a autorização, mas o login não foi lido: ${r.message ?? "resposta sem o login"}`,
            );
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

  const lerBandeja = useCallback(async () => {
    setInboxLoading(true);
    setInboxErro(null);
    try {
      setInbox(await api.githubInbox());
    } catch (e) {
      setInbox(null);
      setInboxErro(String(e));
    } finally {
      setInboxLoading(false);
    }
  }, []);

  const conectado = status?.loggedIn === true;

  // A primeira leitura de cada sessão conectada; depois disso só botão. Assim um
  // erro não vira nova tentativa atrás da outra contra a API.
  const lidoPara = useRef<string | null>(null);

  useEffect(() => {
    if (!conectado) {
      lidoPara.current = null;
      return;
    }
    const chave = status?.login ?? "sem-login";
    if (lidoPara.current === chave) return;
    lidoPara.current = chave;
    void lerRepos();
    void lerBandeja();
  }, [conectado, status, lerRepos, lerBandeja]);

  const salvarClientId = async () => {
    setSavingId(true);
    try {
      setStatus(await api.githubSetClientId(clientId.trim()));
      setClientId("");
      setMostrarConfig(false);
      pushToast("info", "Client ID salvo.");
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
      setInbox(null);
      setRepoSel("");
      pushToast("info", "Conta desconectada. O token saiu da memória desta execução.");
    } catch (e) {
      pushToast("error", String(e));
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
      setNovoAberto(false);
      setNovoNome("");
      setNovoDescricao("");
      escolherRepo(criado.fullName);
      pushToast("info", `${criado.fullName} criado (${criado.private ? "privado" : "público"}).`);
    } catch (e) {
      setConfirmando(false);
      pushToast("error", String(e));
    } finally {
      setCriando(false);
    }
  };

  function escolherRepo(slug: string) {
    setRepoSel(slug);
    setAba("visao");
    setPrAlvo(null);
    setIssueAlvo(null);
  }

  // A bandeja é o atalho: um item leva direto ao repo e à aba certa.
  const abrirItem = (item: GitHubSearchItem) => {
    setRepoSel(item.repo);
    setPrAlvo(null);
    setIssueAlvo(null);
    if (item.isPullRequest) {
      setAba("pulls");
      setPrAlvo(item.number);
    } else {
      setAba("issues");
      setIssueAlvo(item.number);
    }
  };

  const repoObj = useMemo(
    () => (repos ?? []).find((r) => r.fullName === repoSel) ?? null,
    [repos, repoSel],
  );

  // Qual repositório a aplicação já enxerga nesta máquina: uma pasta cadastrada
  // cujo origin resolve para ele.
  const pastaPorRepo = useMemo(() => {
    const mapa = new Map<string, Project>();
    for (const p of projects) {
      const chave = chaveDoRemoto(p.gitRemote);
      if (chave !== null && !mapa.has(chave)) mapa.set(chave, p);
    }
    return mapa;
  }, [projects]);

  // A pasta desta máquina que é o clone do repositório escolhido na sidebar.
  const pastaDesteRepo = useMemo(
    () => (repoSel === "" ? null : pastaPorRepo.get(repoSel.toLowerCase()) ?? null),
    [pastaPorRepo, repoSel],
  );

  const reposVisiveis = useMemo(() => {
    const todos = repos ?? [];
    const termo = filtro.trim().toLowerCase();
    if (termo === "") return todos;
    return todos.filter((r) => r.fullName.toLowerCase().includes(termo));
  }, [repos, filtro]);

  const pendentes = (inbox?.reviewRequests.length ?? 0) + (inbox?.myPulls.length ?? 0) + (inbox?.myIssues.length ?? 0);

  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <SectionTitle>Conta GitHub</SectionTitle>
            {status === null ? (
              <Spinner className="size-3.5" />
            ) : conectado ? (
              <>
                <Badge tone="success">conectado</Badge>
                <span className="font-mono text-sm">{status.login ?? "—"}</span>
                {pendentes > 0 ? <Badge tone="accent">{pendentes} na bandeja</Badge> : null}
              </>
            ) : (
              <Badge tone={status.hasClientId ? "info" : "warning"}>
                {status.hasClientId ? "sem conexão" : "falta Client ID"}
              </Badge>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => setMostrarConfig((v) => !v)}>
              <SettingsIcon className="size-3.5" /> Configuração
            </Button>
            <Button size="sm" onClick={() => void recarregarStatus()}>
              <RefreshIcon className="size-3.5" /> Atualizar estado
            </Button>
            {conectado ? (
              <Button size="sm" variant="ghost" onClick={() => void desconectar()}>
                Desconectar
              </Button>
            ) : null}
          </div>
        </div>

        {conectado ? null : (
          <div className="mt-3 space-y-3">
            {flow === null ? (
              <Button variant="primary" loading={starting} disabled={!status?.hasClientId} onClick={() => void iniciarConexao()}>
                <KeyIcon className="size-4" /> Conectar conta
              </Button>
            ) : (
              <div className="rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
                <p className="text-xs uppercase tracking-wider text-zinc-500">Código de verificação</p>
                <p className="mt-1 font-mono text-2xl font-semibold tracking-widest">{flow.userCode}</p>
                <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
                  Expira em {Math.round(flow.expiresIn / 60)} min · o token fica só na memória desta execução.
                </p>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    onClick={() =>
                      void openUrl(flow.verificationUri).catch(() =>
                        pushToast("info", `Abra manualmente em ${flow.verificationUri}`),
                      )
                    }
                  >
                    <ExternalLinkIcon className="size-3.5" /> Autorizar no navegador
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
          </div>
        )}

        {mostrarConfig ? (
          <div className="mt-3 rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
            <Field
              label={status?.hasClientId ? "Novo Client ID (substitui o salvo)" : "Client ID da OAuth App"}
              hint="Os atuais começam com “Ov”; os antigos são hexadecimais; um GitHub App começa com “Iv1.”."
            >
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  className="max-w-xs font-mono"
                  placeholder="campo Client ID"
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
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void openUrl(DEVELOPERS_URL).catch(() => pushToast("info", `Abra manualmente em ${DEVELOPERS_URL}`))
                  }
                >
                  <ExternalLinkIcon className="size-3.5" /> github.com/settings/developers
                </Button>
              </div>
            </Field>
            <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
              {status?.hasClientId
                ? "Há um Client ID salvo; o valor completo não volta para a interface."
                : "Nenhum Client ID salvo."}
            </p>
          </div>
        ) : null}
      </Card>

      {conectado ? (
        <div className="grid min-h-0 gap-4 lg:grid-cols-[minmax(0,15rem)_minmax(0,1fr)]">
          <Card className="flex max-h-[38rem] flex-col gap-2 overflow-y-auto p-2">
            <button
              type="button"
              onClick={() => setRepoSel("")}
              className={`flex items-center justify-between rounded-lg border px-2.5 py-2 text-left transition ${
                repoSel === ""
                  ? "border-teal-500/70 bg-teal-500/5"
                  : "border-zinc-200 hover:border-zinc-300 dark:border-zinc-800 dark:hover:border-zinc-700"
              }`}
            >
              <span className="flex items-center gap-2 text-sm">
                <InboxIcon className="size-4 text-teal-500" /> Bandeja
              </span>
              <Badge tone={pendentes > 0 ? "accent" : "neutral"}>{pendentes}</Badge>
            </button>

            <div className="flex items-center gap-1.5">
              <div className="relative min-w-0 flex-1">
                <SearchIcon className="pointer-events-none absolute left-2 top-2 size-3.5 text-zinc-400" />
                <Input
                  className="pl-8 text-xs"
                  placeholder="filtrar repositórios"
                  value={filtro}
                  onChange={(e) => setFiltro(e.target.value)}
                />
              </div>
              <Button size="sm" variant="ghost" loading={reposLoading} onClick={() => void lerRepos()}>
                <CloudIcon className="size-3.5" />
              </Button>
            </div>

            {reposError ? (
              <p className="px-1 text-[11px] text-red-600 dark:text-red-400">{reposError}</p>
            ) : repos === null ? (
              <p className="flex items-center gap-2 px-1 text-xs text-zinc-500 dark:text-zinc-400">
                <Spinner className="size-3" /> lendo a conta…
              </p>
            ) : (
              <ul className="flex flex-col gap-1">
                {reposVisiveis.map((r) => (
                  <li key={r.fullName}>
                    <button
                      type="button"
                      onClick={() => escolherRepo(r.fullName)}
                      className={`w-full rounded-lg border px-2.5 py-1.5 text-left transition ${
                        repoSel === r.fullName
                          ? "border-teal-500/70 bg-teal-500/5"
                          : "border-zinc-200 hover:border-zinc-300 dark:border-zinc-800 dark:hover:border-zinc-700"
                      }`}
                    >
                      <p className="truncate font-mono text-xs">{r.name}</p>
                      <p className="mt-0.5 flex items-center gap-1.5 text-[10px] text-zinc-500 dark:text-zinc-400">
                        <span>{r.private ? "privado" : "público"}</span>
                        <span>{r.pushedAt ? formatAgo(r.pushedAt) : "sem push"}</span>
                        {pastaPorRepo.get(r.fullName.toLowerCase()) ? (
                          <span className="ml-auto flex items-center gap-1 text-teal-600 dark:text-teal-400">
                            <FolderIcon className="size-3" /> mapeado
                          </span>
                        ) : null}
                      </p>
                    </button>
                  </li>
                ))}
                {reposVisiveis.length === 0 ? (
                  <li className="px-1 text-[11px] text-zinc-500 dark:text-zinc-400">
                    {repos.length === 0 ? "A conta respondeu zero repositórios." : "Nada corresponde ao filtro."}
                  </li>
                ) : null}
              </ul>
            )}

            <Button size="sm" variant="secondary" onClick={() => setNovoAberto(true)}>
              <PlusIcon className="size-3.5" /> Novo repositório
            </Button>
          </Card>

          <div className="min-w-0 space-y-3">
            {repoSel === "" ? (
              <BandejaPanel
                inbox={inbox}
                loading={inboxLoading}
                error={inboxErro}
                onAbrir={abrirItem}
                onTentarDeNovo={() => void lerBandeja()}
              />
            ) : (
              <>
                <Card className="p-3">
                  <div className="flex flex-wrap items-center gap-1.5">
                    {ABAS.filter((a) => !(repoObj === null && a.id === "visao")).map((a) => {
                      const Icone = a.icone;
                      return (
                        <button
                          key={a.id}
                          type="button"
                          onClick={() => {
                            setAba(a.id);
                            if (a.id !== "pulls") setPrAlvo(null);
                            if (a.id !== "issues") setIssueAlvo(null);
                          }}
                          className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition ${
                            aba === a.id
                              ? "bg-teal-500/10 text-teal-600 dark:text-teal-400"
                              : "text-zinc-500 hover:bg-zinc-500/10 dark:text-zinc-400"
                          }`}
                        >
                          <Icone className="size-3.5" /> {a.nome}
                        </button>
                      );
                    })}
                    <span className="ml-auto flex items-center gap-2">
                      <span className="font-mono text-[11px] text-zinc-500 dark:text-zinc-400">{repoSel}</span>
                      <Button size="sm" variant="ghost" onClick={() => void lerBandeja()}>
                        <RefreshIcon className="size-3.5" />
                      </Button>
                    </span>
                  </div>
                  {repoObj === null ? (
                    <p className="mt-2 text-[11px] text-zinc-500 dark:text-zinc-400">
                      Este repositório não está na lista da sua conta — as abas abaixo leem direto do slug{" "}
                      <span className="font-mono">{repoSel}</span>, então o que aparece é o que o GitHub respondeu.
                    </p>
                  ) : null}
                </Card>

                {aba === "visao" && repoObj !== null ? <RepoOverview repo={repoObj} onNavigate={onNavigate} /> : null}
                {aba === "pulls" ? (
                  <PullRequestPanel
                    repo={repoSel}
                    numeroInicial={prAlvo}
                    branchPadrao={repoObj?.defaultBranch ?? null}
                    onNavigate={onNavigate}
                  />
                ) : null}
                {aba === "issues" ? <IssuePanel repo={repoSel} numeroInicial={issueAlvo} onNavigate={onNavigate} /> : null}
                {aba === "acoes" ? <ChecksPanel repo={repoSel} /> : null}
                {aba === "local" ? (
                  pastaDesteRepo === null ? (
                    <Card className="p-5">
                      <EmptyState
                        icon={<FolderIcon className="size-8" />}
                        title="Nenhuma pasta desta máquina para este repositório"
                        description={
                          mapeando ? "Escolhendo a pasta…" : "Este repositório ainda não tem uma pasta mapeada nesta máquina."
                        }
                      >
                        <Button variant="primary" disabled={mapeando} onClick={() => void mapear(repoSel)}>
                          <FolderIcon className="size-4" /> Mapear pasta local
                        </Button>
                        <Button disabled={mapeando} onClick={() => onNavigate("git")}>
                          Abrir na seção Git
                        </Button>
                      </EmptyState>
                    </Card>
                  ) : (
                    <div className="h-[46rem]">
                      <GitView onNavigate={onNavigate} projetoFixo={pastaDesteRepo.id} />
                    </div>
                  )
                ) : null}
              </>
            )}
          </div>
        </div>
      ) : (
        <Card className="p-5">
          <EmptyState
            icon={<CloudIcon className="size-8" />}
            title="Nenhum repositório lido"
            description="Conecte a conta acima para abrir a bandeja, os pull requests, as issues e as verificações."
          />
        </Card>
      )}

      <Modal open={novoAberto} onClose={() => setNovoAberto(false)} title="Criar repositório no GitHub">
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
            <Textarea rows={2} value={novoDescricao} onChange={(e) => setNovoDescricao(e.target.value)} />
          </Field>
          <Checkbox
            checked={novoPrivado}
            onChange={setNovoPrivado}
            label="Privado (desmarcado, o repositório nasce público e qualquer pessoa lê o código)"
          />
          <div className="flex justify-end gap-2">
            <Button onClick={() => setNovoAberto(false)}>Cancelar</Button>
            <Button
              variant="primary"
              disabled={novoNome.trim().length === 0}
              onClick={() => {
                setNovoAberto(false);
                setConfirmando(true);
              }}
            >
              Criar
            </Button>
          </div>
        </div>
      </Modal>

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
    </div>
  );
}
