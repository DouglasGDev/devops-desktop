import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AuditEntry,
  AuthRequest,
  BranchInfo,
  CommitDetail,
  ConnectionTestResult,
  DeployActive,
  DeployProfile,
  DeployProfileInput,
  DeployPreflight,
  DeployRun,
  LogEvent,
  StepEvent,
  FileContent,
  FileDiff,
  GitActionOutcome,
  GitIdentity,
  GitOperation,
  GitHubConnectResult,
  GitHubDeviceFlow,
  GitHubInbox,
  GitHubIssue,
  GitHubComment,
  GitHubPull,
  GitHubPullDetail,
  GitHubRepo,
  GitHubRun,
  GitHubStatus,
  MergeMethod,
  ReviewEvent,
  GraphRow,
  ImportSshHost,
  Project,
  RemoteEntry,
  RemoteAccess,
  RemoteInfo,
  RepoStatus,
  ResetMode,
  Routine,
  RoutineInput,
  SearchResults,
  SecretFinding,
  StashEntry,
  Server,
  ServerInput,
  ServerMetrics,
  SshDiscoveryReport,
  TagInfo,
  TerminalHistoryEntry,
  TerminalSessionInfo,
  ToolVersions,
  TransferInfo,
  TransferProgress,
  PullStrategy,
} from "../types";

export interface TerminalOutputEvent {
  sessionId: number;
  data: string;
}

export interface TerminalExitEvent {
  sessionId: number;
  exitCode: number | null;
}

export function onTerminalOutput(handler: (e: TerminalOutputEvent) => void): Promise<UnlistenFn> {
  return listen<TerminalOutputEvent>("terminal-output", (ev) => handler(ev.payload));
}

export function onTerminalExit(handler: (e: TerminalExitEvent) => void): Promise<UnlistenFn> {
  return listen<TerminalExitEvent>("terminal-exit", (ev) => handler(ev.payload));
}

export function onTransferProgress(handler: (e: TransferProgress) => void): Promise<UnlistenFn> {
  return listen<TransferProgress>("transfer-progress", (ev) => handler(ev.payload));
}

export function onDeployStep(handler: (e: StepEvent) => void): Promise<UnlistenFn> {
  return listen<StepEvent>("deploy-step", (ev) => handler(ev.payload));
}

export function onDeployLog(handler: (e: LogEvent) => void): Promise<UnlistenFn> {
  return listen<LogEvent>("deploy-log", (ev) => handler(ev.payload));
}

export function onDeployDone(handler: (run: DeployRun) => void): Promise<UnlistenFn> {
  return listen<{ runId: number; run: DeployRun }>("deploy-done", (ev) => handler(ev.payload.run));
}

// O backend pede uma senha pontual para uma operação; nada é persistido.
export function onAuthRequest(handler: (e: AuthRequest) => void): Promise<UnlistenFn> {
  return listen<AuthRequest>("auth-request", (ev) => handler(ev.payload));
}

