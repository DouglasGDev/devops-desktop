import { useCallback, useEffect, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { CORTES, INSTRUMENTOS } from "../lib/analysis";
import { formatDateTime, formatDuration } from "../lib/format";
import { useApp } from "../store/AppContext";
import type {
  AnalysisCheck,
  AnalysisCounts,
  AnalysisFinding,
  AnalysisPackage,
  AnalysisRule,
  AnalysisRun,
  AnalysisTool,
  NodeAnalyzer,
  SeverityGate,
} from "../types";
import {
  AlertTriangleIcon,
  ChecksIcon,
  CodeIcon,
  CopyIcon,
  DownloadIcon,
  RefreshIcon,
  SearchIcon,
  TrashIcon,
  XIcon,
} from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  Confirm,
  EmptyState,
  Field,
  Input,
  Modal,
  SectionTitle,
  Select,
  type BadgeTone,
} from "../components/ui";

/** A mesma chave que `analysis::CHAVE_CONFIG` lê no backend. */
const CHAVE_SEMGREP = "analise.semgrep.caminho";

const SEVERIDADE_TOM: Record<string, BadgeTone> = { ERROR: "error", WARNING: "warning", INFO: "info" };
const STATUS_TOM: Record<string, BadgeTone> = {
  concluida: "success",
  falhou: "error",
  interrompida: "warning",
  rodando: "info",
};
const FERRAMENTA_TOM: Record<string, BadgeTone> = {
  encontrado: "success",
  ausente: "neutral",
  "sem-resposta": "warning",
};
const CONFERENCIA_TOM: Record<string, BadgeTone> = {
  validadas: "success",
  inválidas: "error",
  "sem-ferramenta": "neutral",
  "sem-resposta": "warning",
};

/** `total()` é um método do Rust, não um campo do JSON: a soma é feita aqui. */
function total(cont: AnalysisCounts): number {
  return cont.error + cont.warning + cont.info;
}

