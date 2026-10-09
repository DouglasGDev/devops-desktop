import type { ReactNode } from "react";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { api, onAuthRequest } from "../lib/api";
import type { AuthRequest, Project, Server } from "../types";

export type ToastKind = "success" | "error" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

interface AppContextValue {
  ready: boolean;
  settings: Record<string, string>;
  setSetting: (key: string, value: string) => Promise<void>;
  theme: "dark" | "light";
  setTheme: (theme: "dark" | "light") => void;
  servers: Server[];
  projects: Project[];
  reloadServers: () => Promise<void>;
  reloadProjects: () => Promise<void>;
  toasts: Toast[];
  pushToast: (kind: ToastKind, message: string) => void;
  /** Prompts de senha aguardando resposta, na ordem em que chegaram. */
  authRequests: AuthRequest[];
  answerAuth: (id: number, password: string) => Promise<void>;
  declineAuth: (id: number) => Promise<void>;
}

const AppContext = createContext<AppContextValue | null>(null);

let toastSeq = 1;

function applyThemeClass(theme: "dark" | "light") {
  document.documentElement.classList.toggle("dark", theme === "dark");
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false);
  const [settings, setSettings] = useState<Record<string, string>>({});
  const [servers, setServers] = useState<Server[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [authRequests, setAuthRequests] = useState<AuthRequest[]>([]);

  const pushToast = useCallback((kind: ToastKind, message: string) => {
    const id = toastSeq++;
    setToasts((prev) => [...prev.slice(-4), { id, kind, message }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 6000);
  }, []);

  const dropAuth = useCallback((id: number) => {
    setAuthRequests((prev) => prev.filter((r) => r.id !== id));
  }, []);

  const answerAuth = useCallback(
    async (id: number, password: string) => {
      // A senha vive apenas nesta chamada: não entra em estado nem em disco.
      try {
        await api.providePassword(id, password);
      } catch (e) {
        pushToast("error", `Não foi possível enviar a senha: ${String(e)}`);
      } finally {
        dropAuth(id);
      }
    },
    [dropAuth, pushToast],
  );

  const declineAuth = useCallback(
    async (id: number) => {
      try {
        await api.cancelPasswordPrompt(id);
      } catch {
        // A solicitação já expirou no backend; resta limpar a interface.
      }
      dropAuth(id);
    },
    [dropAuth],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    onAuthRequest((req) => {
      if (cancelled) return;
      setAuthRequests((prev) => (prev.some((r) => r.id === req.id) ? prev : [...prev, req]));
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Sem bridge de eventos o prompt não chega: as operações seguem por chave/agente.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const setSetting = useCallback(async (key: string, value: string) => {
    await api.setSetting(key, value);
    setSettings((prev) => ({ ...prev, [key]: value }));
    if (key === "theme") applyThemeClass(value === "light" ? "light" : "dark");
  }, []);

  const reloadServers = useCallback(async () => {
    setServers(await api.listServers());
  }, []);

  const reloadProjects = useCallback(async () => {
    setProjects(await api.listProjects());
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [s, srv, prj] = await Promise.all([api.getSettings(), api.listServers(), api.listProjects()]);
        if (cancelled) return;
        setSettings(s);
        setServers(srv);
        setProjects(prj);
        applyThemeClass(s.theme === "light" ? "light" : "dark");
      } catch (e) {
        pushToast("error", `Falha ao carregar dados locais: ${String(e)}`);
      } finally {
        if (!cancelled) setReady(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pushToast]);

  const value = useMemo<AppContextValue>(
    () => ({
      ready,
      settings,
      setSetting,
      theme: settings.theme === "light" ? "light" : "dark",
      setTheme: (t) => void setSetting("theme", t),
      servers,
      projects,
      reloadServers,
      reloadProjects,
      toasts,
      pushToast,
      authRequests,
      answerAuth,
      declineAuth,
    }),
    [
      ready,
      settings,
      setSetting,
      servers,
      projects,
      reloadServers,
      reloadProjects,
      toasts,
      pushToast,
      authRequests,
      answerAuth,
      declineAuth,
    ],
  );

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp deve ser usado dentro de AppProvider");
  return ctx;
}
