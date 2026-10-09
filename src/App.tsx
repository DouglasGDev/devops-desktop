import { useState } from "react";
import { PasswordPrompt } from "./components/PasswordPrompt";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { Topbar } from "./components/Topbar";
import { AppProvider, useApp } from "./store/AppContext";
import type { ViewId } from "./types";
import { DashboardView } from "./views/DashboardView";
import { DeployView } from "./views/DeployView";
import { FilesView } from "./views/FilesView";
import { GitView } from "./views/GitView";
import { HistoryView } from "./views/HistoryView";
import { ProjectsView } from "./views/ProjectsView";
import { ServersView } from "./views/ServersView";
import { SettingsView } from "./views/SettingsView";
import { TerminalView } from "./views/TerminalView";
import { Spinner } from "./components/ui";

function Shell() {
  const { ready } = useApp();
  const [view, setView] = useState<ViewId>("dashboard");

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar view={view} onNavigate={setView} />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar view={view} onNavigate={setView} />
        <main className="flex-1 overflow-y-auto px-5 py-5">
          {ready ? (
            <>
              {/* O terminal permanece montado: as sessões não podem ser
                  destruídas ao navegar entre seções. */}
              <div className={view === "terminal" ? "h-full min-h-0" : "hidden"}>
                <TerminalView active={view === "terminal"} onNavigate={setView} />
              </div>
              {view === "dashboard" ? <DashboardView onNavigate={setView} /> : null}
              {view === "servers" ? <ServersView /> : null}
              {view === "files" ? <FilesView onNavigate={setView} /> : null}
              {view === "projects" ? <ProjectsView /> : null}
              {view === "deploy" ? <DeployView onNavigate={setView} /> : null}
              {view === "git" ? (
                <div className="h-full min-h-0">
                  <GitView onNavigate={setView} />
                </div>
              ) : null}
              {view === "history" ? <HistoryView /> : null}
              {view === "settings" ? <SettingsView /> : null}
            </>
          ) : (
            <div className="flex h-full items-center justify-center gap-2 text-sm text-zinc-500 dark:text-zinc-400">
              <Spinner /> Carregando dados locais…
            </div>
          )}
        </main>
      </div>
      {/* O prompt de senha é global: uma operação em segundo plano pode pedí-lo
          em qualquer seção. */}
      <PasswordPrompt />
      <Toasts />
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
