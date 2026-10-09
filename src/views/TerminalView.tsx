import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { api, onTerminalExit, onTerminalOutput } from "../lib/api";
import { useApp } from "../store/AppContext";
import type { Routine, RoutineParam, Server, TerminalHistoryEntry, TerminalSessionInfo, ViewId } from "../types";
import {
  AlertTriangleIcon,
  FolderIcon,
  PencilIcon,
  PlusIcon,
  RefreshIcon,
  TerminalIcon,
  TrashIcon,
  XIcon,
} from "../components/Icons";
import {
  Badge,
  Button,
  Card,
  Confirm,
  EmptyState,
  ENV_LABELS,
  ENV_TONES,
  Field,
  Input,
  Modal,
  Select,
  StatusDot,
  Textarea,
} from "../components/ui";

interface TabTerminal {
  terminal: Terminal;
  fit: FitAddon;
}

interface EditorState {
  id: number | null;
  name: string;
  description: string;
  command: string;
  danger: "read" | "write";
  params: RoutineParam[];
}

function xtermTheme(dark: boolean) {
  return dark
    ? { background: "#09090b", foreground: "#e4e4e7", cursor: "#5eead4", selectionBackground: "#134e4a" }
    : { background: "#ffffff", foreground: "#18181b", cursor: "#0f766e", selectionBackground: "#99f6e4" };
}

function resolveTemplate(command: string, values: Record<string, string>) {
  return command.replace(/\{\{([\w-]+)\}\}/g, (_, name) => values[name] ?? "");
}

function timeOf(ts: string) {
  const d = new Date(ts);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" });
}