export function AnalysisView() {
  const { projects, settings, setSetting, pushToast } = useApp();
  const [projectId, setProjectId] = useState<number>(projects[0]?.id ?? 0);
  const [analyzer, setAnalyzer] = useState<NodeAnalyzer>("embutido");
  const [gate, setGate] = useState<SeverityGate>("ERROR");
  const [subpasta, setSubpasta] = useState("");
  const [prazo, setPrazo] = useState(0);

  const [ferramenta, setFerramenta] = useState<AnalysisTool | null>(null);
  const [conferencia, setConferencia] = useState<AnalysisCheck | null>(null);
  const [regras, setRegras] = useState<AnalysisRule[]>([]);
  const [mostrarRegras, setMostrarRegras] = useState(false);
  const [caminhoSemgrep, setCaminhoSemgrep] = useState(settings[CHAVE_SEMGREP] ?? "");

  const [corridas, setCorridas] = useState<AnalysisRun[]>([]);
  const [selecionada, setSelecionada] = useState<number | null>(null);
  const [achados, setAchados] = useState<AnalysisFinding[]>([]);
  const [filtro, setFiltro] = useState<"" | "ERROR" | "WARNING" | "INFO">("");
  const [varrendo, setVarrendo] = useState<number[]>([]);
  const [pedido, setPedido] = useState(false);
  const [conferindo, setConferindo] = useState(false);
  const [pacote, setPacote] = useState<AnalysisPackage | null>(null);
  const [destino, setDestino] = useState<string | null>(null);
  const [removendo, setRemovendo] = useState<AnalysisRun | null>(null);

  const corrida = corridas.find((c) => c.id === selecionada) ?? null;
  const emVarredura = varrendo.includes(projectId);

  const sondar = useCallback(async () => {
    try {
      setFerramenta(await api.analysisDetectar());
    } catch (e) {
      pushToast("error", `Falha ao sondar a ferramenta: ${String(e)}`);
    }
  }, [pushToast]);

  const carregarCorridas = useCallback(async () => {
    if (!projectId) return;
    try {
      setCorridas(await api.analysisCorridas(projectId, 50));
    } catch (e) {
      pushToast("error", `Falha ao ler as varreduras: ${String(e)}`);
    }
  }, [projectId, pushToast]);

  useEffect(() => {
    void sondar();
    api.analysisRegras().then(setRegras).catch((e) => pushToast("error", String(e)));
  }, [sondar, pushToast]);

  // O registro de varreduras vive no backend: é dele que a tela lê "está varrendo",
  // inclusive quando a varredura começou antes desta janela existir.
  useEffect(() => {
    let vivo = true;
    const sonda = async () => {
      try {
        const ids = await api.analysisVarrendo();
        if (vivo) setVarrendo(ids);
      } catch {
        /* a janela segue mostrando o que já leu */
      }
    };
    void sonda();
    const timer = setInterval(() => void sonda(), 2000);
    return () => {
      vivo = false;
      clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (pedido || emVarredura) return;
    void carregarCorridas();
  }, [carregarCorridas, emVarredura, pedido]);

  useEffect(() => {
    if (!selecionada) {
      setAchados([]);
      return;
    }
    let vivo = true;
    api.analysisAchados(selecionada, filtro || null, 500)
      .then((lista) => {
        if (vivo) setAchados(lista);
      })
      .catch((e) => pushToast("error", String(e)));
    return () => {
      vivo = false;
    };
  }, [selecionada, filtro, pushToast]);

  const varrer = async () => {
    if (!projectId) return;
    setPedido(true);
    setDestino(null);
    try {
      const nova = await api.analysisExecutar({
        projectId,
        analyzer,
        severityGate: gate,
        path: subpasta.trim() === "" ? null : subpasta.trim(),
        timeoutSecs: prazo,
      });
      setCorridas((atual) => [nova, ...atual.filter((c) => c.id !== nova.id)]);
      setSelecionada(nova.id);
      pushToast(
        nova.blocked ? "info" : "success",
        `Corrida #${nova.id}: ${nova.counts.error} erro, ${nova.counts.warning} aviso, ${nova.counts.info} info em ${nova.filesScanned} arquivo(s)${nova.note ? ` · ${nova.note}` : ""}`,
      );
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setPedido(false);
      void sondar();
    }
  };

  const interromper = async () => {
    try {
      const conseguiu = await api.analysisInterromper(projectId);
      pushToast(
        "info",
        conseguiu ? "Varredura interrompida." : "Nenhuma varredura deste projeto em andamento.",
      );
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const conferir = async () => {
    setConferindo(true);
    try {
      setConferencia(await api.analysisConferir());
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setConferindo(false);
    }
  };

  const apontarFerramenta = async () => {
    // Sem filtro de extensão: no POSIX o semgrep é um script sem `.exe`, e a
    // checagem de quem existe é do backend, não da janela de escolha.
    const escolha = await open({ directory: false, multiple: false, title: "Executável do semgrep" });
    if (typeof escolha === "string" && escolha.trim() !== "") setCaminhoSemgrep(escolha);
  };

  const salvarCaminho = async () => {
    try {
      await setSetting(CHAVE_SEMGREP, caminhoSemgrep.trim());
      setCaminhoSemgrep(caminhoSemgrep.trim());
      await sondar();
    } catch (e) {
      pushToast("error", `Falha ao salvar o caminho: ${String(e)}`);
    }
  };

  const exportar = async (formato: "pacote" | "sarif") => {
    if (!selecionada) return;
    const escolhas = formato === "sarif" ? ["sarif", "json"] : ["json"];
    const escolha = await save({
      title: formato === "sarif" ? "Exportar SARIF" : "Exportar pacote",
      defaultPath: `analise-corrida-${selecionada}.${escolhas[0]}`,
      filters: [{ name: formato === "sarif" ? "SARIF" : "JSON", extensions: escolhas }],
    });
    if (!escolha) return;
    try {
      setDestino(await api.analysisExportar(selecionada, formato, escolha));
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const lerPacote = async () => {
    if (!selecionada) return;
    try {
      setPacote(await api.analysisPacote(selecionada));
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const remover = async () => {
    if (!removendo) return;
    const id = removendo.id;
    setRemovendo(null);
    try {
      await api.analysisRemover(id);
      if (selecionada === id) setSelecionada(null);
      await carregarCorridas();
      pushToast("info", `Corrida #${id} removida.`);
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const copiarPacote = async () => {
    if (!pacote) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(pacote, null, 2));
      pushToast("success", "Pacote copiado.");
    } catch (e) {
      pushToast("error", `Não foi possível copiar: ${String(e)}`);
    }
  };

  return (
    <div className="space-y-4">
      <Card className="p-4">
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
          <Field label="Projeto">
            <Select value={projectId} onChange={(e) => (setProjectId(Number(e.target.value)), setSelecionada(null))}>
              {projects.length === 0 ? <option value={0}>Nenhum projeto</option> : null}
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Instrumento">
            <Select value={analyzer} onChange={(e) => setAnalyzer(e.target.value as NodeAnalyzer)}>
              {INSTRUMENTOS.map((i) => (
                <option key={i.valor} value={i.valor}>
                  {i.rotulo}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Corte">
            <Select value={gate} onChange={(e) => setGate(e.target.value as SeverityGate)}>
              {CORTES.map((c) => (
                <option key={c.valor} value={c.valor}>
                  {c.rotulo}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Subpasta">
            <Input
              value={subpasta}
              placeholder="src"
              onChange={(e) => setSubpasta(e.target.value)}
              spellCheck={false}
            />
          </Field>
          <Field label="Prazo (s)">
            <Input
              type="number"
              min={0}
              max={3600}
              value={prazo}
              onChange={(e) => setPrazo(Math.max(0, Math.min(3600, Number(e.target.value) || 0)))}
            />
          </Field>
        </div>

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button variant="primary" loading={pedido || emVarredura} disabled={!projectId} onClick={() => void varrer()}>
            <SearchIcon /> Varrer código
          </Button>
          {emVarredura ? (
            <Button variant="danger" onClick={() => void interromper()}>
              <XIcon /> Interromper
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" onClick={() => setMostrarRegras((v) => !v)}>
            {mostrarRegras ? "Ocultar" : "Ver"} regras embarcadas ({regras.length})
          </Button>
          <Button size="sm" variant="ghost" loading={conferindo} onClick={() => void conferir()}>
            <ChecksIcon /> Conferir regras
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void sondar()}>
            <RefreshIcon /> Sondar ferramenta
          </Button>
        </div>

        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
          {ferramenta ? (
            <>
              <Badge tone={FERRAMENTA_TOM[ferramenta.estado] ?? "neutral"}>semgrep: {ferramenta.estado}</Badge>
              {ferramenta.versao ? <Badge>v {ferramenta.versao}</Badge> : null}
              {ferramenta.origem ? <Badge>origem: {ferramenta.origem}</Badge> : null}
              <span className="text-zinc-500 dark:text-zinc-400">sonda {formatDuration(ferramenta.duracaoMs)}</span>
            </>
          ) : (
            <Badge>semgrep: —</Badge>
          )}
          {conferencia ? (
            <>
              <Badge tone={CONFERENCIA_TOM[conferencia.estado] ?? "neutral"}>regras: {conferencia.estado}</Badge>
              <span className="max-w-xl truncate text-zinc-500 dark:text-zinc-400" title={conferencia.mensagem}>
                {conferencia.mensagem}
              </span>
            </>
          ) : null}
        </div>

        <div className="mt-3 flex flex-wrap items-end gap-2">
          <Field label="Caminho do semgrep">
            <Input
              className="min-w-64 font-mono text-xs"
              value={caminhoSemgrep}
              placeholder="semgrep"
              onChange={(e) => setCaminhoSemgrep(e.target.value)}
              spellCheck={false}
            />
          </Field>
          <Button size="sm" onClick={() => void apontarFerramenta()}>
            Escolher…
          </Button>
          <Button size="sm" variant="primary" onClick={() => void salvarCaminho()}>
            Salvar
          </Button>
        </div>

        {mostrarRegras ? (
          <ul className="mt-3 max-h-64 divide-y divide-zinc-100 overflow-y-auto border border-zinc-200 text-xs dark:divide-zinc-800 dark:border-zinc-800">
            {regras.map((r) => (
              <li key={r.id} className="flex flex-wrap items-center gap-2 px-2.5 py-1.5">
                <Badge tone={SEVERIDADE_TOM[r.severidade] ?? "neutral"}>{r.severidade}</Badge>
                <span className="font-mono">{r.id}</span>
                <span className="min-w-0 flex-1 truncate text-zinc-600 dark:text-zinc-300" title={r.mensagem}>
                  {r.mensagem}
                </span>
                {r.cwe ? <span className="font-mono text-[11px] text-zinc-400">{r.cwe}</span> : null}
                <span className="text-[11px] text-zinc-400">{r.familias.join(", ")}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </Card>

      {projects.length === 0 ? (
        <Card>
          <EmptyState
            icon={<CodeIcon className="size-8" />}
            title="Nenhum projeto cadastrado"
            description="A varredura lê o código de um projeto cadastrado."
          />
        </Card>
      ) : (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
          <Card className="p-2">
            <SectionTitle>Varreduras deste projeto</SectionTitle>
            {corridas.length === 0 ? (
              <p className="px-2 py-3 text-xs text-zinc-500 dark:text-zinc-400">
                {emVarredura ? "Varredura em andamento…" : "Nenhuma varredura gravada."}
              </p>
            ) : (
              <ul className="mt-1 space-y-1">
                {corridas.map((c) => (
                  <li key={c.id}>
                    <button
                      onClick={() => (setSelecionada(c.id), setDestino(null))}
                      className={`w-full rounded-lg px-2.5 py-2 text-left text-xs transition ${
                        selecionada === c.id
                          ? "bg-teal-600/10 dark:bg-teal-400/10"
                          : "hover:bg-zinc-100 dark:hover:bg-zinc-800/70"
                      }`}
                    >
                      <span className="flex items-center gap-1.5">
                        <span className="font-medium">#{c.id}</span>
                        <Badge tone={STATUS_TOM[c.status] ?? "neutral"}>{c.status}</Badge>
                        {c.blocked ? <Badge tone="error">barra</Badge> : null}
                        <span className="ml-auto text-zinc-400">{formatDateTime(c.startedAt)}</span>
                      </span>
                      <span className="mt-1 flex items-center gap-2 text-zinc-500 dark:text-zinc-400">
                        <span>{c.analyzer}</span>
                        <span>{c.filesScanned} arquivo(s)</span>
                        <span className="font-mono">
                          {c.counts.error}/{c.counts.warning}/{c.counts.info}
                        </span>
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Card>

          {corrida ? (
            <Card className="p-4">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-semibold">Corrida #{corrida.id}</span>
                <Badge tone={STATUS_TOM[corrida.status] ?? "neutral"}>{corrida.status}</Badge>
                <Badge tone={corrida.blocked ? "error" : "success"}>
                  {corrida.blocked ? "barra a publicação" : "não barra"}
                </Badge>
                <Badge tone="accent">{corrida.analyzer}</Badge>
                <Badge>{corrida.tool}</Badge>
                {corrida.toolVersion ? <Badge>v {corrida.toolVersion}</Badge> : null}
                <Badge>corte {corrida.severityGate || "nada"}</Badge>
                <span className="ml-auto text-xs text-zinc-500 dark:text-zinc-400">
                  {formatDuration(corrida.durationMs)}
                </span>
              </div>

              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                <Badge tone={corrida.counts.error > 0 ? "error" : "neutral"}>{corrida.counts.error} erro</Badge>
                <Badge tone={corrida.counts.warning > 0 ? "warning" : "neutral"}>{corrida.counts.warning} aviso</Badge>
                <Badge tone={corrida.counts.info > 0 ? "info" : "neutral"}>{corrida.counts.info} info</Badge>
                <Badge>{corrida.filesScanned} lidos</Badge>
                {corrida.filesSkipped > 0 ? <Badge>{corrida.filesSkipped} pulados</Badge> : null}
                {corrida.toolErrors > 0 ? <Badge tone="warning">{corrida.toolErrors} erros da ferramenta</Badge> : null}
                <Badge title="O código de saída da ferramenta não é o veredito">
                  código {corrida.exitCode ?? "—"}
                </Badge>
              </div>

              {corrida.note ? (
                <p className="mt-2 flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300">
                  <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />
                  {corrida.note}
                </p>
              ) : null}

              <div className="mt-3 flex flex-wrap items-center gap-2">
                <Button size="sm" onClick={() => void lerPacote()}>
                  <CodeIcon /> Pacote para IA
                </Button>
                <Button size="sm" onClick={() => void exportar("pacote")}>
                  <DownloadIcon /> Exportar pacote
                </Button>
                <Button size="sm" onClick={() => void exportar("sarif")}>
                  <DownloadIcon /> Exportar SARIF
                </Button>
                <Button size="sm" variant="danger" onClick={() => setRemovendo(corrida)}>
                  <TrashIcon /> Remover
                </Button>
                {destino ? (
                  <span className="min-w-0 truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400" title={destino}>
                    {destino}
                  </span>
                ) : null}
              </div>

              <div className="mt-4 flex items-center gap-2">
                <span className="text-xs font-medium text-zinc-500 dark:text-zinc-400">Achados</span>
                <Select
                  className="max-w-40"
                  value={filtro}
                  onChange={(e) => setFiltro(e.target.value as typeof filtro)}
                >
                  <option value="">Todas as severidades</option>
                  <option value="ERROR">Só ERROR</option>
                  <option value="WARNING">Só WARNING</option>
                  <option value="INFO">Só INFO</option>
                </Select>
                <span className="text-[11px] text-zinc-400">
                  {achados.length} de {total(corrida.counts)}
                </span>
              </div>

              {achados.length === 0 ? (
                <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
                  {total(corrida.counts) === 0 ? "Nenhum achado nesta corrida." : "Nenhum achado com este filtro."}
                </p>
              ) : (
                <ul className="mt-2 divide-y divide-zinc-100 dark:divide-zinc-800">
                  {achados.map((a) => (
                    <li key={a.id} className="px-1 py-2 text-xs">
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge tone={SEVERIDADE_TOM[a.severity] ?? "neutral"}>{a.severity}</Badge>
                        <span className="font-mono">{a.ruleId}</span>
                        <span className="text-zinc-400">{a.source}</span>
                        {a.cwe ? <span className="font-mono text-[11px] text-zinc-400">{a.cwe}</span> : null}
                      </div>
                      <p className="mt-1 text-zinc-700 dark:text-zinc-200">{a.message}</p>
                      <p className="mt-0.5 font-mono text-[11px] text-zinc-500 dark:text-zinc-400">
                        {a.file}:{a.lineStart}
                        {a.lineEnd > a.lineStart ? `-${a.lineEnd}` : ""}
                      </p>
                      {a.snippet ? (
                        <pre className="mt-1 overflow-x-auto rounded-md bg-zinc-50 px-2 py-1 font-mono text-[11px] text-zinc-700 dark:bg-zinc-950 dark:text-zinc-300">
                          {a.snippet}
                        </pre>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          ) : (
            <Card className="p-4">
              <EmptyState
                icon={<SearchIcon className="size-8" />}
                title="Nenhuma corrida escolhida"
                description="Escolha uma varredura na lista ou varra o código agora."
              />
            </Card>
          )}
        </div>
      )}

      <Modal open={pacote !== null} onClose={() => setPacote(null)} title={`Pacote da corrida #${pacote?.execucao.id ?? ""}`} width="max-w-4xl">
        {pacote ? (
          <>
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <Badge tone="accent">{pacote.veredito.classificacao}</Badge>
              <Badge tone={pacote.veredito.bloqueia ? "error" : "success"}>
                {pacote.veredito.bloqueia ? "barra" : "não barra"}
              </Badge>
              {pacote.decisoes.map((d) => (
                <Badge key={d.acao} tone="info" title={`${d.motivo} · evidência: ${d.evidencia}`}>
                  {d.acao}
                </Badge>
              ))}
              {pacote.truncado ? <Badge tone="warning">achados truncados</Badge> : null}
              <Button size="sm" className="ml-auto" onClick={() => void copiarPacote()}>
                <CopyIcon /> Copiar
              </Button>
            </div>
            <pre className="mt-3 max-h-[55vh] overflow-auto rounded-lg bg-zinc-50 p-3 font-mono text-[11px] leading-relaxed text-zinc-700 dark:bg-zinc-950 dark:text-zinc-300">
              {JSON.stringify(pacote, null, 2)}
            </pre>
          </>
        ) : null}
      </Modal>

      <Confirm
        open={removendo !== null}
        title="Remover varredura"
        message={`Remover a corrida #${removendo?.id} e os ${removendo ? total(removendo.counts) : 0} achados dela do histórico local? O código varrido não é tocado.`}
        confirmLabel="Remover"
        onConfirm={() => void remover()}
        onClose={() => setRemovendo(null)}
      />
    </div>
  );
}
