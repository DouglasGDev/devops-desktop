import { useEffect, useMemo, useRef, useState } from "react";
import { CORTES, INSTRUMENTOS } from "../../lib/analysis";
import { api } from "../../lib/api";
import type {
  DeployRunStep,
  EngineStep,
  NodeAnalyzer,
  NodeKind,
  PipelineNode,
  PipelineNodeInput,
  SeverityGate,
  StepStatus,
} from "../../types";
import { useApp } from "../../store/AppContext";
import { AlertTriangleIcon, CheckIcon, PlusIcon, RefreshIcon, TrashIcon } from "../Icons";
import { Button, Card, Checkbox, Field, Input, SectionTitle, Select, Spinner, Textarea } from "../ui";

const NODE_W = 184;
const NODE_H = 56;
const GRID = 8;
const CANVAS_W = 920;

const KIND_FILL: Record<NodeKind, string> = {
  motor: "fill-sky-500/10 dark:fill-sky-500/15",
  comando: "fill-violet-500/10 dark:fill-violet-500/15",
  aprovacao: "fill-amber-500/10 dark:fill-amber-500/15",
  git: "fill-lime-500/10 dark:fill-lime-500/15",
  analise: "fill-rose-500/10 dark:fill-rose-500/15",
};

const KIND_STROKE: Record<NodeKind, string> = {
  motor: "stroke-sky-500/70",
  comando: "stroke-violet-500/70",
  aprovacao: "stroke-amber-500/70",
  git: "stroke-lime-500/70",
  analise: "stroke-rose-500/70",
};

const STATUS_STROKE: Record<StepStatus, string> = {
  waiting: "stroke-sky-400",
  running: "stroke-teal-500",
  done: "stroke-emerald-500",
  failed: "stroke-red-500",
  skipped: "stroke-amber-500",
};

const STATUS_FILL: Record<StepStatus, string> = {
  waiting: "fill-sky-400",
  running: "fill-teal-500",
  done: "fill-emerald-500",
  failed: "fill-red-500",
  skipped: "fill-amber-500",
};

function novoNo(quantos: number): PipelineNodeInput {
  return {
    nodeKey: `no-${quantos + 1}`,
    title: `Nó ${quantos + 1}`,
    kind: "comando",
    engineStep: null,
    executor: "local",
    environment: null,
    workingDir: null,
    commands: [""],
    dependsOn: [],
    parallel: false,
    timeoutSecs: 0,
    retryLimit: 0,
    condition: null,
    requiresApproval: false,
    x: 48,
    y: 48,
  };
}

function comoEntrada(no: PipelineNode): PipelineNodeInput {
  return {
    nodeKey: no.nodeKey,
    title: no.title,
    kind: no.kind,
    engineStep: no.engineStep,
    executor: no.executor,
    environment: no.environment,
    workingDir: no.workingDir,
    commands: no.commands,
    gitOp: no.gitOp,
    gitRef: no.gitRef,
    analyzer: no.analyzer,
    severityGate: no.severityGate,
    dependsOn: no.dependsOn,
    parallel: no.parallel,
    timeoutSecs: no.timeoutSecs,
    retryLimit: no.retryLimit,
    condition: no.condition,
    requiresApproval: no.requiresApproval,
    x: no.x,
    y: no.y,
  };
}

/** Estado vivo de cada nó da corrida atual, endereçado pela chave do grafo. */
function statusesDosNos(steps: DeployRunStep[]): Map<string, { status: StepStatus; exitCode: number | null }> {
  const map = new Map<string, { status: StepStatus; exitCode: number | null }>();
  for (const s of steps) {
    if (s.nodeKey) map.set(s.nodeKey, { status: s.status, exitCode: s.exitCode });
  }
  return map;
}

/**
 * Construtor visual do pipeline. O que é desenhado aqui é o que o motor percorre:
 * salvar passa pelo validador do backend, e rodar um nó é o mesmo motor de sempre
 * entrando pela porta do grafo.
 */
