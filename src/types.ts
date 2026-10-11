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

/** Editor encontrado neste computador pela detecção de ferramentas. */
export interface EditorInstalado {
  id: GitEditor;
  nome: string;
  caminho: string;
}

export interface ToolVersions {
  ssh: string | null;
  sshAdd: string | null;
  git: string | null;
  agentRunning: boolean | null;
  /** Chaves carregadas no ssh-agent. */
  agentKeys: number | null;
  /** `user.name` + `user.email` resolvidos pelo git, em qualquer escopo. */
  gitUser: string | null;
  gh: string | null;
  /** Conta ativa do `gh auth status`; nunca o token. */
  ghAccount: string | null;
  /** Programas chamados que não responderam dentro do prazo: existe, só não respondeu. */
  semResposta: string[];
  /** Editores achados no disco. A lista da Configurações vem daqui. */
  editors: EditorInstalado[];
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

/** O que o Git guarda de um arquivo no HEAD: texto, ou a nota dizendo por que
 *  não há texto. */
export interface FileContent {
  path: string;
  text: string | null;
  note: string | null;
  truncated: boolean;
}

/** Operação que o Git deixou parada no meio, lida das marcas do `.git`. */
export type GitOperation = "merge" | "rebase" | "cherry-pick" | "revert";

export interface RepoStatus {
  branch: string | null;
  detachedAt: string | null;
  operation: GitOperation | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: StatusFile[];
  conflictCount: number;
}

/** Um trabalho guardado pelo Git. `index` é a posição na lista do git, onde 0 é
 *  o mais recente. */
export interface StashEntry {
  index: number;
  selector: string;
  sha: string;
  date: string;
  subject: string;
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
  /** `null` é o modo sem servidor: a publicação vai para uma pasta desta máquina. */
  serverId: number | null;
  serverName: string | null;
  serverEnvironment: string | null;
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
  /** Número da definição salva mais recente; cada alteração sobe este número. */
  version: number;
};

export type DeployProfileInput = {
  id: number | null;
  projectId: number;
  name: string;
  /** `null` pede o modo sem servidor — só estratégias que não precisam de host. */
  serverId: number | null;
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
  /** O nó do grafo desta linha; `null` no pipeline linear de quinze etapas. */
  nodeKey: string | null;
  /** Se a linha escreveu no destino — é o que decide se dá para repetir. */
  touchedDestination: boolean;
}

export interface DeployRun {
  id: number;
  profileId: number;
  projectName: string;
  /** `null` quando o modelo publica nesta máquina. */
  serverName: string | null;
  strategy: DeployStrategy;
  status: "running" | "success" | "failed" | "cancelled";
  startedAt: string;
  finishedAt: string | null;
  releaseDir: string | null;
  previousRelease: string | null;
  detail: string | null;
  /** Procedência: de onde esta execução saiu. `null` nas execuções anteriores à v6. */
  definitionVersion: number | null;
  destination: string | null;
  environment: string | null;
  commitSha: string | null;
  steps: DeployRunStep[];
}

/** Uma versão salva de uma definição, com o retrato do que ela era. */
export interface DefinitionRevision {
  version: number;
  savedAt: string;
  spec: Record<string, unknown>;
}

/** Uma definição como o contrato a entrega: o modelo salvo e a última execução dele. */
export interface DefinitionSummary {
  definition: DeployProfile;
  lastRunId: number | null;
  lastRunStatus: DeployRun["status"] | null;
  lastRunAt: string | null;
}

/** Se vale oferecer uma nova tentativa — e por que não vale. */
export interface RetryPolicy {
  allowed: boolean;
  reason: string | null;
}

// ---------- Grafo do pipeline (plano CI/CD, etapa 3) ----------

/** Os três tipos que o motor sabe executar: uma etapa dele, um comando, uma espera. */
export type NodeKind = "motor" | "comando" | "aprovacao";

export type NodeExecutor = "local" | "ssh";

/** Uma das quinze etapas do motor, com o nome que o motor dá a ela no modo do modelo. */
export interface EngineStep {
  position: number;
  name: string;
}

/** Um nó como está salvo na definição. É o que o motor percorre. */
export interface PipelineNode {
  id: number;
  profileId: number;
  nodeKey: string;
  title: string;
  kind: NodeKind;
  /** Etapa do motor (1..15) que este nó executa; só faz sentido em `motor`. */
  engineStep: number | null;
  executor: NodeExecutor;
  environment: string | null;
  workingDir: string | null;
  commands: string[];
  dependsOn: string[];
  parallel: boolean;
  /** 0 = herdar o prazo do modelo. */
  timeoutSecs: number;
  retryLimit: number;
  /** `ramo=<nome>` ou `ambiente=<nome>`; `null` = executa sempre. */
  condition: string | null;
  requiresApproval: boolean;
  /** Derivado do tipo e do executor pelo backend — a tela lê, não edita. */
  touchesDestination: boolean;
  x: number;
  y: number;
}

/** O que a tela envia para salvar. `touchesDestination` é derivado, não digitado. */
export type PipelineNodeInput = {
  nodeKey: string;
  title: string;
  kind: NodeKind;
  engineStep?: number | null;
  executor?: NodeExecutor | null;
  environment?: string | null;
  workingDir?: string | null;
  commands?: string[] | null;
  dependsOn?: string[] | null;
  parallel?: boolean | null;
  timeoutSecs?: number | null;
  retryLimit?: number | null;
  condition?: string | null;
  requiresApproval?: boolean | null;
  x?: number | null;
  y?: number | null;
};

/** Um pedido de aprovação que o motor emitiu e está esperando. */
export interface ApprovalRequest {
  id: number;
  runId: number;
  nodeKey: string;
  title: string;
  definition: string;
  environment: string;
  destination: string;
  waitsSecs: number;
}

export interface StepEvent {
  runId: number;
  position: number;
  name: string;
  status: StepStatus;
  command: string | null;
  exitCode: number | null;
  message: string | null;
  /** O nó do grafo que esta linha é; `null` no pipeline linear. */
  nodeKey: string | null;
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

// Workspace do GitHub: cada campo abaixo é lido de uma rota real de
// api.github.com no momento em que a tela mostra.

export interface GitHubSearchItem {
  number: number;
  title: string;
  /** `dono/repo`, embutido na resposta da busca. */
  repo: string;
  author: string | null;
  state: string;
  isPullRequest: boolean;
  draft: boolean;
  url: string;
  updatedAt: string | null;
}

export interface GitHubInbox {
  reviewRequests: GitHubSearchItem[];
  myPulls: GitHubSearchItem[];
  myIssues: GitHubSearchItem[];
}

export interface GitHubPullDetail {
  number: number;
  title: string;
  state: string;
  author: string | null;
  headRef: string;
  baseRef: string;
  headSha: string;
  draft: boolean;
  /** null enquanto o GitHub calcula; a tela diz isso em vez de afirmar. */
  mergeable: boolean | null;
  mergeableState: string | null;
  additions: number;
  deletions: number;
  changedFiles: number;
  body: string | null;
  updatedAt: string | null;
  url: string;
}

export interface GitHubIssue {
  number: number;
  title: string;
  state: string;
  author: string | null;
  labels: string[];
  comments: number;
  body: string | null;
  updatedAt: string | null;
  url: string;
}

/** Um comentário do fio. `body` nulo é o GitHub dizendo que não há texto. */
export interface GitHubComment {
  author: string | null;
  body: string | null;
  createdAt: string | null;
  url: string;
}

export interface GitHubRun {  id: number;
  name: string;
  displayTitle: string;
  headBranch: string;
  event: string;
  status: string;
  conclusion: string | null;
  createdAt: string | null;
  updatedAt: string | null;
  url: string;
}

export type ReviewEvent = "APPROVE" | "REQUEST_CHANGES" | "COMMENT";
export type MergeMethod = "merge" | "squash" | "rebase";

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
