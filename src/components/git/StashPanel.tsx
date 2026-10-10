import { useState } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import type { GitRepo } from "../../lib/useGitRepo";
import type { FileDiff, StashEntry } from "../../types";
import {
  ArchiveIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  PlusIcon,
  RefreshIcon,
  TrashIcon,
} from "../Icons";
import { Button, Card, Checkbox, Confirm, EmptyState, Field, Input, Spinner } from "../ui";
import { DiffView } from "./DiffView";

/**
 * Os trabalhos guardados pelo Git (`git stash`): o que está na mão sai da pasta,
 * fica endereçado pelo próprio git e volta quando você quiser. A lista, a ordem e
 * o conteúdo vêm do git de cada vez — nada aqui é simulado.
 */
export function StashPanel({ git, projectId }: { git: GitRepo; projectId: number }) {
  const { pushToast } = useApp();
  const busy = git.busy !== null;
  const [mensagem, setMensagem] = useState("");
  const [incluirNovos, setIncluirNovos] = useState(false);
  const [excluindo, setExcluindo] = useState<StashEntry | null>(null);
  const [aberto, setAberto] = useState<string | null>(null);
  const [diffs, setDiffs] = useState<Record<string, FileDiff[]>>({});
  const [carregando, setCarregando] = useState<string | null>(null);

  const semNadaNaMao = git.status === null || git.status.files.length === 0;

  const guardar = async () => {
    const feito = await git.stashPush(mensagem.trim() === "" ? null : mensagem.trim(), incluirNovos);
    if (feito !== null) setMensagem("");
  };

  /** O conteúdo do guardado é o diff do commit dele em relação à base — lido do
   * git com o mesmo parser do diff local. */
  const verConteudo = async (entrada: StashEntry) => {
    if (aberto === entrada.selector) {
      setAberto(null);
      return;
    }
    setAberto(entrada.selector);
    if (diffs[entrada.selector] !== undefined) return;
    setCarregando(entrada.selector);
    try {
      const files = await api.gitDiff({ projectId, commit: entrada.sha });
      setDiffs((atual) => ({ ...atual, [entrada.selector]: files }));
    } catch (e) {
      setAberto(null);
      pushToast("error", String(e));
    } finally {
      setCarregando(null);
    }
  };

  const excluir = async () => {
    if (excluindo === null) return;
    const selecionado = excluindo.selector;
    const feito = await git.stashDrop(excluindo.index);
    if (feito !== null) {
      setDiffs((atual) => {
        const resto = { ...atual };
        delete resto[selecionado];
        return resto;
      });
      setAberto(null);
      setExcluindo(null);
    }
  };

  return (
    <div className="space-y-3">
      <Card className="p-3">
        <div className="flex flex-wrap items-end gap-2">
          <div className="min-w-52 flex-1">
            <Field label="Nome do guardado (opcional)">
              <Input
                value={mensagem}
                onChange={(e) => setMensagem(e.target.value)}
                placeholder="ex.: teste do antes de trocar de ramo"
              />
            </Field>
          </div>
          <Checkbox checked={incluirNovos} onChange={setIncluirNovos} label="levar os arquivos novos" />
          <Button
            variant="primary"
            size="sm"
            disabled={busy || semNadaNaMao}
            title={semNadaNaMao ? "A pasta está limpa: não há o que guardar" : "Tirar da pasta e guardar"}
            onClick={() => void guardar()}
          >
            <PlusIcon className="size-3.5" /> Guardar
          </Button>
        </div>
      </Card>

      <Card className="p-2">
        <div className="flex items-center gap-2 px-2 pb-1 pt-1.5">
          <p className="flex-1 text-[11px] font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
            Guardados · {git.stash.length}
          </p>
          <Button size="sm" variant="ghost" disabled={busy} title="Reler a lista do git" onClick={() => void git.reloadStash()}>
            <RefreshIcon className="size-3.5" />
          </Button>
        </div>
        {git.stash.length === 0 ? (
          <EmptyState
            icon={<ArchiveIcon className="size-7" />}
            title="Nenhum trabalho guardado"
            description="Guarde o que está na mão para trocar de ramo ou puxar sem conflito. O guardado fica só nesta máquina."
          />
        ) : null}
        <ul className="space-y-1">
          {git.stash.map((entrada) => (
            <li key={entrada.selector} className="rounded-lg px-2 py-1.5 hover:bg-zinc-100/70 dark:hover:bg-zinc-800/50">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span className="font-mono text-[11px] text-zinc-400 dark:text-zinc-500">{entrada.selector}</span>
                <span className="min-w-0 flex-1 truncate text-xs text-zinc-700 dark:text-zinc-200" title={entrada.subject}>
                  {entrada.subject}
                </span>
                <span className="font-mono text-[11px] text-zinc-400 dark:text-zinc-500">{entrada.sha}</span>
                <span className="text-[11px] text-zinc-500 dark:text-zinc-400">{entrada.date.slice(0, 16)}</span>
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => void git.stashApply(entrada.index, false)}>
                  Aplicar
                </Button>
                <Button size="sm" disabled={busy} title="Aplicar e tirar da lista" onClick={() => void git.stashApply(entrada.index, true)}>
                  Aplicar e tirar
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  title={aberto === entrada.selector ? "Fechar o conteúdo" : "Ver o que há dentro, antes de aplicar"}
                  onClick={() => void verConteudo(entrada)}
                >
                  {aberto === entrada.selector ? (
                    <ChevronDownIcon className="size-3.5" />
                  ) : (
                    <ChevronRightIcon className="size-3.5" />
                  )}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  title="Excluir o guardado sem aplicar"
                  onClick={() => setExcluindo(entrada)}
                >
                  <TrashIcon className="size-3.5" />
                </Button>
              </div>
              {aberto === entrada.selector ? (
                carregando === entrada.selector ? (
                  <div className="flex items-center gap-2 px-1 py-3 text-xs text-zinc-500 dark:text-zinc-400">
                    <Spinner /> Lendo o conteúdo do guardado…
                  </div>
                ) : (
                  <div className="mt-2 border-t border-zinc-200/70 pt-2 dark:border-zinc-700/60">
                    <DiffView files={diffs[entrada.selector] ?? []} loading={false} emptyHint="O guardado não tem mudança nenhuma." />
                  </div>
                )
              ) : null}
            </li>
          ))}
        </ul>
      </Card>

      <Confirm
        open={excluindo !== null}
        title={`Excluir ${excluindo?.selector ?? ""}`}
        message={
          excluido(excluindo) +
          " Nada é aplicado na pasta antes: o conteúdo deste guardado sai da lista e a aplicação não o recupera."
        }
        confirmLabel="Excluir"
        busy={busy}
        onConfirm={() => void excluir()}
        onClose={() => setExcluindo(null)}
      />
    </div>
  );
}

function excluido(entrada: StashEntry | null) {
  if (entrada === null) return "";
  return `Será excluído o guardado "${entrada.subject}" (${entrada.sha}), sem aplicá-lo.`;
}
