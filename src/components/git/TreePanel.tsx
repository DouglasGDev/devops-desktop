import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import type { FileContent, RepoStatus, StatusFile } from "../../types";
import { KIND_TONES } from "./ChangesPanel";
import { ChevronDownIcon, ChevronRightIcon, ExternalLinkIcon, FileIcon, FolderIcon, RefreshIcon } from "../Icons";
import { Badge, Button, SectionTitle, Spinner } from "../ui";

type Pasta = {
  nome: string;
  caminho: string;
  pastas: Map<string, Pasta>;
  arquivos: { nome: string; caminho: string }[];
};

function novaPasta(nome: string, caminho: string): Pasta {
  return { nome, caminho, pastas: new Map(), arquivos: [] };
}

/** Árvore a partir dos caminhos relativos que o git devolve, sem caminhar no disco. */
function montarArvore(caminhos: string[]): Pasta {
  const raiz = novaPasta("", "");
  for (const caminho of caminhos) {
    const partes = caminho.split("/");
    let atual = raiz;
    for (let i = 0; i < partes.length - 1; i += 1) {
      const nome = partes[i];
      let filho = atual.pastas.get(nome);
      if (!filho) {
        filho = novaPasta(nome, partes.slice(0, i + 1).join("/"));
        atual.pastas.set(nome, filho);
      }
      atual = filho;
    }
    atual.arquivos.push({ nome: partes[partes.length - 1], caminho });
  }
  return raiz;
}

export function TreePanel({
  projectId,
  status,
  busy,
  selectedPath,
  onSelecionarArquivo,
  onConteudo,
}: {
  projectId: number;
  status: RepoStatus | null;
  busy: boolean;
  selectedPath: string | null;
  onSelecionarArquivo: (path: string, staged: boolean) => void;
  onConteudo: (arquivo: FileContent) => void;
}) {
  const { pushToast } = useApp();
  const [caminhos, setCaminhos] = useState<string[] | null>(null);
  const [erro, setErro] = useState<string | null>(null);
  const [abertas, setAbertas] = useState<Set<string>>(new Set());
  const [lendo, setLendo] = useState<string | null>(null);

  const ler = useCallback(async () => {
    setErro(null);
    try {
      setCaminhos(await api.gitTree(projectId));
    } catch (e) {
      setCaminhos(null);
      setErro(String(e));
    }
  }, [projectId]);

  // O índice muda a cada stage, commit ou troca de ramo feito aqui, então a
  // árvore é relida junto com o status: o que aparece é o repositório agora.
  useEffect(() => {
    void ler();
    setAbertas(new Set());
  }, [ler, status]);

  const porPath = useMemo(() => {
    const mapa = new Map<string, StatusFile>();
    for (const f of status?.files ?? []) mapa.set(f.path, f);
    return mapa;
  }, [status]);

  const arvore = useMemo(() => {
    const todos = new Set(caminhos ?? []);
    for (const f of status?.files ?? []) todos.add(f.path);
    return montarArvore([...todos].sort((a, b) => a.localeCompare(b)));
  }, [caminhos, status]);

  const alternar = (caminho: string) =>
    setAbertas((atual) => {
      const proxo = new Set(atual);
      if (proxo.has(caminho)) proxo.delete(caminho);
      else proxo.add(caminho);
      return proxo;
    });

  const abrirConteudo = async (path: string) => {
    setLendo(path);
    try {
      onConteudo(await api.gitFileContent(projectId, path));
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setLendo(null);
    }
  };

  // O clique mostra o diff quando o arquivo tem diferença em algum lado; sem
  // mudança pendente, o que há para ver é o conteúdo que o Git guarda no HEAD.
  const clicarArquivo = (path: string) => {
    const f = porPath.get(path);
    if (f?.conflict || f?.unstaged) onSelecionarArquivo(path, false);
    else if (f?.staged) onSelecionarArquivo(path, true);
    else void abrirConteudo(path);
  };

  const renderizar = (pasta: Pasta, nivel: number): ReactNode => {
    const abertasAqui = nivel === 0 || abertas.has(pasta.caminho);
    const pastas = [...pasta.pastas.values()].sort((a, b) => a.nome.localeCompare(b.nome));
    return (
      <div key={pasta.caminho || "#raiz"}>
        {nivel > 0 ? (
          <button
            type="button"
            onClick={() => alternar(pasta.caminho)}
            className="flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-left hover:bg-zinc-100 dark:hover:bg-zinc-800/60"
            style={{ paddingLeft: `${nivel * 12}px` }}
          >
            {abertasAqui ? (
              <ChevronDownIcon className="size-3 shrink-0 text-zinc-400" />
            ) : (
              <ChevronRightIcon className="size-3 shrink-0 text-zinc-400" />
            )}
            <FolderIcon className="size-3.5 shrink-0 text-teal-600 dark:text-teal-400" />
            <span className="truncate text-xs">{pasta.nome}</span>
            <span className="ml-auto shrink-0 text-[10px] text-zinc-400">{pastas.length + pasta.arquivos.length}</span>
          </button>
        ) : null}
        {abertasAqui ? (
          <>
            {pastas.map((p) => renderizar(p, nivel + 1))}
            {pasta.arquivos.map((a) => {
              const f = porPath.get(a.caminho);
              const destacada = selectedPath === a.caminho;
              return (
                <div
                  key={a.caminho}
                  className={`flex items-center gap-1.5 rounded pr-1 py-0.5 hover:bg-zinc-100 dark:hover:bg-zinc-800/60 ${
                    destacada ? "bg-teal-500/10" : ""
                  }`}
                  style={{ paddingLeft: `${(nivel + 1) * 12 + 14}px` }}
                >
                  <FileIcon className="size-3.5 shrink-0 text-zinc-400" />
                  <button type="button" onClick={() => clicarArquivo(a.caminho)} className="min-w-0 flex-1 truncate text-left text-xs">
                    {a.nome}
                  </button>
                  {f ? <Badge tone={KIND_TONES[f.kind] ?? "neutral"}>{f.kind}</Badge> : null}
                  <button
                    type="button"
                    title="Ver o conteúdo que o Git guarda no HEAD"
                    className="shrink-0 text-zinc-400 hover:text-teal-600 dark:hover:text-teal-400"
                    onClick={() => void abrirConteudo(a.caminho)}
                  >
                    {lendo === a.caminho ? <Spinner className="size-3" /> : <FileIcon className="size-3" />}
                  </button>
                </div>
              );
            })}
          </>
        ) : null}
      </div>
    );
  };

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <SectionTitle>Árvore do repositório</SectionTitle>
        {caminhos !== null ? (
          <span className="text-[11px] text-zinc-500 dark:text-zinc-400">{caminhos.length} rastreado(s)</span>
        ) : null}
        <Button size="sm" variant="ghost" className="ml-auto" disabled={busy} onClick={() => void ler()} title="Reler o índice do git">
          <RefreshIcon className="size-3" />
        </Button>
      </div>
      {erro ? (
        <p className="text-xs text-red-600 dark:text-red-400">{erro}</p>
      ) : caminhos === null ? (
        <p className="flex items-center gap-2 py-4 text-xs text-zinc-500 dark:text-zinc-400">
          <Spinner className="size-3" /> lendo o índice do git…
        </p>
      ) : caminhos.length === 0 && (status?.files.length ?? 0) === 0 ? (
        <p className="py-4 text-xs text-zinc-500 dark:text-zinc-400">
          O índice está vazio — nada rastreado ainda. O primeiro arquivo aparece em Alterações.
        </p>
      ) : (
        renderizar(arvore, 0)
      )}
    </div>
  );
}

