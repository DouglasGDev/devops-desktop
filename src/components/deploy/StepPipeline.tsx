import { useState } from "react";
import type { DeployRunStep, StepStatus } from "../../types";
import { StepLogs } from "./StepLogs";
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, LoaderIcon, XIcon } from "../Icons";

const TONE: Record<StepStatus, { icon: string; ring: string; text: string }> = {
  waiting: { icon: "border-zinc-300 dark:border-zinc-700", ring: "", text: "text-zinc-400 dark:text-zinc-500" },
  running: { icon: "border-teal-500", ring: "bg-teal-500/10", text: "text-teal-700 dark:text-teal-300" },
  done: { icon: "border-emerald-500", ring: "", text: "text-emerald-700 dark:text-emerald-300" },
  failed: { icon: "border-red-500", ring: "bg-red-500/10", text: "text-red-700 dark:text-red-300" },
  skipped: { icon: "border-amber-500", ring: "", text: "text-amber-700 dark:text-amber-300" },
};

export const STEP_STATUS_LABELS: Record<StepStatus, string> = {
  waiting: "aguardando",
  running: "em execução",
  done: "concluída",
  failed: "falhou",
  skipped: "não executada",
};

function StepMark({ status }: { status: StepStatus }) {
  if (status === "running") return <LoaderIcon className="size-3.5 animate-spin text-teal-600 dark:text-teal-300" />;
  if (status === "done") return <CheckIcon className="size-3.5 text-emerald-600 dark:text-emerald-300" />;
  if (status === "failed") return <XIcon className="size-3.5 text-red-600 dark:text-red-300" />;
  return <span className="block size-1.5 rounded-full bg-current opacity-60" />;
}

/** As quinze etapas do capítulo 10, na ordem, com o estado real de cada uma. */
export function StepPipeline({ steps, runId }: { steps: DeployRunStep[]; runId: number | null }) {
  const [aberta, setAberta] = useState<number | null>(null);
  return (
    <ol className="space-y-1">
      {steps.map((step) => {
        const tone = TONE[step.status] ?? TONE.waiting;
        const abrindo = aberta === step.position;
        return (
          <li
            key={step.position}
            className={`rounded-lg border px-3 py-2 ${tone.ring} ${
              step.status === "running"
                ? "border-teal-500/60 bg-teal-500/5"
                : "border-zinc-200 dark:border-zinc-800"
            }`}
          >
            <div className="flex items-start gap-2.5">
              <span
                className={`mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border ${tone.icon}`}
              >
                <StepMark status={step.status} />
              </span>
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-baseline gap-x-2 text-sm">
                  <span className="font-mono text-[11px] text-zinc-400 dark:text-zinc-500">
                    {String(step.position).padStart(2, "0")}
                  </span>
                  <span className="font-medium">{step.name}</span>
                  <span className={`text-[11px] ${tone.text}`}>{STEP_STATUS_LABELS[step.status] ?? step.status}</span>
                  {step.exitCode !== null && step.exitCode !== 0 ? (
                    <span className="text-[11px] text-red-600 dark:text-red-400">
                      saída {step.exitCode}
                    </span>
                  ) : null}
                </p>
                {step.command ? (
                  <p className="truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400" title={step.command}>
                    $ {step.command}
                  </p>
                ) : null}
                {step.message ? (
                  <p className="whitespace-pre-line text-[11px] text-zinc-500 dark:text-zinc-400">{step.message}</p>
                ) : null}
                {runId !== null ? (
                  <>
                    <button
                      type="button"
                      onClick={() => setAberta(abrindo ? null : step.position)}
                      className="mt-1 inline-flex items-center gap-1 text-[11px] text-zinc-500 hover:text-teal-700 dark:text-zinc-400 dark:hover:text-teal-300"
                    >
                      {abrindo ? (
                        <ChevronDownIcon className="size-3" />
                      ) : (
                        <ChevronRightIcon className="size-3" />
                      )}
                      Saída da etapa
                    </button>
                    {abrindo ? <StepLogs runId={runId} position={step.position} /> : null}
                  </>
                ) : null}
              </div>
            </div>
          </li>
        );
      })}
    </ol>
  );
}
