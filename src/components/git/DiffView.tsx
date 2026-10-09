import { useState } from "react";
import type { DiffHunk, FileDiff } from "../../types";
import { Badge, Button, EmptyState, Spinner } from "../ui";
import { AlertTriangleIcon, CodeIcon } from "../Icons";

const LINE_STYLES: Record<string, string> = {
  add: "bg-emerald-500/10 text-emerald-800 dark:text-emerald-300",
  del: "bg-red-500/10 text-red-800 dark:text-red-300",
  ctx: "text-zinc-600 dark:text-zinc-400",
};

const MARKS: Record<string, string> = { add: "+", del: "-", ctx: " " };

const STATUS_LABELS: Record<string, { label: string; tone: "success" | "error" | "warning" | "accent" }> = {
  A: { label: "novo", tone: "success" },
  M: { label: "modificado", tone: "accent" },
  D: { label: "removido", tone: "error" },
  R: { label: "renomeado", tone: "warning" },
  C: { label: "copiado", tone: "warning" },
  T: { label: "tipo alterado", tone: "warning" },
};

function statusOf(char: string) {
  return STATUS_LABELS[char] ?? { label: char, tone: "accent" as const };
}

function fileStats(file: FileDiff) {
  let added = 0;
  let removed = 0;
  for (const hunk of file.hunks) {
    for (const line of hunk.lines) {
      if (line.kind === "add") added += 1;
      else if (line.kind === "del") removed += 1;
    }
  }
  return { added, removed };
}

function HunkLines({ hunk }: { hunk: DiffHunk }) {
  return (
    <div className="overflow-x-auto font-mono text-[12px] leading-[1.55]">
      {hunk.lines.map((line, i) => (
        <div key={i} className="flex min-w-max">
          <span className="w-12 shrink-0 select-none border-r border-zinc-100 px-1 text-right text-[11px] text-zinc-400 dark:border-zinc-800 dark:text-zinc-600">
            {line.oldNo ?? ""}
          </span>
          <span className="w-12 shrink-0 select-none border-r border-zinc-100 px-1 text-right text-[11px] text-zinc-400 dark:border-zinc-800 dark:text-zinc-600">
            {line.newNo ?? ""}
          </span>
          <span className={`w-full whitespace-pre px-2 ${LINE_STYLES[line.kind]}`}>
            {MARKS[line.kind]}
            {line.text}
          </span>
        </div>
      ))}
    </div>
  );
}

/**
 * Diff linha a linha com staging por trecho.
 *
 * `onToggleHunks` só é passado na tela de alterações: é ela que permite preparar
 * um hunk individual. Na visualização de um commit os hunks são somente leitura.
 */
export function DiffView({
  files,
  loading,
  emptyHint,
  onToggleHunks,
  onOpenFile,
}: {
  files: FileDiff[];
  loading: boolean;
  emptyHint: string;
  onToggleHunks?: (path: string, indexes: number[]) => Promise<number | null>;
  onOpenFile?: (path: string) => void;
}) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [applying, setApplying] = useState<string | null>(null);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 py-14 text-sm text-zinc-500 dark:text-zinc-400">
        <Spinner /> Carregando o diff…
      </div>
    );
  }

  if (files.length === 0) {
    return (
      <div className="flex h-full items-center justify-center">
        <EmptyState icon={<CodeIcon className="size-8" />} title="Nada para comparar" description={emptyHint} />
      </div>
    );
  }

  const toggle = async (path: string, index: number) => {
    if (!onToggleHunks) return;
    setApplying(`${path}#${index}`);
    try {
      await onToggleHunks(path, [index]);
    } finally {
      setApplying(null);
    }
  };

  return (
    <div className="space-y-4">
      {files.map((file) => {
        const { added, removed } = fileStats(file);
        const info = statusOf(file.statusChar);
        const isCollapsed = collapsed[file.path] === true;
        return (
          <div key={file.path} className="overflow-hidden rounded-xl border border-zinc-200 dark:border-zinc-800">
            <div className="flex flex-wrap items-center gap-2 border-b border-zinc-200 bg-zinc-50 px-3 py-2 dark:border-zinc-800 dark:bg-zinc-900/60">
              <button
                onClick={() => setCollapsed((prev) => ({ ...prev, [file.path]: !(prev[file.path] === true) }))}
                className="text-xs text-zinc-500 transition hover:text-zinc-800 dark:hover:text-zinc-200"
                title={isCollapsed ? "Expandir o arquivo" : "Recolher o arquivo"}
              >
                {isCollapsed ? "▸" : "▾"}
              </button>
              <span className="min-w-0 truncate font-mono text-xs font-medium" title={file.path}>
                {file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}
              </span>
              <Badge tone={info.tone}>{info.label}</Badge>
              <span className="font-mono text-[11px] text-emerald-600 dark:text-emerald-400">+{added}</span>
              <span className="font-mono text-[11px] text-red-600 dark:text-red-400">-{removed}</span>
              {onOpenFile ? (
                <Button size="sm" variant="ghost" className="ml-auto" onClick={() => onOpenFile(file.path)}>
                  Abrir no editor
                </Button>
              ) : null}
            </div>
            {isCollapsed ? null : (
              <div className="divide-y divide-zinc-100 dark:divide-zinc-800/70">
                {file.binary ? (
                  <p className="flex items-center gap-2 px-3 py-3 text-xs text-amber-700 dark:text-amber-400">
                    <AlertTriangleIcon className="size-3.5" /> Arquivo binário: o git não gera diff linha a linha.
                  </p>
                ) : null}
                {file.hunks.length === 0 && !file.binary ? (
                  <p className="px-3 py-3 text-xs text-zinc-500 dark:text-zinc-400">
                    Nenhuma linha alterada — apenas metadados do arquivo mudaram.
                  </p>
                ) : null}
                {file.hunks.map((hunk, index) => (
                  <div key={`${hunk.header}-${index}`}>
                    <div className="flex items-center gap-2 bg-zinc-50/80 px-3 py-1 dark:bg-zinc-900/40">
                      <span className="truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400">
                        {hunk.header}
                      </span>
                      {onToggleHunks ? (
                        <Button
                          size="sm"
                          variant="ghost"
                          className="ml-auto shrink-0"
                          loading={applying === `${file.path}#${index}`}
                          onClick={() => void toggle(file.path, index)}
                          title="Preparar apenas este trecho para o commit"
                        >
                          Preparar trecho
                        </Button>
                      ) : null}
                    </div>
                    <HunkLines hunk={hunk} />
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
