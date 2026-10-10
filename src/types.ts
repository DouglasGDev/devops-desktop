export type Environment = "dev" | "staging" | "prod" | "database" | "docker" | "other";
export type AuthMethod = "key" | "agent" | "password" | "unknown";

export interface Server {
  id: number;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  identityFile: string | null;
  sshAlias: string | null;
  environment: Environment;
  groupName: string | null;
  notes: string | null;
  source: "imported" | "manual";
  lastStatus: string | null;
  lastStatusDetail: string | null;
  lastCheckedAt: string | null;
  lastOkAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ServerInput {
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  identityFile: string | null;
  sshAlias: string | null;
  environment: Environment;
  groupName: string | null;
  notes: string | null;
}

export interface ConnectionTestResult {
  ok: boolean;
  exitCode: number | null;
  durationMs: number;
  errorKind: string | null;
  message: string;
}

export interface ImportSshHost {
  alias: string;
  host: string;
  user: string;
  port: number;
  identityFile: string | null;
  environment: Environment;
}

export interface SshHostCandidate {
  patterns: string[];
  alias: string;
  hostName: string | null;
  user: string | null;
  port: number | null;
  identityFiles: string[];
  proxyJump: string | null;
  sourceFile: string;
  isConcrete: boolean;
}

export interface KeyFile {
  path: string;
  kind: "private" | "public";
  keyType: string | null;
  hasPair: boolean;
  encrypted: boolean | null;
}

export interface AgentKey {
  fingerprint: string;
  keyType: string;
  comment: string | null;
}

export interface KnownHostsInfo {
  path: string;
  total: number;
  hashed: number;
}

export interface SshDiscoveryReport {
  configFiles: string[];
  hosts: SshHostCandidate[];
  publicKeys: KeyFile[];
  privateKeys: KeyFile[];
  knownHosts: KnownHostsInfo | null;
  agent: {
    available: boolean;
    keys: AgentKey[];
    note: string | null;
  };
  warnings: string[];
}

export interface Project {
  id: number;
  name: string;
  localPath: string;
  isGitRepo: boolean;
  gitBranch: string | null;
  gitRemote: string | null;
  defaultServerId: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface AuditEntry {
  id: number;
  ts: string;
  level: "info" | "success" | "warning" | "error";
  category: string;
  operation: string;
  serverName: string | null;
  projectName: string | null;
  exitCode: number | null;
  detail: string | null;
}

export interface ToolVersions {
  ssh: string | null;
  sshAdd: string | null;
  git: string | null;
  agentRunning: boolean | null;
  /** Chaves carregadas no ssh-agent. */
  agentKeys: number | null;
  /** user.name + user.email globais do git. */
  gitUser: string | null;
  gh: string | null;
  /** Conta ativa do `gh auth status`; nunca o token. */
  ghAccount: string | null;
}

export interface RoutineParam {
  name: string;
  label: string | null;
  required: boolean | null;
  default: string | null;
}

export interface Routine {
  id: number;
  name: string;
  description: string | null;
  command: string;
  danger: "read" | "write";
  params: RoutineParam[];
  isBuiltin: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface RoutineInput {
  id: number | null;
  name: string;
  description: string | null;
  command: string;
  danger: "read" | "write";
  params: RoutineParam[];
}

export interface TerminalSessionInfo {
  id: number;
  serverId: number;
  serverName: string;
  environment: string;
  username: string;
  host: string;
  port: number;
  alive: boolean;
  exitCode: number | null;
  startedAt: string;
}

export interface TerminalHistoryEntry {
  id: number;
  ts: string;
  serverId: number | null;
  serverName: string | null;
  command: string;
}

export interface SearchResults {
  servers: { id: number; name: string; host: string }[];
  projects: { id: number; name: string; localPath: string }[];
  history: AuditEntry[];
}

export interface RemoteEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  perms: string | null;
  size: number | null;
  owner: string | null;
  group: string | null;
  modified: string | null;
}

export interface TransferProgress {
  transferId: number;
  fileName: string;
  pct: number;
  transferred: number;
  total: number | null;
  rate: number | null;
  eta: number | null;
  done: boolean;
  ok: boolean;
  error: string | null;
}

export interface TransferInfo {
  id: number;
  kind: "upload" | "download";
  remotePath: string;
  localPath: string;
  serverName: string;
  elapsedSeconds: number;
}

export interface AuthRequest {
  id: number;
  serverName: string;
  purpose: string;
  timeoutSecs: number;
}

// ---------- Git ----------

export interface StatusFile {
  path: string;
  oldPath: string | null;
  indexCode: string;
  worktreeCode: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  conflict: boolean;
  kind: string;
}

export interface RepoStatus {
  branch: string | null;
  detachedAt: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: StatusFile[];
  conflictCount: number;
}

export type DiffLineKind = "add" | "del" | "ctx";

export interface DiffLine {
  kind: DiffLineKind;
  oldNo: number | null;
  newNo: number | null;
  text: string;
}

export interface DiffHunk {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  statusChar: string;
  hunks: DiffHunk[];
  binary: boolean;
}

export interface CommitInfo {
  id: string;
  shortId: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  unixDate: number;
  subject: string;
  refs: string[];
  body: string;
}

export interface GraphRow {
  graph: string;
  commit: CommitInfo;
}

export interface BranchInfo {
  name: string;
  isHead: boolean;
  remote: string | null;
  ahead: number;
  behind: number;
  lastSubject: string;
  lastUnixDate: number;
}

export interface TagInfo {
  name: string;
  target: string;
  message: string | null;
}

export interface RemoteInfo {
  name: string;
  fetchUrl: string | null;
  pushUrl: string | null;
}

/** Veredito do teste de acesso da máquina a um remoto (git ls-remote). */
export interface RemoteAccess {
  ok: boolean;
  remote: string;
  detail: string;
}

export interface CommitDetail {
  commit: CommitInfo;
  files: StatusFile[];
  totalAdditions: number;
  totalDeletions: number;
}

export interface GitIdentity {
  userName: string | null;
  userEmail: string | null;
}

export interface GitActionOutcome {
  ok: boolean;
  output: string;
}

export type GitEditor = "system" | "vscode" | "cursor";

/// Modos de reversão do Git. `hard` é o único que descarta conteúdo em trabalho.
export type ResetMode = "soft" | "mixed" | "hard";

export type DeployStrategy = "sftp" | "git" | "artifacts" | "script" | "docker";

export type DeployProfile = {
  id: number;
  projectId: number;
  name: string;
  serverId: number;
  serverName: string;
  serverEnvironment: string;
  strategy: DeployStrategy;
  gitBranch: string | null;
  remotePath: string;
  includePaths: string[];
  excludePaths: string[];
  prepareCommands: string[];
  buildCommands: string[];
  restartCommands: string[];
  healthChecks: string[];
  rollbackCommands: string[];
  testCommands: string[];
  secretKeys: string[];
  backupEnabled: boolean;
  releasesEnabled: boolean;
  deleteExtras: boolean;
  stepTimeoutSecs: number;
  isDefault: boolean;
  createdAt: string;
  updatedAt: string;
};

export type DeployProfileInput = {
  id: number | null;
  projectId: number;
  name: string;
  serverId: number;
  strategy: DeployStrategy;
  gitBranch: string | null;
  remotePath: string;
  includePaths: string[];
  excludePaths: string[];
  prepareCommands: string[];
  buildCommands: string[];
  restartCommands: string[];
  healthChecks: string[];
  rollbackCommands: string[];
  testCommands: string[];
  secretKeys: string[];
  backupEnabled: boolean;
  releasesEnabled: boolean;
  deleteExtras: boolean;
  stepTimeoutSecs: number;
  isDefault: boolean;
};
export type PullStrategy = "ff-only" | "merge" | "rebase";

// ---------- Deploy (capítulo 10) ----------

export type StepStatus = "waiting" | "running" | "done" | "failed" | "skipped";

export interface DeployRunStep {
  position: number;
  name: string;
  status: StepStatus;
  command: string | null;
  exitCode: number | null;
  message: string | null;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface DeployRun {
  id: number;
  profileId: number;
  projectName: string;
  serverName: string;
  strategy: DeployStrategy;
  status: "running" | "success" | "failed" | "cancelled";
  startedAt: string;
  finishedAt: string | null;
  releaseDir: string | null;
  previousRelease: string | null;
  detail: string | null;
  steps: DeployRunStep[];
}

export interface StepEvent {
  runId: number;
  position: number;
  name: string;
  status: StepStatus;
  command: string | null;
  exitCode: number | null;
  message: string | null;
}

export interface LogEvent {
  runId: number;
  position: number;
  stream: "out" | "err" | "cmd";
  text: string;
}

export interface DeployActive {
  runId: number;
  label: string;
}

export interface SecretFinding {
  path: string;
  kind: "nome" | "extensao" | "conteudo" | "rastreado" | "ignorado" | string;
  reason: string;
  severity: "block" | "warn" | string;
  suggestion: string;
}

export interface DiskUsage {
  mount: string;
  totalKb: number;
  usedKb: number;
  availKb: number;
  usePercent: number;
}

export interface ServiceInfo {
  name: string;
  kind: string;
  state: string;
}

/**
 * Métricas lidas do servidor (§12). Todo campo vem de um comando de leitura
 * executado no host; quando o host não respondeu algo, o campo é null e a
 * interface mostra "não informado" — nunca um valor inventado.
 */
export interface ServerMetrics {
  serverId: number;
  serverName: string;
  collectedAt: number;
  hostname: string | null;
  os: string | null;
  kernel: string | null;
  arch: string | null;
  cpuCount: number | null;
  cpuModel: string | null;
  load1: number | null;
  load5: number | null;
  load15: number | null;
  uptimeSeconds: number | null;
  memTotalKb: number | null;
  memAvailableKb: number | null;
  swapTotalKb: number | null;
  swapFreeKb: number | null;
  disks: DiskUsage[];
  services: ServiceInfo[];
  notes: string[];
  warnings: string[];
}

export interface DeployPreflight {
  files: number;
  bytes: number;
  sample: string[];
  findings: SecretFinding[];
  blocked: number;
  warnings: number;
  message: string;
}

// GitHub: todos os campos abaixo vêm de respostas reais de api.github.com. O
// token da conta fica apenas na memória do backend e nunca chega à interface.
export interface GitHubStatus {
  hasClientId: boolean;
  loggedIn: boolean;
  login: string | null;
}

export interface GitHubDeviceFlow {
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}

export interface GitHubConnectResult {
  status: "pending" | "connected" | "error";
  message: string | null;
  login: string | null;
}

export interface GitHubRepo {
  name: string;
  fullName: string;
  private: boolean;
  defaultBranch: string;
  htmlUrl: string;
  cloneUrl: string;
  sshUrl: string;
  sizeKb: number;
  language: string | null;
  pushedAt: string | null;
  archived: boolean;
  fork: boolean;
}

export interface GitHubPull {
  number: number;
  title: string;
  state: string;
  author: string | null;
  headRef: string;
  baseRef: string;
  draft: boolean;
  updatedAt: string | null;
  htmlUrl: string;
}

export type ViewId =
  | "dashboard"
  | "servers"
  | "terminal"
  | "files"
  | "git"
  | "projects"
  | "github"
  | "deploy"
  | "history"
  | "settings";