export const api = {
  getSettings: () => invoke<Record<string, string>>("get_settings"),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  getAppDataDir: () => invoke<string>("get_app_data_dir"),

  getToolVersions: () => invoke<ToolVersions>("get_tool_versions"),

  listServers: () => invoke<Server[]>("list_servers"),
  addServer: (input: ServerInput) => invoke<Server>("add_server", { input }),
  updateServer: (id: number, input: ServerInput) => invoke<Server>("update_server", { id, input }),
  /** Devolve quantos modelos de deploy que publicavam nele saíram junto. */
  deleteServer: (id: number) => invoke<number>("delete_server", { id }),
  testConnection: (serverId: number) => invoke<ConnectionTestResult>("test_connection", { serverId }),
  // Coleta opcional de métricas reais (§12): só roda quando o usuário pede e
  // executa apenas comandos de leitura no servidor.
  serverMetrics: (serverId: number) => invoke<ServerMetrics>("server_metrics", { serverId }),

  providePassword: (requestId: number, password: string) =>
    invoke<void>("provide_password", { requestId, password }),
  cancelPasswordPrompt: (requestId: number) => invoke<void>("cancel_password_prompt", { requestId }),

  discoverSsh: () => invoke<SshDiscoveryReport>("discover_ssh"),
  importSshHosts: (entries: ImportSshHost[]) => invoke<Server[]>("import_ssh_hosts", { entries }),

  listProjects: () => invoke<Project[]>("list_projects"),
  addProject: (path: string, name: string | null) => invoke<Project>("add_project", { path, name }),
  refreshProject: (id: number) => invoke<Project>("refresh_project", { id }),
  updateProject: (id: number, name: string, defaultServerId: number | null) =>
    invoke<Project>("update_project", { id, name, defaultServerId }),
  /** Devolve quantos modelos do projeto saíram junto com ele. */
  deleteProject: (id: number) => invoke<number>("delete_project", { id }),

  listDeployProfiles: (projectId: number | null) =>
    invoke<DeployProfile[]>("list_deploy_profiles", { projectId }),
  saveDeployProfile: (input: DeployProfileInput) =>
    invoke<DeployProfile>("save_deploy_profile", { input }),
  deleteDeployProfile: (id: number) => invoke<void>("delete_deploy_profile", { id }),
  setDefaultDeployProfile: (id: number) => invoke<DeployProfile>("set_default_deploy_profile", { id }),

  // Deploy: o motor roda o pipeline de verdade em thread própria e emite o
  // progresso. A interface nunca inventa resultado.
  deployPreflight: (profileId: number) => invoke<DeployPreflight>("deploy_preflight", { profileId }),
  deployStart: (profileId: number, secretValues: Record<string, string>, allowSecrets: boolean) =>
    invoke<number>("deploy_start", { profileId, secretValues, allowSecrets }),
  deployRollback: (runId: number, secretValues: Record<string, string>) =>
    invoke<number>("deploy_rollback", { runId, secretValues }),
  deployCancel: (runId: number) => invoke<string>("deploy_cancel", { runId }),
  deployActive: () => invoke<DeployActive[]>("deploy_active"),
  deployRuns: (projectId: number | null, limit?: number | null) =>
    invoke<DeployRun[]>("deploy_runs", { projectId, limit: limit ?? null }),
  deployRun: (runId: number) => invoke<DeployRun>("deploy_run", { runId }),

  queryAuditLog: (params: {
    category?: string | null;
    level?: string | null;
    search?: string | null;
    limit?: number | null;
    offset?: number | null;
  }) =>
    invoke<AuditEntry[]>("query_audit_log", {
      category: params.category ?? null,
      level: params.level ?? null,
      search: params.search ?? null,
      limit: params.limit ?? null,
      offset: params.offset ?? null,
    }),

  searchAll: (query: string) => invoke<SearchResults>("search_all", { query }),

  openTerminalSession: (serverId: number, cols: number, rows: number) =>
    invoke<TerminalSessionInfo>("open_terminal_session", { serverId, cols, rows }),
  writeTerminalSession: (sessionId: number, data: string) =>
    invoke<void>("write_terminal_session", { sessionId, data }),
  resizeTerminalSession: (sessionId: number, cols: number, rows: number) =>
    invoke<void>("resize_terminal_session", { sessionId, cols, rows }),
  closeTerminalSession: (sessionId: number) => invoke<void>("close_terminal_session", { sessionId }),
  listTerminalSessions: () => invoke<TerminalSessionInfo[]>("list_terminal_sessions"),

  listRoutines: () => invoke<Routine[]>("list_routines"),
  saveRoutine: (input: RoutineInput) => invoke<Routine>("save_routine", { input }),
  deleteRoutine: (id: number) => invoke<void>("delete_routine", { id }),

  recordTerminalCommand: (sessionId: number, command: string) =>
    invoke<void>("record_terminal_command", { sessionId, command }),
  listTerminalHistory: (limit?: number | null) =>
    invoke<TerminalHistoryEntry[]>("list_terminal_history", { limit: limit ?? null }),
  clearTerminalHistory: () => invoke<void>("clear_terminal_history"),

  sftpList: (serverId: number, path: string) => invoke<RemoteEntry[]>("sftp_list", { serverId, path }),
  sftpMkdir: (serverId: number, path: string) => invoke<void>("sftp_mkdir", { serverId, path }),
  sftpRename: (serverId: number, from: string, to: string) =>
    invoke<void>("sftp_rename", { serverId, from, to }),
  sftpRemove: (serverId: number, path: string) => invoke<void>("sftp_remove", { serverId, path }),
  sftpRemoveDir: (serverId: number, path: string) => invoke<void>("sftp_remove_dir", { serverId, path }),
  sftpChmod: (serverId: number, path: string, mode: string) =>
    invoke<void>("sftp_chmod", { serverId, path, mode }),
  sftpReadText: (serverId: number, path: string) => invoke<string>("sftp_read_text", { serverId, path }),
  sftpWriteText: (serverId: number, path: string, content: string) =>
    invoke<void>("sftp_write_text", { serverId, path, content }),

  transferStart: (params: {
    serverId: number;
    kind: "upload" | "download";
    remotePath: string;
    localPath: string;
    recursive: boolean;
  }) =>
    invoke<number>("transfer_start", { input: params }),
  transferCancel: (transferId: number) => invoke<void>("transfer_cancel", { transferId }),
  transfersList: () => invoke<TransferInfo[]>("transfers_list"),

  // Git: todas as operações partem do projeto cadastrado, nunca de um caminho
  // arbitrário vindo da interface.
  gitStatus: (projectId: number) => invoke<RepoStatus>("git_status", { projectId }),
  gitIdentity: (projectId: number) => invoke<GitIdentity>("git_identity", { projectId }),
  gitDiff: (params: {
    projectId: number;
    path?: string | null;
    staged?: boolean;
    commit?: string | null;
    from?: string | null;
    to?: string | null;
  }) =>
    invoke<FileDiff[]>("git_diff", {
      projectId: params.projectId,
      path: params.path ?? null,
      staged: params.staged ?? false,
      commit: params.commit ?? null,
      from: params.from ?? null,
      to: params.to ?? null,
    }),
  gitStage: (projectId: number, paths: string[]) => invoke<RepoStatus>("git_stage", { projectId, paths }),
  gitUnstage: (projectId: number, paths: string[]) =>
    invoke<RepoStatus>("git_unstage", { projectId, paths }),
  gitStageHunks: (params: {
    projectId: number;
    path: string;
    indexes: number[];
    staged?: boolean;
    reverse?: boolean;
  }) =>
    invoke<number>("git_stage_hunks", {
      projectId: params.projectId,
      path: params.path,
      indexes: params.indexes,
      staged: params.staged ?? false,
      reverse: params.reverse ?? false,
    }),
  gitDiscard: (projectId: number, path: string) => invoke<RepoStatus>("git_discard", { projectId, path }),
  gitCommit: (projectId: number, message: string, amend?: boolean, allowSecrets?: boolean) =>
    invoke<string>("git_commit", {
      projectId,
      message,
      amend: amend ?? false,
      allowSecrets: allowSecrets ?? false,
    }),
  // O que a operação publicaria, checado no conteúdo real do Git (§8).
  // `"index"` (padrão) é o próximo commit; `"push"` é o que o remoto ainda não tem.
  gitSecretCheck: (projectId: number, scope?: "index" | "push") =>
    invoke<SecretFinding[]>("git_secret_check", { projectId, scope: scope ?? null }),
  scanProjectSecrets: (projectId: number) =>
    invoke<SecretFinding[]>("scan_project_secrets", { projectId }),
  // Tira do índice sem apagar o arquivo local; `gitignore` lista no .gitignore.
  gitUntrackFile: (projectId: number, path: string, gitignore?: boolean) =>
    invoke<RepoStatus>("git_untrack_file", {
      projectId,
      path,
      gitignore: gitignore ?? true,
    }),
  gitGraph: (projectId: number, limit?: number, allRefs?: boolean) =>
    invoke<GraphRow[]>("git_graph", {
      projectId,
      limit: limit ?? 100,
      allRefs: allRefs ?? true,
    }),
  gitCommitDetail: (projectId: number, id: string) =>
    invoke<CommitDetail>("git_commit_detail", { projectId, id }),
  gitBranches: (projectId: number) => invoke<BranchInfo[]>("git_branches", { projectId }),
  gitCheckout: (projectId: number, name: string, create?: boolean, startPoint?: string | null) =>
    invoke<RepoStatus>("git_checkout", {
      projectId,
      name,
      create: create ?? false,
      startPoint: startPoint ?? null,
    }),
  gitRenameBranch: (projectId: number, from: string, to: string) =>
    invoke<BranchInfo[]>("git_rename_branch", { projectId, from, to }),
  gitDeleteBranch: (projectId: number, name: string, force?: boolean) =>
    invoke<BranchInfo[]>("git_delete_branch", { projectId, name, force: force ?? false }),
  gitMerge: (projectId: number, branch: string) =>
    invoke<GitActionOutcome>("git_merge", { projectId, branch }),
  gitRebase: (projectId: number, upstream: string) =>
    invoke<GitActionOutcome>("git_rebase", { projectId, upstream }),
  gitAbort: (projectId: number, kind: GitOperation) => invoke<RepoStatus>("git_abort", { projectId, kind }),
  gitContinuePending: (projectId: number, kind: GitOperation) =>
    invoke<GitActionOutcome>("git_continue_pending", { projectId, kind }),
  gitSkipPending: (projectId: number, kind: GitOperation) =>
    invoke<GitActionOutcome>("git_skip_pending", { projectId, kind }),
  gitResolveConflict: (projectId: number, path: string, side: "ours" | "theirs") =>
    invoke<RepoStatus>("git_resolve_conflict", { projectId, path, side }),
  gitRevert: (projectId: number, commit: string, mainline?: number | null) =>
    invoke<GitActionOutcome>("git_revert", { projectId, commit, mainline: mainline ?? null }),
  gitCherryPick: (projectId: number, commit: string, mainline?: number | null) =>
    invoke<GitActionOutcome>("git_cherry_pick", { projectId, commit, mainline: mainline ?? null }),
  // O guardado é endereçado pela posição na lista do git, nunca por caminho.
  gitStashList: (projectId: number) => invoke<StashEntry[]>("git_stash_list", { projectId }),
  gitStashPush: (projectId: number, message?: string | null, includeUntracked?: boolean) =>
    invoke<GitActionOutcome>("git_stash_push", {
      projectId,
      message: message ?? null,
      includeUntracked: includeUntracked ?? false,
    }),
  gitStashApply: (projectId: number, index: number, pop?: boolean) =>
    invoke<GitActionOutcome>("git_stash_apply", { projectId, index, pop: pop ?? false }),
  gitStashDrop: (projectId: number, index: number) =>
    invoke<GitActionOutcome>("git_stash_drop", { projectId, index }),
  gitReset: (projectId: number, mode: ResetMode, target: string) =>
    invoke<GitActionOutcome>("git_reset", { projectId, mode, target }),
  gitFetch: (projectId: number, remote?: string | null) =>
    invoke<GitActionOutcome>("git_fetch", { projectId, remote: remote ?? null }),
  gitPull: (projectId: number, strategy: PullStrategy) =>
    invoke<GitActionOutcome>("git_pull", { projectId, strategy }),
  gitPush: (params: {
    projectId: number;
    remote?: string | null;
    branch?: string | null;
    setUpstream?: boolean;
    allowSecrets?: boolean;
  }) =>
    invoke<GitActionOutcome>("git_push", {
      projectId: params.projectId,
      remote: params.remote ?? null,
      branch: params.branch ?? null,
      setUpstream: params.setUpstream ?? false,
      allowSecrets: params.allowSecrets ?? false,
    }),
  gitPublish: (params: {
    projectId: number;
    remote?: string | null;
    tag?: string | null;
    commands?: string[];
    allowSecrets?: boolean;
  }) =>
    invoke<GitActionOutcome>("git_publish", {
      projectId: params.projectId,
      remote: params.remote ?? null,
      tag: params.tag ?? null,
      commands: params.commands ?? [],
      allowSecrets: params.allowSecrets ?? false,
    }),
  gitUnpublishTag: (projectId: number, name: string, remote?: string | null) =>
    invoke<GitActionOutcome>("git_unpublish_tag", { projectId, name, remote: remote ?? null }),
  gitTags: (projectId: number) => invoke<TagInfo[]>("git_tags", { projectId }),
  gitCreateTag: (projectId: number, name: string, target?: string | null, message?: string | null) =>
    invoke<TagInfo[]>("git_create_tag", {
      projectId,
      name,
      target: target ?? null,
      message: message ?? null,
    }),
  gitDeleteTag: (projectId: number, name: string) => invoke<TagInfo[]>("git_delete_tag", { projectId, name }),
  gitRemotes: (projectId: number) => invoke<RemoteInfo[]>("git_remotes", { projectId }),
  // Árvore lida do índice (`git ls-files`) e conteúdo lido do HEAD — nada de
  // caminho absoluto: o git responde por caminho relativo dentro do repositório.
  gitTree: (projectId: number) => invoke<string[]>("git_tree", { projectId }),
  gitFileContent: (projectId: number, path: string) =>
    invoke<FileContent>("git_file_content", { projectId, path }),
  // Só lê refs no remoto, com prazo de 25s: prova que esta máquina consegue
  // publicar. Um `ok:false` traz a frase do que falta, não um erro.
  gitCheckRemote: (projectId: number, remote?: string | null) =>
    invoke<RemoteAccess>("git_check_remote", { projectId, remote: remote ?? null }),
  gitAddRemote: (projectId: number, name: string, url: string) =>
    invoke<RemoteInfo[]>("git_add_remote", { projectId, name, url }),
  gitSetRemoteUrl: (projectId: number, name: string, url: string) =>
    invoke<RemoteInfo[]>("git_set_remote_url", { projectId, name, url }),
  gitRemoveRemote: (projectId: number, name: string) =>
    invoke<RemoteInfo[]>("git_remove_remote", { projectId, name }),
  gitClone: (params: {
    parentPath: string;
    url: string;
    targetName?: string | null;
    branch?: string | null;
  }) =>
    invoke<string>("git_clone", {
      parentPath: params.parentPath,
      url: params.url,
      targetName: params.targetName ?? null,
      branch: params.branch ?? null,
    }),
  openInEditor: (projectId: number, path?: string | null) =>
    invoke<void>("open_in_editor", { projectId, path: path ?? null }),

  // GitHub: o token fica no backend; a interface só recebe status e dados.
  githubStatus: () => invoke<GitHubStatus>("github_status"),
  githubSetClientId: (clientId: string) =>
    invoke<GitHubStatus>("github_set_client_id", { input: { clientId } }),
  githubConnectStart: () => invoke<GitHubDeviceFlow>("github_connect_start"),
  githubConnectPoll: () => invoke<GitHubConnectResult>("github_connect_poll"),
  githubDisconnect: () => invoke<GitHubStatus>("github_disconnect"),
  githubRepos: () => invoke<GitHubRepo[]>("github_repos"),
  githubPulls: (repo: string) => invoke<GitHubPull[]>("github_pulls", { repo }),
  githubCreateRepo: (params: { name: string; private: boolean; description?: string | null }) =>
    invoke<GitHubRepo>("github_create_repo", {
      input: {
        name: params.name,
        private: params.private,
        description: params.description ?? null,
      },
    }),
  githubInbox: () => invoke<GitHubInbox>("github_inbox"),
  githubPullDetail: (repo: string, number: number) =>
    invoke<GitHubPullDetail>("github_pull_detail", { input: { repo, number } }),
  githubPullFiles: (repo: string, number: number) =>
    invoke<FileDiff[]>("github_pull_files", { input: { repo, number } }),
  githubIssues: (repo: string) => invoke<GitHubIssue[]>("github_issues", { repo }),
  githubRuns: (repo: string) => invoke<GitHubRun[]>("github_runs", { repo }),
  githubCreateIssue: (params: { repo: string; title: string; body?: string | null }) =>
    invoke<GitHubIssue>("github_create_issue", {
      input: { repo: params.repo, title: params.title, body: params.body ?? null },
    }),
  githubComment: (params: { repo: string; number: number; body: string }) =>
    invoke<void>("github_comment", { input: params }),
  githubCloseIssue: (repo: string, number: number) =>
    invoke<void>("github_close_issue", { input: { repo, number } }),
  githubReview: (params: { repo: string; number: number; event: ReviewEvent; body?: string | null }) =>
    invoke<void>("github_review", {
      input: { repo: params.repo, number: params.number, event: params.event, body: params.body ?? null },
    }),
  githubMerge: (params: { repo: string; number: number; method: MergeMethod; sha: string }) =>
    invoke<string>("github_merge", { input: params }),
  githubRerun: (repo: string, runId: number) => invoke<void>("github_rerun", { input: { repo, runId } }),
  githubComments: (repo: string, number: number) =>
    invoke<GitHubComment[]>("github_comments", { input: { repo, number } }),
  githubBranches: (repo: string) => invoke<string[]>("github_branches", { repo }),
  githubCreatePull: (params: {
    repo: string;
    title: string;
    head: string;
    base: string;
    body?: string | null;
    draft?: boolean;
  }) =>
    invoke<GitHubPullDetail>("github_create_pull", {
      input: {
        repo: params.repo,
        title: params.title,
        head: params.head,
        base: params.base,
        body: params.body ?? null,
        draft: params.draft ?? false,
      },
    }),
  gitCheckoutPull: (projectId: number, number: number, remote?: string | null) =>
    invoke<GitActionOutcome>("git_checkout_pull", {
      projectId,
      number,
      remote: remote ?? null,
    }),
};
