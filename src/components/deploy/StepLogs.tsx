import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { StepLogLine } from "../../types";
import { Button } from "../ui";
import { LoaderIcon } from "../Icons";

const TONALIDADE: Record<StepLogLine["stream"], string> = {
  stdout: "text-zinc-300",
  stderr: "text-red-300",
  system: "text-amber-300",
};

/**
 * O log de uma etapa lido do banco, em blocos por tentativa. A gravação é do motor,
 * no mesmo instante em que ele emite a linha; esta é a porta que a relê depois de a
 * janela fechar e reabrir.
 */
export function StepLogs({ runId, position }: { runId: number; position: number }) {
  const [busca, setBusca] = useState("");
  const [consulta, setConsulta] = useState<string | null>(null);
  const [linhas, setLinhas] = useState<StepLogLine[] | null>(null);
  const [erro, setErro] = useState<string | null>(null);

  useEffect(() => {
    let vivo = true;
    setLinhas(null);
    setErro(null);
    api
      .pipelineStepLogs(runId, position, consulta)
      .then((l) => {
        if (vivo) setLinhas(l);
      })
      .catch((e: unknown) => {
        if (vivo) setErro(String(e));
      });
    return () => {
      vivo = false;
    };
  }, [runId, position, consulta]);

  // A leitura vem ordenada por tentativa e ordem, então um bloco novo só começa
  // quando o número da tentativa muda.
  const blocos: { attempt: number; linhas: StepLogLine[] }[] = [];
  for (const linha of linhas ?? []) {
    const ultimo = blocos[blocos.length - 1];
    if (ultimo && ultimo.attempt === linha.attempt) ultimo.linhas.push(linha);
    else blocos.push({ attempt: linha.attempt, linhas: [linha] });
  }

  return (
    <div className="mt-2 space-y-2">
      <div className="flex items-center gap-2">
        <input
          value={busca}
          onChange={(e) => setBusca(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") setConsulta(busca.trim() || null);
          }}
          placeholder="Buscar no log…"
          className="min-w-0 flex-1 rounded-lg border border-zinc-300 bg-white px-2 py-1 font-mono text-[11px] outline-none focus:ring-2 focus:ring-teal-500/40 dark:border-zinc-700 dark:bg-zinc-950"
        />
        {consulta !== null ? (
          <Button
            size="sm"
            onClick={() => {
              setBusca("");
              setConsulta(null);
            }}
          >
            Limpar
          </Button>
        ) : null}
      </div>

      {erro ? <p className="text-[11px] text-red-600 dark:text-red-400">{erro}</p> : null}

      {!erro && linhas === null ? (
        <p className="flex items-center gap-1.5 text-[11px] text-zinc-500 dark:text-zinc-400">
          <LoaderIcon className="size-3 animate-spin" /> lendo o log…
        </p>
      ) : null}

      {!erro && linhas !== null && blocos.length === 0 ? (
        <p className="text-[11px] text-zinc-500 dark:text-zinc-400">
          {consulta ? `nenhuma linha com “${consulta}”` : "nenhuma linha gravada nesta etapa"}
        </p>
      ) : null}

      <div className="max-h-60 overflow-y-auto rounded-lg bg-zinc-950 px-3 py-2 font-mono text-[11px] leading-relaxed">
        {blocos.map((bloco) => (
          <div key={bloco.attempt} className="mb-2 last:mb-0">
            <p className="select-none text-zinc-500">
              tentativa {bloco.attempt} · {bloco.linhas.length} linha(s)
            </p>
            {bloco.linhas.map((linha) => (
              <p key={linha.seq} className={TONALIDADE[linha.stream] ?? "text-zinc-300"}>
                <span className="select-none pr-2 text-zinc-600">{String(linha.seq).padStart(4, "0")}</span>
                {linha.text}
              </p>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