export function ConteudoArquivo({
  arquivo,
  onFechar,
  onAbrirNoEditor,
}: {
  arquivo: FileContent;
  onFechar: () => void;
  onAbrirNoEditor: (path: string) => void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <SectionTitle>Conteúdo no HEAD</SectionTitle>
        <span className="min-w-0 truncate font-mono text-[11px] text-zinc-500 dark:text-zinc-400" title={arquivo.path}>
          {arquivo.path}
        </span>
        <Button size="sm" variant="ghost" className="ml-auto" onClick={() => onAbrirNoEditor(arquivo.path)}>
          <ExternalLinkIcon className="size-3.5" /> Abrir no editor
        </Button>
        <Button size="sm" variant="ghost" onClick={onFechar} title="Fechar o conteúdo">
          <FileIcon className="size-3.5" /> Fechar
        </Button>
      </div>
      {arquivo.note ? (
        <p className="rounded-lg bg-amber-500/10 px-2.5 py-1.5 text-xs text-amber-700 dark:text-amber-300">{arquivo.note}</p>
      ) : null}
      {arquivo.text !== null ? (
        <pre className="min-h-0 flex-1 overflow-auto rounded-lg bg-zinc-100 p-2.5 font-mono text-[11px] leading-relaxed whitespace-pre text-zinc-700 dark:bg-zinc-900 dark:text-zinc-300">
          {arquivo.text === "" ? "(arquivo vazio)" : arquivo.text}
        </pre>
      ) : null}
    </div>
  );
}
