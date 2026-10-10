import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../lib/api";
import type { GitHubDeviceFlow, GitHubStatus, ToolVersions } from "../types";
import { useApp } from "../store/AppContext";
import { AlertTriangleIcon, CheckIcon, ExternalLinkIcon, RefreshIcon, XIcon } from "./Icons";
import { Badge, Button, Card, SectionTitle, Spinner } from "./ui";

type Props = {
  tools: ToolVersions | null;
  loading: boolean;
  onRescan: () => void;
};

/**
 * Checklist real da máquina: o que está instalado, o que já tem credencial e o
 * que falta para publicar. Cada linha vem de um comando executado agora
 * (`git --version`, `ssh -V`, `ssh-add -l`, `gh auth status`) ou do estado do
 * fluxo de dispositivo do GitHub — nada aqui é suposição. Toda sondagem tem
 * prazo: uma ferramenta travada é nomeada na tela em vez de deixar o card
 * girando para sempre.
 */
export function MachineAccess({ tools, loading, onRescan }: Props) {
  const { pushToast } = useApp();
  const [status, setStatus] = useState<GitHubStatus | null>(null);
  const [flow, setFlow] = useState<GitHubDeviceFlow | null>(null);
  const [starting, setStarting] = useState(false);
  const [erro, setErro] = useState<string | null>(null);
  const [statusErro, setStatusErro] = useState<string | null>(null);

  const recarregarStatus = useCallback(async () => {
    try {
      setStatus(await api.githubStatus());
      setStatusErro(null);
    } catch (e) {
      // O login por aqui é um dos caminhos de liberação; os outros continuam
      // valendo, então o card segue funcionando e apenas avisa da leitura falha.
      setStatusErro(String(e));
      pushToast("error", `Não foi possível ler o estado do GitHub: ${String(e)}`);
    }
  }, [pushToast]);

  useEffect(() => {
    void recarregarStatus();
  }, [recarregarStatus]);

  // A cadência vem do próprio GitHub, igual à aba GitHub.
  useEffect(() => {
    if (flow === null) return;
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const r = await api.githubConnectPoll();
          if (r.status === "connected") {
            setFlow(null);
            setErro(null);
            await recarregarStatus();
            pushToast("success", `Máquina liberada: conta ${r.login ?? "desconhecida"} autorizada.`);
          } else if (r.status === "error") {
            setFlow(null);
            setErro(r.message ?? "a autorização não foi concluída");
          }
        } catch (e) {
          setFlow(null);
          setErro(String(e));
        }
      })();
    }, Math.max(1, flow.interval) * 1000);
    return () => window.clearInterval(timer);
  }, [flow, recarregarStatus, pushToast]);

  const liberar = async () => {
    setStarting(true);
    setErro(null);
    try {
      setFlow(await api.githubConnectStart());
    } catch (e) {
      setErro(String(e));
    } finally {
      setStarting(false);
    }
  };

  const gitOk = tools?.git != null;
  const sshOk = tools?.ssh != null;
  const ghOk = tools?.gh != null;
  const temChaveNoAgente = tools?.agentRunning === true && (tools.agentKeys ?? 0) > 0;
  const contaGh = tools?.ghAccount != null;
  const contaApp = status?.loggedIn === true;
  const liberado = contaApp || contaGh || temChaveNoAgente;

  // Uma sonda que não respondeu é diferente de ferramenta ausente: o executável
  // existe, só não devolveu nada dentro do prazo.
  const semResposta = (programa: string) => tools?.semResposta.includes(programa) === true;
  const lendoGitHub = status === null && statusErro === null;
  const incerto = lendoGitHub && !contaGh && !temChaveNoAgente;

  const faltas: string[] = [];
  if (tools !== null) {
    if (!gitOk) faltas.push(semResposta("git") ? "o git responder à sondagem" : "Git");
    if (!sshOk) faltas.push(semResposta("ssh") ? "o ssh responder à sondagem" : "OpenSSH");
    if (gitOk && tools.gitUser == null) faltas.push("identidade do git");
    if (!liberado && !incerto) faltas.push("uma liberação de acesso");
    if (statusErro !== null) faltas.push("a leitura do estado do GitHub");
  }

  const caminho = contaApp
    ? "login do GitHub nesta aplicação"
    : contaGh
      ? `gh CLI (conta ${tools?.ghAccount ?? ""})`
      : "chave no ssh-agent";

  return (
    <Card className="p-5">
      <div className="flex items-center justify-between">
        <SectionTitle>Acesso da máquina</SectionTitle>
        <Button size="sm" loading={loading} onClick={onRescan}>
          <RefreshIcon className="size-3.5" /> Reverificar
        </Button>
      </div>

      {tools === null ? (
        <p className="mt-3 flex items-center gap-2 text-sm text-zinc-500">
          <Spinner className="size-3.5" /> Verificando as ferramentas desta máquina…
        </p>
      ) : incerto ? (
        <p className="mt-3 flex items-center gap-2 rounded-lg bg-zinc-500/10 p-3 text-sm text-zinc-600 dark:text-zinc-300">
          <Spinner className="size-3.5" />
          <span>
            O que está instalado já foi lido. Falta só o estado do GitHub para dizer se o acesso está liberado — as
            linhas abaixo valem do mesmo jeito.
          </span>
        </p>
      ) : faltas.length === 0 ? (
        <div className="mt-3 flex items-start gap-2 rounded-lg bg-emerald-500/10 p-3 text-sm text-emerald-700 dark:text-emerald-400">
          <CheckIcon className="mt-0.5 size-4 shrink-0" />
          <span>
            <span className="font-medium">Máquina liberada.</span> O Git, o OpenSSH e a identidade estão configurados,
            e o acesso está autorizado por {caminho}.
          </span>
        </div>
      ) : (
        <div className="mt-3 flex items-start gap-2 rounded-lg bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-400">
          <AlertTriangleIcon className="mt-0.5 size-4 shrink-0" />
          <span>
            <span className="font-medium">Ainda não está liberada.</span> Falta: {faltas.join(", ")}. Cada item abaixo
            diz o que fazer.
          </span>
        </div>
      )}

      <ul className="mt-3 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
        <Item
          nome="Git"
          valor={tools?.git ?? null}
          falta={
            semResposta("git")
              ? "instalado, mas não respondeu dentro do prazo de 5s — algo está travando o git nesta máquina"
              : "não encontrado no PATH — instale o Git for Windows (winget install Git.Git)"
          }
        />
        <Item
          nome="OpenSSH (ssh / ssh-add)"
          valor={tools?.ssh ?? null}
          falta={
            semResposta("ssh")
              ? "instalado, mas não respondeu dentro do prazo de 5s"
              : "não encontrado — ative o recurso opcional OpenSSH do Windows"
          }
        />
        <Item
          nome="ssh-agent"
          valor={temChaveNoAgente ? `${tools?.agentKeys ?? 0} chave(s) carregada(s)` : null}
          falta={
            semResposta("ssh-add")
              ? "o ssh-add não respondeu dentro do prazo de 5s — o agente pode estar travado"
              : tools?.agentRunning === true
                ? "o agente está rodando, mas nenhuma chave foi carregada — ex.: ssh-add ~/.ssh/id_ed25519"
                : "agente parado — sem chave carregada, um remoto SSH não autentica (ex.: ssh-add ~/.ssh/id_ed25519)"
          }
          oculto={tools?.sshAdd == null && !semResposta("ssh-add")}
        />
        <Item
          nome="Identidade do git"
          valor={tools?.gitUser ?? null}
          falta={
            semResposta("git")
              ? "não deu para ler: o git não respondeu à sondagem dentro do prazo"
              : 'o git não consegue resolver a identidade para commit — git config --global user.name "Seu Nome"'
          }
        />
        <Item
          nome="gh CLI"
          valor={ghOk && contaGh ? `${tools?.gh ?? ""} · conta ${tools?.ghAccount ?? ""}` : null}
          falta={
            semResposta("gh")
              ? "instalado, mas não respondeu dentro do prazo de 5s — um gh travado não bloqueia nada aqui, o app publica sem ele"
              : ghOk
                ? "instalado sem sessão — rode gh auth login num terminal (a interface interativa dele não pode ser dirigida daqui)"
                : "não encontrado no PATH — se o gh já estiver instalado (por exemplo em C:/Program Files/GitHub CLI), adicione essa pasta ao PATH; é um caminho opcional, o app publica sem ele"
          }
          opcional
        />

        <li className="py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            <span>GitHub nesta aplicação</span>
            <span className="ml-auto flex items-center gap-2">
              {contaApp ? (
                <>
                  <Badge tone="success">liberado como @{status?.login ?? "?"}</Badge>
                  <CheckIcon className="size-4 text-emerald-500" />
                </>
              ) : (
                <>
                  <Badge tone={status === null ? "neutral" : "warning"}>
                    {status === null ? (statusErro !== null ? "estado não lido" : "lendo…") : "sem autorização"}
                  </Badge>
                  <Button
                    size="sm"
                    variant="primary"
                    loading={starting}
                    disabled={status?.hasClientId !== true || flow !== null}
                    onClick={() => void liberar()}
                  >
                    <ExternalLinkIcon className="size-3.5" /> Liberar pelo navegador
                  </Button>
                </>
              )}
            </span>
          </div>

          {contaApp || status?.hasClientId ? null : (
            <p className="mt-1.5 text-xs text-zinc-500 dark:text-zinc-400">
              Este caminho precisa do Client ID de um OAuth App (ele é público, não é segredo). Crie um em{" "}
              <button
                type="button"
                className="underline decoration-dotted"
                onClick={() =>
                  void openUrl("https://github.com/settings/developers").catch(() =>
                    pushToast("info", "Abra manualmente github.com/settings/developers"),
                  )
                }
              >
                github.com/settings/developers
              </button>{" "}
              e informe-o na aba GitHub.
            </p>
          )}

          {flow ? (
            <div className="mt-2 rounded-lg border border-zinc-200 p-3 dark:border-zinc-800">
              <p className="text-xs text-zinc-500 dark:text-zinc-400">Código de verificação</p>
              <p className="font-mono text-2xl font-semibold tracking-widest">{flow.userCode}</p>
              <p className="mt-1 text-xs text-zinc-500 dark:text-zinc-400">
                Digite-o em {flow.verificationUri} e autorize. Expira em {Math.round(flow.expiresIn / 60)} min.
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
                  <ExternalLinkIcon className="size-3.5" /> Abrir a autorização
                </Button>
                <span className="flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
                  <Spinner className="size-3" /> aguardando você autorizar
                </span>
                <Button size="sm" variant="ghost" onClick={() => setFlow(null)}>
                  Cancelar aqui
                </Button>
              </div>
            </div>
          ) : null}

          {erro ? (
            <p className="mt-2 flex items-start gap-2 rounded-lg bg-red-500/10 p-2.5 text-xs text-red-600 dark:text-red-400">
              <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {erro}
            </p>
          ) : null}
        </li>
      </ul>

      <p className="mt-3 text-xs text-zinc-500 dark:text-zinc-400">
        A prova definitiva é por repositório: na aba Git use <span className="font-medium">Testar acesso</span>, que faz
        o git perguntar ao próprio remoto (<span className="font-mono">git ls-remote</span>, com prazo de 25s e sem
        prompt de credencial) se esta máquina consegue publicar nele.
      </p>
    </Card>
  );
}

function Item({
  nome,
  valor,
  falta,
  opcional,
  oculto,
}: {
  nome: string;
  valor: string | null;
  falta: string;
  opcional?: boolean;
  oculto?: boolean;
}) {
  if (oculto === true) return null;
  return (
    <li className="flex flex-wrap items-start gap-2 py-2">
      <span>{nome}</span>
      <span className="ml-auto flex items-start gap-2 text-right">
        {valor ? (
          <>
            <span className="font-mono text-xs text-zinc-500 dark:text-zinc-400">{valor}</span>
            <CheckIcon className="mt-0.5 size-4 shrink-0 text-emerald-500" />
          </>
        ) : (
          <>
            <span
              className={
                opcional === true
                  ? "max-w-xs text-xs text-zinc-500 dark:text-zinc-400"
                  : "max-w-xs text-xs text-amber-700 dark:text-amber-400"
              }
            >
              {falta}
            </span>
            {opcional === true ? <Badge tone="neutral">opcional</Badge> : <XIcon className="mt-0.5 size-4 shrink-0 text-red-500" />}
          </>
        )}
      </span>
    </li>
  );
}