export function PipelineGraph({
  definitionId,
  hasServer,
  engineSteps,
  runSteps,
  running,
  onRunNode,
  onSaved,
}: {
  definitionId: number;
  hasServer: boolean;
  engineSteps: EngineStep[];
  runSteps: DeployRunStep[];
  running: boolean;
  onRunNode: (nodeKey: string, withDependencies: boolean) => void;
  onSaved: () => void;
}) {
  const { pushToast } = useApp();
  const [nodes, setNodes] = useState<PipelineNodeInput[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [check, setCheck] = useState<{ ok: boolean; message: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [carregando, setCarregando] = useState(true);
  const arrasto = useRef<{ chave: string; x0: number; y0: number; nx0: number; ny0: number } | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    let alive = true;
    setCarregando(true);
    void api
      .pipelineNodes(definitionId)
      .then((lista) => {
        if (!alive) return;
        setNodes(lista.map(comoEntrada));
        setSelected(lista[0]?.nodeKey ?? null);
        setCheck(null);
      })
      .catch((e) => {
        if (alive) pushToast("error", `Não foi possível ler o grafo: ${String(e)}`);
      })
      .finally(() => {
        if (alive) {
          setCarregando(false);
          setLoaded(true);
        }
      });
    return () => {
      alive = false;
    };
  }, [definitionId, pushToast]);

  const statuses = useMemo(() => statusesDosNos(runSteps), [runSteps]);

  const alterar = (chave: string, mudanca: Partial<PipelineNodeInput>) => {
    setNodes((atual) => atual.map((n) => (n.nodeKey === chave ? { ...n, ...mudanca } : n)));
  };

  const moverPara = (chave: string, x: number, y: number) => {
    setNodes((atual) =>
      atual.map((n) =>
        n.nodeKey === chave
          ? { ...n, x: Math.max(0, Math.round(x / GRID) * GRID), y: Math.max(0, Math.round(y / GRID) * GRID) }
          : n,
      ),
    );
  };

  const renomear = (chave: string, novaChave: string) => {
    const limpa = novaChave.trim();
    setNodes((atual) =>
      atual.map((n) =>
        n.nodeKey === chave
          ? {
              ...n,
              nodeKey: limpa,
              dependsOn: (n.dependsOn ?? []).map((d) => (d === chave ? limpa : d)),
            }
          : { ...n, dependsOn: (n.dependsOn ?? []).map((d) => (d === chave ? limpa : d)) },
      ),
    );
    if (selected === chave) setSelected(limpa);
  };

  const adicionar = () => {
    const no = novoNo(nodes.length);
    let chave = no.nodeKey;
    let n = nodes.length + 1;
    while (nodes.some((x) => x.nodeKey === chave)) {
      n += 1;
      chave = `no-${n}`;
    }
    setNodes((atual) => [...atual, { ...no, nodeKey: chave, x: 48 + ((atual.length * 40) % 480), y: 48 }]);
    setSelected(chave);
    setCheck(null);
  };

  const remover = (chave: string) => {
    setNodes((atual) =>
      atual
        .filter((n) => n.nodeKey !== chave)
        .map((n) => ({ ...n, dependsOn: (n.dependsOn ?? []).filter((d) => d !== chave) })),
    );
    setSelected(null);
    setCheck(null);
  };

  const conferir = async () => {
    try {
      const resumo = await api.pipelineCheckNodes(definitionId, nodes);
      setCheck({ ok: true, message: resumo });
    } catch (e) {
      setCheck({ ok: false, message: String(e) });
    }
  };

  const salvar = async () => {
    setSaving(true);
    try {
      const salvos = await api.pipelineSaveNodes(definitionId, nodes);
      setNodes(salvos.map(comoEntrada));
      const chaveAtual = selected;
      setSelected(salvos.some((n) => n.nodeKey === chaveAtual) ? chaveAtual : (salvos[0]?.nodeKey ?? null));
      setCheck({ ok: true, message: `grafo salvo · ${salvos.length} nó(s)` });
      pushToast("success", salvos.length > 0 ? `Grafo salvo com ${salvos.length} nó(s).` : "Definição voltou à fila de quinze etapas.");
      onSaved();
    } catch (e) {
      setCheck({ ok: false, message: String(e) });
      pushToast("error", `O grafo não foi salvo: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const selecionado = nodes.find((n) => n.nodeKey === selected) ?? null;
  const altura = Math.max(280, ...nodes.map((n) => (n.y ?? 0) + NODE_H + 40));
  const largura = Math.max(CANVAS_W, ...nodes.map((n) => (n.x ?? 0) + NODE_W + 40));

  if (!loaded || carregando) {
    return (
      <div className="flex items-center gap-2 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner /> Lendo o grafo…
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={adicionar} disabled={running}>
          <PlusIcon className="size-3.5" /> Nó
        </Button>
        <Button size="sm" onClick={() => void conferir()}>
          <CheckIcon className="size-3.5" /> Conferir
        </Button>
        <Button size="sm" variant="primary" loading={saving} onClick={() => void salvar()} disabled={running}>
          Salvar grafo
        </Button>
        {nodes.length > 0 ? (
          <Button
            size="sm"
            variant="ghost"
            disabled={running}
            onClick={() => {
              setNodes([]);
              setSelected(null);
              setCheck(null);
            }}
          >
            <TrashIcon className="size-3.5" /> Esvaziar
          </Button>
        ) : null}
        <span className="ml-auto text-[11px] text-zinc-500 dark:text-zinc-400">
          {nodes.length} nó(s){nodes.length === 0 ? " · fila de quinze etapas" : ""}
        </span>
      </div>

      {check ? (
        <p
          className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-xs ${
            check.ok
              ? "border-emerald-300 bg-emerald-50 text-emerald-900 dark:border-emerald-700/60 dark:bg-emerald-900/20 dark:text-emerald-200"
              : "border-red-300 bg-red-50 text-red-900 dark:border-red-700/60 dark:bg-red-900/20 dark:text-red-200"
          }`}
        >
          {check.ok ? <CheckIcon className="mt-0.5 size-3.5 shrink-0" /> : <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />}
          <span>{check.message}</span>
        </p>
      ) : null}

      <Card className="overflow-hidden">
        <div className="border-b border-zinc-200 px-4 py-2 dark:border-zinc-800">
          <SectionTitle>Grafo</SectionTitle>
        </div>
        <div className="overflow-auto">
          <svg
            ref={svgRef}
            width={largura}
            height={altura}
            viewBox={`0 0 ${largura} ${altura}`}
            className="block bg-zinc-50 dark:bg-zinc-950/40"
            onPointerMove={(e) => {
              const a = arrasto.current;
              if (!a) return;
              moverPara(a.chave, a.nx0 + (e.clientX - a.x0), a.ny0 + (e.clientY - a.y0));
            }}
            onPointerUp={() => {
              arrasto.current = null;
            }}
            onPointerLeave={() => {
              arrasto.current = null;
            }}
          >
            {nodes.map((no) =>
              (no.dependsOn ?? [])
                .map((de) => nodes.find((o) => o.nodeKey === de))
                .filter((de): de is PipelineNodeInput => Boolean(de))
                .map((de) => {
                  const x1 = (de.x ?? 0) + NODE_W;
                  const y1 = (de.y ?? 0) + NODE_H / 2;
                  const x2 = no.x ?? 0;
                  const y2 = (no.y ?? 0) + NODE_H / 2;
                  const meio = Math.max(24, (x2 - x1) / 2);
                  return (
                    <path
                      key={`${de.nodeKey}->${no.nodeKey}`}
                      d={`M ${x1} ${y1} C ${x1 + meio} ${y1}, ${x2 - meio} ${y2}, ${x2} ${y2}`}
                      fill="none"
                      strokeWidth={1.5}
                      className="stroke-zinc-400 dark:stroke-zinc-600"
                    />
                  );
                }),
            )}
            {nodes.map((no) => {
              const vivo = statuses.get(no.nodeKey);
              const ativo = no.nodeKey === selected;
              return (
                <g
                  key={no.nodeKey}
                  transform={`translate(${no.x ?? 0}, ${no.y ?? 0})`}
                  className="cursor-grab active:cursor-grabbing"
                  onPointerDown={(e) => {
                    svgRef.current?.setPointerCapture(e.pointerId);
                    arrasto.current = { chave: no.nodeKey, x0: e.clientX, y0: e.clientY, nx0: no.x ?? 0, ny0: no.y ?? 0 };
                    setSelected(no.nodeKey);
                  }}
                >
                  <rect
                    width={NODE_W}
                    height={NODE_H}
                    rx={10}
                    strokeWidth={ativo || vivo ? 2.2 : 1.4}
                    className={`${KIND_FILL[no.kind]} ${vivo ? STATUS_STROKE[vivo.status] : KIND_STROKE[no.kind]}`}
                  />
                  <text x={12} y={22} className="fill-zinc-800 text-[12px] font-medium dark:fill-zinc-100">
                    {(no.title || no.nodeKey).slice(0, 24)}
                  </text>
                  <text x={12} y={40} className="fill-zinc-500 text-[10px] dark:fill-zinc-400">
                    {no.kind === "motor"
                      ? `${no.engineStep ?? "—"}. ${(engineSteps.find((s) => s.position === no.engineStep)?.name ?? "sem etapa").slice(0, 22)}`
                      : no.kind === "aprovacao"
                        ? "espera decisão"
                        : no.kind === "git"
                        ? `git · ${no.gitOp ?? "sem operação"}`
                        : no.kind === "analise"
                          ? `análise · ${no.analyzer} · ${no.severityGate || "sem corte"}`
                          : `${no.executor} · ${(no.commands ?? []).filter((c) => c.trim()).length} comando(s)`}
                  </text>
                  {no.requiresApproval || no.kind === "aprovacao" ? (
                    <circle cx={NODE_W - 14} cy={14} r={4} className="fill-amber-500" />
                  ) : null}
                  {vivo ? (
                    vivo.status === "running" ? (
                      <circle cx={NODE_W - 14} cy={NODE_H - 16} r={5} className="fill-teal-500 animate-pulse" />
                    ) : (
                      <circle cx={NODE_W - 14} cy={NODE_H - 16} r={5} className={STATUS_FILL[vivo.status]} />
                    )
                  ) : null}
                </g>
              );
            })}
            {nodes.length === 0 ? (
              <text x={24} y={40} className="fill-zinc-500 text-[12px] dark:fill-zinc-400">
                sem grafo — a publicação percorre as quinze etapas do motor em fila
              </text>
            ) : null}
          </svg>
        </div>
      </Card>

      {selecionado ? (
        <Card className="p-4">
          <div className="flex items-center justify-between">
            <SectionTitle>Nó {selecionado.nodeKey}</SectionTitle>
            <div className="flex items-center gap-2">
              <Button size="sm" disabled={running} onClick={() => onRunNode(selecionado.nodeKey, false)}>
                Rodar sozinho
              </Button>
              <Button size="sm" disabled={running} onClick={() => onRunNode(selecionado.nodeKey, true)}>
                <RefreshIcon className="size-3.5" /> Com dependências
              </Button>
              <Button size="sm" variant="danger" disabled={running} onClick={() => remover(selecionado.nodeKey)}>
                <TrashIcon className="size-3.5" />
              </Button>
            </div>
          </div>

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <Field label="Identificador">
              <Input
                spellCheck={false}
                value={selecionado.nodeKey}
                onChange={(e) => renomear(selecionado.nodeKey, e.target.value)}
              />
            </Field>
            <Field label="Nome">
              <Input value={selecionado.title} onChange={(e) => alterar(selecionado.nodeKey, { title: e.target.value })} />
            </Field>
            <Field label="Tipo">
              <Select
                value={selecionado.kind}
                onChange={(e) => {
                  const kind = e.target.value as NodeKind;
                  alterar(selecionado.nodeKey, {
                    kind,
                    engineStep: kind === "motor" ? (selecionado.engineStep ?? 1) : null,
                    commands: kind === "comando" ? (selecionado.commands ?? [""]) : [],
                    // Os dois lêem a pasta desta máquina: o executor deles é local
                    // por definição, e um desenho com ssh seria recusado pelo motor.
                    executor: kind === "git" || kind === "analise" ? "local" : selecionado.executor,
                    gitOp: kind === "git" ? (selecionado.gitOp ?? "pull") : null,
                    gitRef: null,
                    analyzer: kind === "analise" ? (selecionado.analyzer ?? "embutido") : null,
                    severityGate: kind === "analise" ? (selecionado.severityGate ?? "ERROR") : null,
                  });
                }}
              >
                <option value="motor">etapa do motor</option>
                <option value="comando">comando</option>
                <option value="aprovacao">aprovação</option>
                <option value="git">git</option>
                <option value="analise">análise de código</option>
              </Select>
            </Field>
            {selecionado.kind === "motor" ? (
              <Field label="Etapa do motor">
                <Select
                  value={selecionado.engineStep ?? 0}
                  onChange={(e) => alterar(selecionado.nodeKey, { engineStep: Number(e.target.value) })}
                >
                  {engineSteps.map((s) => (
                    <option key={s.position} value={s.position}>
                      {String(s.position).padStart(2, "0")} · {s.name}
                    </option>
                  ))}
                </Select>
              </Field>
            ) : selecionado.kind === "comando" ? (
              <Field label="Executor">
                <Select
                  value={selecionado.executor ?? "local"}
                  onChange={(e) => alterar(selecionado.nodeKey, { executor: e.target.value as "local" | "ssh" })}
                >
                  <option value="local">nesta máquina</option>
                  <option value="ssh" disabled={!hasServer}>
                    no servidor
                  </option>
                </Select>
              </Field>
            ) : selecionado.kind === "git" ? (
              <Field label="Operação do Git">
                <Select
                  value={selecionado.gitOp ?? "pull"}
                  onChange={(e) => {
                    const op = e.target.value;
                    alterar(selecionado.nodeKey, {
                      gitOp: op,
                      gitRef: op === "pull" ? null : (selecionado.gitRef ?? ""),
                    });
                  }}
                >
                  <option value="pull">pull</option>
                  <option value="checkout">checkout</option>
                  <option value="tag">tag</option>
                </Select>
              </Field>
            ) : selecionado.kind === "analise" ? (
              <Field label="Instrumento">
                <Select
                  value={selecionado.analyzer ?? "embutido"}
                  onChange={(e) => alterar(selecionado.nodeKey, { analyzer: e.target.value as NodeAnalyzer })}
                >
                  {INSTRUMENTOS.map((i) => (
                    <option key={i.valor} value={i.valor}>
                      {i.rotulo}
                    </option>
                  ))}
                </Select>
              </Field>
            ) : (
              <p className="self-end text-xs text-zinc-500 dark:text-zinc-400">
                espera decisão · prazo de resposta 30 min
              </p>
            )}

            {selecionado.kind === "comando" ? (
              <>
                <div className="sm:col-span-2">
                  <Field label="Comandos (um por linha)">
                    <Textarea
                      rows={4}
                      spellCheck={false}
                      value={(selecionado.commands ?? []).join("\n")}
                      onChange={(e) =>
                        alterar(selecionado.nodeKey, { commands: e.target.value.split("\n") })
                      }
                    />
                  </Field>
                </div>
                <Field label={selecionado.executor === "ssh" ? "Pasta no servidor" : "Pasta no projeto"}>
                  <Input
                    spellCheck={false}
                    value={selecionado.workingDir ?? ""}
                    onChange={(e) => alterar(selecionado.nodeKey, { workingDir: e.target.value })}
                  />
                </Field>
                <Field label="Prazo (s, 0 = herdar)">
                  <Input
                    type="number"
                    value={selecionado.timeoutSecs ?? 0}
                    onChange={(e) => alterar(selecionado.nodeKey, { timeoutSecs: Number(e.target.value) })}
                  />
                </Field>
              </>
            ) : null}

            {selecionado.kind === "git" ? (
              <>
                {selecionado.gitOp !== "pull" ? (
                  <Field label={selecionado.gitOp === "tag" ? "Tag a criar" : "Ramo a alternar"}>
                    <Input
                      spellCheck={false}
                      className="font-mono text-xs"
                      value={selecionado.gitRef ?? ""}
                      onChange={(e) => alterar(selecionado.nodeKey, { gitRef: e.target.value })}
                      placeholder={selecionado.gitOp === "tag" ? "v1.0.0" : "main"}
                    />
                  </Field>
                ) : null}
                <Field label="Pasta no projeto">
                  <Input
                    spellCheck={false}
                    className="font-mono text-xs"
                    value={selecionado.workingDir ?? ""}
                    onChange={(e) => alterar(selecionado.nodeKey, { workingDir: e.target.value })}
                  />
                </Field>
              </>
            ) : null}

            {selecionado.kind === "analise" ? (
              <>
                <Field label="Corte de severidade">
                  <Select
                    value={selecionado.severityGate ?? "ERROR"}
                    onChange={(e) =>
                      alterar(selecionado.nodeKey, { severityGate: e.target.value as SeverityGate })
                    }
                  >
                    {CORTES.map((c) => (
                      <option key={c.valor} value={c.valor}>
                        {c.rotulo}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label="Pasta no projeto">
                  <Input
                    spellCheck={false}
                    className="font-mono text-xs"
                    value={selecionado.workingDir ?? ""}
                    onChange={(e) => alterar(selecionado.nodeKey, { workingDir: e.target.value })}
                  />
                </Field>
                <Field label="Prazo (s, 0 = padrão)">
                  <Input
                    type="number"
                    min={0}
                    max={3600}
                    value={selecionado.timeoutSecs ?? 0}
                    onChange={(e) =>
                      alterar(selecionado.nodeKey, { timeoutSecs: Math.min(3600, Math.max(0, Number(e.target.value) || 0)) })
                    }
                  />
                </Field>
              </>
            ) : null}

            <Field label="Ambiente">
              <Input
                spellCheck={false}
                value={selecionado.environment ?? ""}
                onChange={(e) => alterar(selecionado.nodeKey, { environment: e.target.value })}
              />
            </Field>
            <Field label="Condição (ramo=… / ambiente=…)">
              <Input
                spellCheck={false}
                value={selecionado.condition ?? ""}
                onChange={(e) => alterar(selecionado.nodeKey, { condition: e.target.value })}
              />
            </Field>
            {selecionado.kind !== "aprovacao" ? (
              <Field label="Novas tentativas">
                <Input
                  type="number"
                  value={selecionado.retryLimit ?? 0}
                  onChange={(e) => alterar(selecionado.nodeKey, { retryLimit: Number(e.target.value) })}
                />
              </Field>
            ) : null}

            <div className="flex flex-wrap items-center gap-4 sm:col-span-2">
              <Checkbox
                checked={Boolean(selecionado.requiresApproval)}
                onChange={(v) => alterar(selecionado.nodeKey, { requiresApproval: v })}
                label={<span className="text-xs">Exige aprovação</span>}
              />
              {selecionado.kind === "comando" ? (
                <Checkbox
                  checked={Boolean(selecionado.parallel)}
                  onChange={(v) => alterar(selecionado.nodeKey, { parallel: v })}
                  label={<span className="text-xs">Pode rodar em paralelo</span>}
                />
              ) : null}
            </div>

            <div className="sm:col-span-2">
              <p className="text-xs font-medium">Depende de</p>
              {nodes.length <= 1 ? (
                <p className="mt-1 text-[11px] text-zinc-500 dark:text-zinc-400">nenhum outro nó</p>
              ) : (
                <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1">
                  {nodes
                    .filter((n) => n.nodeKey !== selecionado.nodeKey)
                    .map((n) => (
                      <Checkbox
                        key={n.nodeKey}
                        checked={(selecionado.dependsOn ?? []).includes(n.nodeKey)}
                        onChange={(v) =>
                          alterar(selecionado.nodeKey, {
                            dependsOn: (selecionado.dependsOn ?? []).filter((d) => d !== n.nodeKey).concat(v ? [n.nodeKey] : []),
                          })
                        }
                        label={<span className="text-xs">{n.nodeKey}</span>}
                      />
                    ))}
                </div>
              )}
            </div>
          </div>
        </Card>
      ) : (
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          {nodes.length > 0 ? "Selecione um nó para editar." : "Nenhum nó salvo neste modelo."}
        </p>
      )}
    </div>
  );
}
