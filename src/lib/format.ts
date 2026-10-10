export function formatDateTime(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString("pt-BR", {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Tempo relativo a partir do ISO do GitHub — `há 3 h`, sem data absoluta. */
export function formatAgo(iso: string | null): string {
  if (!iso) return "—";
  const ms = new Date(iso).getTime();
  if (Number.isNaN(ms)) return "—";
  return formatRelative(Math.floor(ms / 1000));
}

export function formatTimestamp(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleString("pt-BR", {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatRelative(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) return "—";
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (delta < 45) return "agora";
  if (delta < 3600) return `há ${Math.round(delta / 60)} min`;
  if (delta < 86400) return `há ${Math.round(delta / 3600)} h`;
  if (delta < 2592000) return `há ${Math.round(delta / 86400)} d`;
  if (delta < 31536000) return `há ${Math.round(delta / 2592000)} mês`;
  return `há ${Math.round(delta / 31536000)} ano(s)`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

export const ERROR_KIND_LABELS: Record<string, string> = {
  auth: "Falha de autenticação",
  dns: "Host não resolvido (DNS)",
  refused: "Conexão recusada (porta fechada?)",
  timeout: "Tempo limite excedido",
  hostkey: "Chave de host desconhecida ou alterada",
  network: "Sem rota para o host",
  other: "Falha na conexão",
};

const SIZE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function formatBytes(n: number | null | undefined): string {
  if (n == null || n < 0) return "—";
  if (n < 1024) return `${n} B`;
  let v = n;
  let i = 0;
  while (v >= 1024 && i < SIZE_UNITS.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${SIZE_UNITS[i]}`;
}

// O backend lê memória e disco em kilobytes (/proc/meminfo, df -P).
export function formatKilobytes(kb: number | null | undefined): string {
  if (kb == null || kb < 0) return "—";
  return formatBytes(kb * 1024);
}

export function formatUptime(seconds: number | null | undefined): string {
  if (seconds == null || seconds < 0) return "—";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}min`;
  return `${m}min`;
}

export function formatRate(bytesPerSec: number | null | undefined): string {
  if (bytesPerSec == null || bytesPerSec <= 0) return "—";
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds == null || seconds < 0) return "—";
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function joinRemotePath(dir: string, name: string): string {
  const alvo = name.replace(/^\/+/, "");
  if (dir === "/" || dir.endsWith("/")) return `${dir}${alvo}`;
  return `${dir}/${alvo}`;
}

export function parentRemotePath(dir: string): string | null {
  const trimmed = dir.replace(/\/+$/, "");
  if (!trimmed || trimmed === "") return null;
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return "/";
  return trimmed.slice(0, idx);
}

export function remoteBasename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}