export function TerminalView({ active, onNavigate }: { active: boolean; onNavigate: (v: ViewId) => void }) {
  const { servers, settings, theme, pushToast } = useApp();
  const isDark = theme === "dark";
  const confirmDestructive = settings.confirm_destructive !== "false";
  const prodWarnings = settings.prod_warnings !== "false";
  const historyEnabled = settings.terminal_history_enabled !== "false";

  const [sessions, setSessions] = useState<TerminalSessionInfo[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [selectedServerId, setSelectedServerId] = useState<number | null>(null);
  const [opening, setOpening] = useState(false);
  const [routines, setRoutines] = useState<Routine[]>([]);
  const [history, setHistory] = useState<TerminalHistoryEntry[]>([]);
  const [quickInput, setQuickInput] = useState("");

  const [prodConfirm, setProdConfirm] = useState<Server | null>(null);
  const [closingSession, setClosingSession] = useState<TerminalSessionInfo | null>(null);
  const [confirmRun, setConfirmRun] = useState<{ command: string; routineName: string; session: TerminalSessionInfo } | null>(null);
  const [runState, setRunState] = useState<{ routine: Routine; values: Record<string, string> } | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Routine | null>(null);
  const [confirmClearHistory, setConfirmClearHistory] = useState(false);

  const terms = useRef<Map<number, TabTerminal>>(new Map());
  const containerRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const pendingOut = useRef<Map<number, string[]>>(new Map());
  const cmdBuffers = useRef<Map<number, string>>(new Map());
  const activeIdRef = useRef<number | null>(null);
  const areaRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);

  const refreshHistory = useCallback(async () => {
    try {
      setHistory(await api.listTerminalHistory(100));
    } catch {
      // painel de histórico é secundário; falha não interrompe o terminal
    }
  }, []);

  const loadRoutines = useCallback(async () => {
    try {
      setRoutines(await api.listRoutines());
    } catch (e) {
      pushToast("error", `Falha ao carregar rotinas: ${String(e)}`);
    }
  }, [pushToast]);

  useEffect(() => {
    void loadRoutines();
    void refreshHistory();
    // Restaura abas de sessões que continuam vivas no backend (ex.: recarga do frontend).
    void api
      .listTerminalSessions()
      .then((list) => {
        if (list.length > 0) {
          setSessions(list);
          setActiveId((prev) => prev ?? list[list.length - 1].id);
        }
      })
      .catch(() => {});
  }, [loadRoutines, refreshHistory]);

  useEffect(() => {
    if (servers.length === 0) {
      setSelectedServerId(null);
      return;
    }
    if (selectedServerId === null || !servers.some((s) => s.id === selectedServerId)) {
      setSelectedServerId(servers[0].id);
    }
  }, [servers, selectedServerId]);

  const trackCommand = (sessionId: number, data: string) => {
    let buf = cmdBuffers.current.get(sessionId) ?? "";
    for (const ch of data) {
      if (ch === "\r" || ch === "\n") {
        const cmd = buf.trim();
        buf = "";
        if (cmd) {
          void api
            .recordTerminalCommand(sessionId, cmd)
            .then(() => refreshHistory())
            .catch(() => {});
        }
      } else if (ch === "\u007f") {
        buf = buf.slice(0, -1);
      } else if (ch === "\u0003") {
        buf = "";
      } else if (ch >= " ") {
        buf = (buf + ch).slice(-5000);
      }
    }
    cmdBuffers.current.set(sessionId, buf);
  };

  const handleWrite = async (sessionId: number, data: string) => {
    try {
      await api.writeTerminalSession(sessionId, data);
    } catch {
      // sessão encerrada entre o toque e a escrita; o evento exit atualiza a aba
    }
    trackCommand(sessionId, data);
  };

  const copyPasteHandler = (sessionId: number, terminal: Terminal, ev: KeyboardEvent): boolean => {
    if (ev.type !== "keydown") return true;
    if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "c") {
      const sel = terminal.getSelection();
      if (sel) {
        void navigator.clipboard.writeText(sel).catch(() => {});
        terminal.clearSelection();
      }
      return false;
    }
    if (ev.ctrlKey && ev.shiftKey && ev.key.toLowerCase() === "v") {
      void navigator.clipboard
        .readText()
        .then((text) => {
          if (text) void handleWrite(sessionId, text);
        })
        .catch(() => pushToast("error", "Não foi possível acessar a área de transferência."));
      return false;
    }
    return true;
  };

  // Cria a instância xterm de cada sessão nova assim que o container existe no DOM.
  useEffect(() => {
    for (const s of sessions) {
      if (terms.current.has(s.id)) continue;
      const div = containerRefs.current.get(s.id);
      if (!div) continue;
      const terminal = new Terminal({
        fontSize: 13,
        fontFamily: '"Cascadia Mono", Consolas, "Courier New", monospace',
        cursorBlink: true,
        scrollback: 5000,
        theme: xtermTheme(isDark),
      });
      const fit = new FitAddon();
      terminal.loadAddon(fit);
      terminal.open(div);
      terminal.attachCustomKeyEventHandler((ev) => copyPasteHandler(s.id, terminal, ev));
      terminal.onData((data) => void handleWrite(s.id, data));
      terms.current.set(s.id, { terminal, fit });
      const pending = pendingOut.current.get(s.id);
      if (pending) {
        for (const chunk of pending) terminal.write(chunk);
        pendingOut.current.delete(s.id);
      }
      if (s.id === activeId) {
        try {
          fit.fit();
          void api.resizeTerminalSession(s.id, terminal.cols, terminal.rows).catch(() => {});
        } catch {
          // container oculto; o ajuste acontece quando a aba fica visível
        }
      }
      terminal.focus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions, activeId, isDark]);

  // Descarta as instâncias xterm apenas quando a view é destruída.
  useEffect(() => {
    return () => {
      for (const { terminal } of terms.current.values()) terminal.dispose();
      terms.current.clear();
      containerRefs.current.clear();
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unOutput: (() => void) | undefined;
    let unExit: (() => void) | undefined;
    void (async () => {
      const stopOutput = await onTerminalOutput((e) => {
        const t = terms.current.get(e.sessionId);
        if (t) {
          t.terminal.write(e.data);
        } else {
          const list = pendingOut.current.get(e.sessionId) ?? [];
          list.push(e.data);
          pendingOut.current.set(e.sessionId, list);
        }
      });
      const stopExit = await onTerminalExit((e) => {
        const t = terms.current.get(e.sessionId);
        if (t) {
          t.terminal.write(
            `\r\n\x1b[90m── sessão encerrada${e.exitCode !== null ? ` (código ${e.exitCode})` : ""} ──\x1b[0m\r\n`,
          );
        }
        setSessions((prev) =>
          prev.map((s) => (s.id === e.sessionId ? { ...s, alive: false, exitCode: e.exitCode } : s)),
        );
      });
      if (disposed) {
        stopOutput();
        stopExit();
        return;
      }
      unOutput = stopOutput;
      unExit = stopExit;
    })();
    return () => {
      disposed = true;
      unOutput?.();
      unExit?.();
    };
  }, []);

  useEffect(() => {
    for (const { terminal } of terms.current.values()) {
      terminal.options.theme = xtermTheme(isDark);
    }
  }, [isDark]);

  const fitActive = useCallback(() => {
    const id = activeIdRef.current;
    if (id === null) return;
    const t = terms.current.get(id);
    if (!t) return;
    try {
      t.fit.fit();
      void api.resizeTerminalSession(id, t.terminal.cols, t.terminal.rows).catch(() => {});
    } catch {
      // aba oculta; ajusta ao ficar visível
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    const area = areaRef.current;
    if (!area) return;
    const ro = new ResizeObserver(() => fitActive());
    ro.observe(area);
    fitActive();
    return () => ro.disconnect();
  }, [active, activeId, sessions, fitActive]);

  const continueOpen = async (serverId: number) => {
    if (opening) return;
    setOpening(true);
    try {
      const info = await api.openTerminalSession(serverId, 100, 28);
      setSessions((prev) => [...prev, info]);
      setActiveId(info.id);
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setOpening(false);
    }
  };

  const openTab = (serverId: number) => {
    const server = servers.find((s) => s.id === serverId);
    if (!server) return;
    if (server.environment === "prod" && prodWarnings) {
      setProdConfirm(server);
      return;
    }
    void continueOpen(serverId);
  };

  const doClose = async (id: number) => {
    const t = terms.current.get(id);
    if (t) {
      t.terminal.dispose();
      terms.current.delete(id);
    }
    containerRefs.current.delete(id);
    cmdBuffers.current.delete(id);
    pendingOut.current.delete(id);
    const remaining = sessions.filter((s) => s.id !== id);
    setSessions(remaining);
    if (activeId === id) setActiveId(remaining.length > 0 ? remaining[remaining.length - 1].id : null);
    try {
      await api.closeTerminalSession(id);
    } catch {
      // sessão já encerrada no backend
    }
  };

  const requestClose = (s: TerminalSessionInfo) => {
    if (s.alive && confirmDestructive) {
      setClosingSession(s);
      return;
    }
    void doClose(s.id);
  };

  const reconnect = (s: TerminalSessionInfo) => {
    // abre uma sessão nova; nada é reenviado automaticamente
    void continueOpen(s.serverId);
  };

  const executeCommand = async (sessionId: number, command: string) => {
    try {
      await api.writeTerminalSession(sessionId, command + "\n");
      await api.recordTerminalCommand(sessionId, command);
      await refreshHistory();
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const sendRoutine = (routine: Routine, values: Record<string, string>, session: TerminalSessionInfo) => {
    const command = resolveTemplate(routine.command, values).trim();
    if (!command) {
      pushToast("error", "Preencha os parâmetros da rotina.");
      return;
    }
    if (routine.danger === "write") {
      setConfirmRun({ command, routineName: routine.name, session });
      return;
    }
    void executeCommand(session.id, command);
  };

  const startRoutine = (routine: Routine) => {
    const session = sessions.find((s) => s.id === activeId && s.alive);
    if (!session) {
      pushToast("error", "Abra uma sessão de terminal ativa primeiro.");
      return;
    }
    if (routine.params.length === 0) {
      sendRoutine(routine, {}, session);
      return;
    }
    const defaults: Record<string, string> = {};
    for (const p of routine.params) defaults[p.name] = p.default ?? "";
    setRunState({ routine, values: defaults });
  };

  const saveRoutineFromEditor = async () => {
    if (!editor) return;
    try {
      await api.saveRoutine({
        id: editor.id,
        name: editor.name,
        description: editor.description.trim() || null,
        command: editor.command,
        danger: editor.danger,
        params: editor.params,
      });
      pushToast("success", editor.id ? "Rotina atualizada." : "Rotina criada.");
      setEditor(null);
      await loadRoutines();
    } catch (e) {
      pushToast("error", String(e));
    }
  };

  const sendQuick = (ev: FormEvent) => {
    ev.preventDefault();
    const session = sessions.find((s) => s.id === activeId && s.alive);
    const cmd = quickInput.trim();
    if (!session || !cmd) return;
    setQuickInput("");
    void executeCommand(session.id, cmd);
  };

  const activeSession = sessions.find((s) => s.id === activeId) ?? null;
  const activeAlive = activeSession?.alive === true;

  const emptyEditor = (): EditorState => ({
    id: null,
    name: "",
    description: "",
    command: "",
    danger: "read",
    params: [],
  });

  const fromRoutine = (r: Routine): EditorState => ({
    id: r.id,
    name: r.name,
    description: r.description ?? "",
    command: r.command,
    danger: r.danger,
    params: r.params.map((p) => ({ ...p })),
  });

  const updateParam = (index: number, patch: Partial<RoutineParam>) => {
    if (!editor) return;
    setEditor({
      ...editor,
      params: editor.params.map((p, i) => (i === index ? { ...p, ...patch } : p)),
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          className="max-w-xs"
          value={selectedServerId ?? ""}
          onChange={(e) => setSelectedServerId(e.target.value ? Number(e.target.value) : null)}
        >
          <option value="" disabled>
            Selecione um servidor…
          </option>
          {servers.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} · {ENV_LABELS[s.environment] ?? s.environment}
            </option>
          ))}
        </Select>
        <Button
          variant="primary"
          size="sm"
          disabled={selectedServerId === null || opening}
          loading={opening}
          onClick={() => selectedServerId !== null && openTab(selectedServerId)}
        >
          <PlusIcon className="size-3.5" /> Nova aba
        </Button>
        {activeSession ? (
          <span className="ml-auto flex flex-wrap items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400">
            <StatusDot status={activeSession.alive ? "ok" : "failed"} />
            <span className="font-mono">
              {activeSession.username}@{activeSession.host}:{activeSession.port}
            </span>
            <Badge tone={ENV_TONES[activeSession.environment] ?? "neutral"}>
              {ENV_LABELS[activeSession.environment] ?? activeSession.environment}
            </Badge>
            {activeSession.alive ? (
              <Badge tone="success">conectado</Badge>
            ) : (
              <Badge tone="error">
                encerrado{activeSession.exitCode !== null ? ` (código ${activeSession.exitCode})` : ""}
              </Badge>
            )}
          </span>
        ) : null}
      </div>

      {sessions.length > 0 ? (
        <div className="flex items-center gap-1 overflow-x-auto rounded-lg border border-zinc-200 bg-zinc-50 p-1 dark:border-zinc-800 dark:bg-zinc-900/60">
          {sessions.map((s) => (
            <div
              key={s.id}
              className={`flex shrink-0 items-center rounded-md ${
                s.id === activeId
                  ? "bg-white shadow-sm dark:bg-zinc-800"
                  : "hover:bg-zinc-200/60 dark:hover:bg-zinc-800/60"
              }`}
            >
              <button
                onClick={() => setActiveId(s.id)}
                className="flex items-center gap-1.5 rounded-l-md px-2.5 py-1 text-xs"
                title={`${s.username}@${s.host}:${s.port} · ${ENV_LABELS[s.environment] ?? s.environment}`}
              >
                <StatusDot status={s.alive ? "ok" : "failed"} />
                <span className={s.id === activeId ? "font-medium" : ""}>{s.serverName}</span>
                {!s.alive ? <span className="text-zinc-400 dark:text-zinc-500">encerrado</span> : null}
              </button>
              {!s.alive ? (
                <button
                  onClick={() => reconnect(s)}
                  title="Reconectar (abre uma nova sessão, sem repetir comandos)"
                  className="p-1 text-zinc-400 transition hover:text-teal-600 dark:hover:text-teal-400"
                >
                  <RefreshIcon className="size-3" />
                </button>
              ) : null}
              <button
                onClick={() => requestClose(s)}
                title="Fechar aba"
                className="rounded-r-md p-1 text-zinc-400 transition hover:text-red-500"
              >
                <XIcon className="size-3.5" />
              </button>
            </div>
          ))}
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 gap-3">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
          <div
            ref={areaRef}
            className="relative min-h-0 flex-1 overflow-hidden rounded-lg border border-zinc-200 bg-white p-1 dark:border-zinc-800 dark:bg-zinc-950"
          >
            {sessions.length === 0 ? (
              servers.length === 0 ? (
                <EmptyState
                  icon={<FolderIcon className="size-10" />}
                  title="Nenhum servidor cadastrado"
                  description="O terminal abre sessões SSH nos servidores cadastrados. Importe seus acessos existentes de ~/.ssh ou cadastre um servidor manualmente."
                >
                  <Button size="sm" variant="primary" onClick={() => onNavigate("servers")}>
                    Ir para Servidores
                  </Button>
                </EmptyState>
              ) : (
                <EmptyState
                  icon={<TerminalIcon className="size-10" />}
                  title="Nenhuma sessão aberta"
                  description="Selecione um servidor acima e clique em “Nova aba”. A verificação de chave de host e o login acontecem dentro do próprio terminal — nada é aceito automaticamente."
                />
              )
            ) : (
              sessions.map((s) => (
                <div
                  key={s.id}
                  ref={(el) => {
                    if (el) containerRefs.current.set(s.id, el);
                    else containerRefs.current.delete(s.id);
                  }}
                  className={`absolute inset-0 ${s.id === activeId ? "" : "hidden"}`}
                />
              ))
            )}
          </div>
          <form onSubmit={sendQuick} className="flex items-center gap-2">
            <Input
              value={quickInput}
              onChange={(e) => setQuickInput(e.target.value)}
              disabled={!activeAlive}
              placeholder={
                activeAlive ? "Comando rápido — Enter envia ao servidor" : "Abra uma sessão para enviar comandos"
              }
              className="font-mono text-xs"
            />
          </form>
        </div>

        <div className="flex w-72 shrink-0 flex-col gap-3 overflow-y-auto">
          <Card className="flex min-h-0 flex-1 flex-col p-3">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">Rotinas</h3>
              <Button size="sm" onClick={() => setEditor(emptyEditor())}>
                <PlusIcon className="size-3" /> Nova
              </Button>
            </div>
            <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto">
              {routines.length === 0 ? (
                <li className="px-1 text-xs text-zinc-500 dark:text-zinc-400">Nenhuma rotina salva.</li>
              ) : (
                routines.map((r) => (
                  <li
                    key={r.id}
                    className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                  >
                    <button
                      className="min-w-0 flex-1 text-left"
                      onClick={() => startRoutine(r)}
                      title={r.description ?? r.command}
                    >
                      <span className="block truncate text-xs font-medium">{r.name}</span>
                      <span className="block truncate font-mono text-[10px] text-zinc-500 dark:text-zinc-400">
                        {r.command}
                      </span>
                    </button>
                    <Badge tone={r.danger === "read" ? "neutral" : "warning"}>
                      {r.danger === "read" ? "leitura" : "altera"}
                    </Badge>
                    {!r.isBuiltin ? (
                      <>
                        <button
                          onClick={() => setEditor(fromRoutine(r))}
                          title="Editar rotina"
                          className="p-0.5 text-zinc-400 transition hover:text-teal-600 dark:hover:text-teal-400"
                        >
                          <PencilIcon className="size-3" />
                        </button>
                        <button
                          onClick={() => setDeleteTarget(r)}
                          title="Excluir rotina"
                          className="p-0.5 text-zinc-400 transition hover:text-red-500"
                        >
                          <TrashIcon className="size-3" />
                        </button>
                      </>
                    ) : (
                      <span className="text-[10px] text-zinc-400 dark:text-zinc-500" title="Rotina pré-instalada">
                        padrão
                      </span>
                    )}
                  </li>
                ))
              )}
            </ul>
            <p className="mt-2 text-[10px] leading-relaxed text-zinc-500 dark:text-zinc-400">
              Rotinas de leitura são enviadas direto. Rotinas que alteram o servidor pedem confirmação antes de rodar.
            </p>
          </Card>

          <Card className="flex min-h-0 flex-1 flex-col p-3">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">
                Histórico de comandos
              </h3>
              {history.length > 0 ? (
                <Button size="sm" variant="ghost" onClick={() => setConfirmClearHistory(true)}>
                  <TrashIcon className="size-3" /> Limpar
                </Button>
              ) : null}
            </div>
            {!historyEnabled ? (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">
                O histórico está desativado — pode ser reativado em Configurações.
              </p>
            ) : (
              <ul className="min-h-0 flex-1 space-y-1 overflow-y-auto">
                {history.length === 0 ? (
                  <li className="px-1 text-xs text-zinc-500 dark:text-zinc-400">Nenhum comando registrado ainda.</li>
                ) : (
                  history.map((h) => (
                    <li key={h.id}>
                      <button
                        className="w-full rounded-lg px-2 py-1 text-left hover:bg-zinc-100 dark:hover:bg-zinc-800"
                        onClick={() => setQuickInput(h.command)}
                        title="Clique para preencher o campo de comando rápido"
                      >
                        <span className="block truncate font-mono text-[11px]">{h.command}</span>
                        <span className="block text-[10px] text-zinc-500 dark:text-zinc-400">
                          {timeOf(h.ts)}
                          {h.serverName ? ` · ${h.serverName}` : ""}
                        </span>
                      </button>
                    </li>
                  ))
                )}
              </ul>
            )}
          </Card>
        </div>
      </div>

      <Confirm
        open={prodConfirm !== null}
        title="Terminal em servidor de produção"
        message={
          prodConfirm
            ? `Você está prestes a abrir um terminal no servidor de PRODUÇÃO “${prodConfirm.name}” (${prodConfirm.username}@${prodConfirm.host}). Comandos executados nele afetam o servidor real imediatamente.`
            : ""
        }
        confirmLabel="Abrir terminal"
        onConfirm={() => {
          const server = prodConfirm;
          setProdConfirm(null);
          if (server) void continueOpen(server.id);
        }}
        onClose={() => setProdConfirm(null)}
      />

      <Confirm
        open={closingSession !== null}
        title="Fechar sessão ativa"
        message={
          closingSession
            ? `A sessão com “${closingSession.serverName}” ainda está conectada. Fechar a aba encerra o ssh no servidor e interrompe qualquer comando em execução.`
            : ""
        }
        confirmLabel="Encerrar sessão"
        onConfirm={() => {
          const s = closingSession;
          setClosingSession(null);
          if (s) void doClose(s.id);
        }}
        onClose={() => setClosingSession(null)}
      />

      <Confirm
        open={confirmRun !== null}
        title="Executar rotina que altera o servidor"
        message={
          confirmRun
            ? `${
                confirmRun.session.environment === "prod" && prodWarnings
                  ? "ATENÇÃO: servidor de PRODUÇÃO.\n\n"
                  : ""
              }A rotina “${confirmRun.routineName}” altera o estado do servidor ${confirmRun.session.serverName}. Comando:\n${confirmRun.command}`
            : ""
        }
        confirmLabel="Executar"
        onConfirm={() => {
          const run = confirmRun;
          setConfirmRun(null);
          if (run && run.session.alive) void executeCommand(run.session.id, run.command);
        }}
        onClose={() => setConfirmRun(null)}
      />

      <Confirm
        open={deleteTarget !== null}
        title="Excluir rotina"
        message={deleteTarget ? `Excluir a rotina “${deleteTarget.name}”? Esta ação não pode ser desfeita.` : ""}
        confirmLabel="Excluir"
        onConfirm={() => {
          const r = deleteTarget;
          setDeleteTarget(null);
          if (r) {
            void api
              .deleteRoutine(r.id)
              .then(() => loadRoutines())
              .then(() => pushToast("success", "Rotina excluída."))
              .catch((e) => pushToast("error", String(e)));
          }
        }}
        onClose={() => setDeleteTarget(null)}
      />

      <Confirm
        open={confirmClearHistory}
        title="Limpar histórico de comandos"
        message="Apagar todos os comandos registrados localmente? Os registros de auditoria de sessão não são afetados."
        confirmLabel="Limpar"
        onConfirm={() => {
          setConfirmClearHistory(false);
          void api
            .clearTerminalHistory()
            .then(() => refreshHistory())
            .then(() => pushToast("success", "Histórico limpo."))
            .catch((e) => pushToast("error", String(e)));
        }}
        onClose={() => setConfirmClearHistory(false)}
      />

      <Modal
        open={runState !== null}
        onClose={() => setRunState(null)}
        title={runState ? `Rotina: ${runState.routine.name}` : ""}
      >
        {runState ? (
          <div className="space-y-3">
            {runState.routine.description ? (
              <p className="text-xs text-zinc-500 dark:text-zinc-400">{runState.routine.description}</p>
            ) : null}
            {runState.routine.params.map((p) => (
              <Field key={p.name} label={p.label ?? p.name} hint={p.required ? "obrigatório" : "opcional"}>
                <Input
                  value={runState.values[p.name] ?? ""}
                  onChange={(e) =>
                    setRunState({ ...runState, values: { ...runState.values, [p.name]: e.target.value } })
                  }
                  placeholder={p.default ?? ""}
                  className="font-mono text-xs"
                />
              </Field>
            ))}
            <div>
              <p className="mb-1 text-xs font-medium text-zinc-500 dark:text-zinc-400">Comando que será executado:</p>
              <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded-lg bg-zinc-100 px-3 py-2 font-mono text-xs dark:bg-zinc-800">
                {resolveTemplate(runState.routine.command, runState.values)}
              </pre>
            </div>
            {runState.routine.danger === "write" ? (
              <p className="flex items-start gap-1.5 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-300">
                <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />
                Esta rotina altera o estado do servidor e pedirá uma confirmação antes de ser enviada.
              </p>
            ) : null}
            <div className="flex justify-end gap-2">
              <Button onClick={() => setRunState(null)}>Cancelar</Button>
              <Button
                variant={runState.routine.danger === "write" ? "danger" : "primary"}
                onClick={() => {
                  const missing = runState.routine.params.filter(
                    (p) => p.required && !(runState.values[p.name] ?? "").trim(),
                  );
                  if (missing.length > 0) {
                    pushToast("error", `Preencha: ${missing.map((p) => p.label ?? p.name).join(", ")}`);
                    return;
                  }
                  const session = sessions.find((s) => s.id === activeId && s.alive);
                  if (!session) {
                    pushToast("error", "A sessão ativa foi encerrada.");
                    setRunState(null);
                    return;
                  }
                  sendRoutine(runState.routine, runState.values, session);
                  setRunState(null);
                }}
              >
                Executar
              </Button>
            </div>
          </div>
        ) : null}
      </Modal>

      <Modal
        open={editor !== null}
        onClose={() => setEditor(null)}
        title={editor?.id ? "Editar rotina" : "Nova rotina"}
        width="max-w-2xl"
      >
        {editor ? (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <Field label="Nome">
                <Input value={editor.name} onChange={(e) => setEditor({ ...editor, name: e.target.value })} />
              </Field>
              <Field label="Classificação" hint="Rotinas de alteração pedem confirmação antes de rodar.">
                <Select
                  value={editor.danger}
                  onChange={(e) => setEditor({ ...editor, danger: e.target.value as "read" | "write" })}
                >
                  <option value="read">Somente leitura</option>
                  <option value="write">Altera o servidor</option>
                </Select>
              </Field>
            </div>
            <Field label="Descrição">
              <Input value={editor.description} onChange={(e) => setEditor({ ...editor, description: e.target.value })} />
            </Field>
            <Field label="Comando" hint="Use {{nome}} para inserir parâmetros. Ex.: tail -n 100 {{arquivo}}">
              <Textarea
                rows={3}
                value={editor.command}
                onChange={(e) => setEditor({ ...editor, command: e.target.value })}
                className="font-mono text-xs"
              />
            </Field>
            <div>
              <p className="mb-1 text-xs font-medium text-zinc-500 dark:text-zinc-400">Parâmetros</p>
              <div className="space-y-2">
                {editor.params.map((p, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <Input
                      className="font-mono text-xs"
                      placeholder="nome"
                      value={p.name}
                      onChange={(e) => updateParam(i, { name: e.target.value })}
                    />
                    <Input
                      className="text-xs"
                      placeholder="rótulo"
                      value={p.label ?? ""}
                      onChange={(e) => updateParam(i, { label: e.target.value || null })}
                    />
                    <Input
                      className="font-mono text-xs"
                      placeholder="padrão"
                      value={p.default ?? ""}
                      onChange={(e) => updateParam(i, { default: e.target.value || null })}
                    />
                    <label className="flex shrink-0 items-center gap-1 text-xs" title="Preenchimento obrigatório">
                      <input
                        type="checkbox"
                        className="size-3.5 accent-teal-600"
                        checked={p.required ?? false}
                        onChange={(e) => updateParam(i, { required: e.target.checked })}
                      />
                      obrig.
                    </label>
                    <button
                      onClick={() => setEditor({ ...editor, params: editor.params.filter((_, j) => j !== i) })}
                      className="shrink-0 p-1 text-zinc-400 transition hover:text-red-500"
                      title="Remover parâmetro"
                    >
                      <XIcon className="size-3.5" />
                    </button>
                  </div>
                ))}
              </div>
              <Button
                size="sm"
                className="mt-2"
                onClick={() =>
                  setEditor({
                    ...editor,
                    params: [...editor.params, { name: "", label: null, required: null, default: null }],
                  })
                }
              >
                <PlusIcon className="size-3" /> Parâmetro
              </Button>
            </div>
            <div className="flex justify-end gap-2 border-t border-zinc-200 pt-3 dark:border-zinc-800">
              <Button onClick={() => setEditor(null)}>Cancelar</Button>
              <Button variant="primary" onClick={() => void saveRoutineFromEditor()}>
                Salvar
              </Button>
            </div>
          </div>
        ) : null}
      </Modal>
    </div>
  );
}
