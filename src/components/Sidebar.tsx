import type { ViewId } from "../types";
import { CloudIcon, CommitIcon, DashboardIcon, FolderIcon, GitBranchIcon, HistoryIcon, RocketIcon, ServerIcon, SettingsIcon, TerminalIcon } from "./Icons";

const NAV = [
  { id: "dashboard" as ViewId, label: "Dashboard", icon: DashboardIcon },
  { id: "servers" as ViewId, label: "Servidores", icon: ServerIcon },
  { id: "terminal" as ViewId, label: "Terminal", icon: TerminalIcon },
  { id: "files" as ViewId, label: "Arquivos remotos", icon: FolderIcon },
  { id: "git" as ViewId, label: "Git", icon: CommitIcon },
  { id: "github" as ViewId, label: "GitHub", icon: CloudIcon },
  { id: "projects" as ViewId, label: "Projetos", icon: GitBranchIcon },
  { id: "deploy" as ViewId, label: "Deploy", icon: RocketIcon },
  { id: "history" as ViewId, label: "Histórico", icon: HistoryIcon },
  { id: "settings" as ViewId, label: "Configurações", icon: SettingsIcon },
];

export const VIEW_TITLES: Record<ViewId, string> = {
  dashboard: "Dashboard",
  servers: "Servidores",
  terminal: "Terminal SSH",
  files: "Arquivos remotos (SFTP)",
  git: "Gerenciador Git",
  github: "GitHub",
  projects: "Projetos",
  deploy: "Deploy",
  history: "Histórico e auditoria",
  settings: "Configurações",
};

export function Sidebar({ view, onNavigate }: { view: ViewId; onNavigate: (v: ViewId) => void }) {
  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-zinc-200 bg-zinc-50 dark:border-zinc-800 dark:bg-zinc-900/60">
      <div className="flex items-center gap-2.5 px-4 py-4">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-teal-600 text-white">
          <TerminalIcon className="size-4" />
        </div>
        <div className="min-w-0">
          <p className="text-sm font-semibold leading-tight">DevOps Desktop</p>
          <p className="truncate text-[10px] text-zinc-500 dark:text-zinc-400">Central de SSH, Git e Deploy</p>
        </div>
      </div>
      <nav className="flex flex-1 flex-col gap-0.5 px-2 py-2">
        {NAV.map(({ id, label, icon: Icon }) => {
          const active = view === id;
          return (
            <button
              key={id}
              onClick={() => onNavigate(id)}
              className={`flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm transition ${
                active
                  ? "bg-teal-600/10 font-medium text-teal-700 dark:bg-teal-400/10 dark:text-teal-300"
                  : "text-zinc-600 hover:bg-zinc-200/70 dark:text-zinc-400 dark:hover:bg-zinc-800/70"
              }`}
            >
              <Icon className="size-4 shrink-0" />
              {label}
            </button>
          );
        })}
      </nav>
      <div className="px-4 py-3 text-[10px] leading-relaxed text-zinc-400 dark:text-zinc-500">
        v0.1.0 · inventário local em SQLite · GitHub só quando você conecta
      </div>
    </aside>
  );
}
