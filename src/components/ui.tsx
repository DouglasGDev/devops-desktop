import {
  useEffect,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from "react";
import { LoaderIcon, XIcon } from "./Icons";

type ButtonVariant = "primary" | "secondary" | "danger" | "ghost";
type ButtonSize = "sm" | "md";

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary: "bg-teal-600 text-white hover:bg-teal-500",
  secondary:
    "border border-zinc-300 bg-white text-zinc-800 hover:bg-zinc-100 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-200 dark:hover:bg-zinc-800",
  danger:
    "border border-red-200 bg-white text-red-600 hover:bg-red-50 dark:border-red-900/60 dark:bg-zinc-900 dark:text-red-400 dark:hover:bg-red-950/40",
  ghost: "text-zinc-600 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-800",
};

const BUTTON_SIZES: Record<ButtonSize, string> = {
  sm: "px-2.5 py-1 text-xs",
  md: "px-3.5 py-1.5 text-sm",
};

export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  children,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}) {
  return (
    <button
      {...rest}
      disabled={rest.disabled || loading}
      className={`inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition disabled:pointer-events-none disabled:opacity-50 ${BUTTON_VARIANTS[variant]} ${BUTTON_SIZES[size]} ${className}`}
    >
      {loading ? <LoaderIcon className="size-3.5 animate-spin" /> : null}
      {children}
    </button>
  );
}

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={`rounded-xl border border-zinc-200 bg-white dark:border-zinc-800 dark:bg-zinc-900 ${className}`}>
      {children}
    </div>
  );
}

export type BadgeTone = "neutral" | "accent" | "success" | "error" | "warning" | "info" | "violet";

const BADGE_TONES: Record<BadgeTone, string> = {
  neutral: "bg-zinc-100 text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300",
  accent: "bg-teal-100 text-teal-800 dark:bg-teal-950 dark:text-teal-300",
  success: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300",
  error: "bg-red-100 text-red-700 dark:bg-red-950 dark:text-red-300",
  warning: "bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300",
  info: "bg-sky-100 text-sky-800 dark:bg-sky-950 dark:text-sky-300",
  violet: "bg-violet-100 text-violet-800 dark:bg-violet-950 dark:text-violet-300",
};

export function Badge({
  tone = "neutral",
  title,
  children,
}: {
  tone?: BadgeTone;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title}
      className={`inline-flex items-center whitespace-nowrap rounded-md px-2 py-0.5 text-xs font-medium ${BADGE_TONES[tone]}`}
    >
      {children}
    </span>
  );
}

export const ENV_LABELS: Record<string, string> = {
  dev: "Desenvolvimento",
  staging: "Homologação",
  prod: "Produção",
  database: "Banco de dados",
  docker: "Docker",
  other: "Outro",
};

export const STRATEGY_LABELS: Record<string, string> = {
  sftp: "Modo A — sincronizar por SFTP",
  git: "Modo B — Git no servidor",
  artifacts: "Modo C — build local e enviar artefatos",
  script: "Modo D — sequência de scripts",
  docker: "Modo E — Docker / Compose",
};

export const STRATEGY_HINTS: Record<string, string> = {
  sftp: "Envia somente arquivos novos ou alterados do diretório local para o destino. Nada é apagado sem uma regra explícita.",
  git: "O servidor atualiza o repositório pela branch configurada. Mudanças locais não previstas são detectadas antes de sobrescrever.",
  artifacts: "Compila no computador e envia apenas os artefatos listados. Nada do node_modules ou do código-fonte vai ao servidor.",
  script: "Executa a sequência de comandos que você configurou, com log e código de saída de cada etapa.",
  docker: "Build e atualização de containers. O servidor precisa ter Docker (e Compose, quando usado) — isso é verificado antes.",
};

export const ENV_TONES: Record<string, BadgeTone> = {
  dev: "info",
  staging: "warning",
  prod: "error",
  database: "violet",
  docker: "info",
  other: "neutral",
};

/** Uma publicação sem servidor tem um único destino: o computador em uso. */
export const DESTINO_LOCAL = "nesta máquina";

export const AUTH_LABELS: Record<string, string> = {
  key: "Chave privada",
  agent: "SSH Agent",
  password: "Senha (por prompt)",
  unknown: "Não informado",
};

export const LEVEL_TONES: Record<string, BadgeTone> = {
  info: "neutral",
  success: "success",
  warning: "warning",
  error: "error",
};

export const LEVEL_LABELS: Record<string, string> = {
  info: "Info",
  success: "Sucesso",
  warning: "Aviso",
  error: "Erro",
};

export const CATEGORY_LABELS: Record<string, string> = {
  connection: "Conexão",
  server: "Servidor",
  project: "Projeto",
  settings: "Configurações",
  git: "Git",
  github: "GitHub",
  deploy: "Deploy",
  files: "Arquivos",
  transfer: "Transferência",
  security: "Segurança",
  terminal: "Terminal",
  app: "Aplicativo",
  análise: "Análise",
};

export function Modal({
  open,
  onClose,
  title,
  children,
  width = "max-w-lg",
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  width?: string;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className={`w-full ${width} rounded-xl border border-zinc-200 bg-white shadow-2xl dark:border-zinc-800 dark:bg-zinc-900`}>
        <div className="flex items-center justify-between border-b border-zinc-200 px-5 py-3.5 dark:border-zinc-800">
          <h2 className="text-sm font-semibold">{title}</h2>
          <button
            onClick={onClose}
            aria-label="Fechar"
            className="rounded-md p-1 text-zinc-500 transition hover:bg-zinc-100 hover:text-zinc-800 dark:hover:bg-zinc-800 dark:hover:text-zinc-200"
          >
            <XIcon className="size-4" />
          </button>
        </div>
        <div className="max-h-[70vh] overflow-y-auto px-5 py-4">{children}</div>
      </div>
    </div>
  );
}

export function Confirm({
  open,
  title,
  message,
  confirmLabel = "Confirmar",
  busy = false,
  onConfirm,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  message: string;
  confirmLabel?: string;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
  /** controles extras da confirmação (por exemplo, uma opção de força) */
  children?: ReactNode;
}) {
  return (
    <Modal open={open} onClose={busy ? () => {} : onClose} title={title} width="max-w-md">
      <p className="text-sm text-zinc-600 dark:text-zinc-300">{message}</p>
      {children ? <div className="mt-3">{children}</div> : null}
      <div className="mt-5 flex justify-end gap-2">
        <Button onClick={onClose} disabled={busy}>
          Cancelar
        </Button>
        <Button variant="danger" onClick={onConfirm} loading={busy}>
          {confirmLabel}
        </Button>
      </div>
    </Modal>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs font-medium text-zinc-500 dark:text-zinc-400">{label}</span>
      {children}
      {hint ? <span className="mt-1 block text-[11px] text-zinc-400 dark:text-zinc-500">{hint}</span> : null}
    </label>
  );
}

const inputClasses =
  "w-full rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-sm text-zinc-900 placeholder:text-zinc-400 focus:outline-none focus:ring-2 focus:ring-teal-500/50 dark:border-zinc-700 dark:bg-zinc-950 dark:text-zinc-100 dark:placeholder:text-zinc-600";

export function Input({ className = "", ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...rest} className={`${inputClasses} ${className}`} />;
}

export function Select({ className = "", children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select {...rest} className={`${inputClasses} ${className}`}>
      {children}
    </select>
  );
}

export function Textarea({ className = "", ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...rest} className={`${inputClasses} ${className}`} />;
}

export function Checkbox({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: ReactNode;
}) {
  return (
    <label className="inline-flex cursor-pointer items-start gap-2 text-sm">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 size-4 accent-teal-600"
      />
      <span>{label}</span>
    </label>
  );
}

export function Spinner({ className = "size-4" }: { className?: string }) {
  return <LoaderIcon className={`${className} animate-spin`} />;
}

export function StatusDot({ status }: { status: string | null }) {
  const tone = status === "ok" ? "bg-emerald-500" : status === "failed" ? "bg-red-500" : "bg-zinc-400 dark:bg-zinc-600";
  return <span className={`inline-block size-2 shrink-0 rounded-full ${tone}`} />;
}

export function SectionTitle({ children }: { children: ReactNode }) {
  return (
    <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-zinc-400 dark:text-zinc-500">{children}</h3>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  children,
}: {
  icon?: ReactNode;
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-14 text-center">
      {icon ? <div className="text-zinc-300 dark:text-zinc-600">{icon}</div> : null}
      <p className="text-sm font-medium">{title}</p>
      {description ? <p className="max-w-sm text-xs text-zinc-500 dark:text-zinc-400">{description}</p> : null}
      {children ? <div className="mt-2 flex gap-2">{children}</div> : null}
    </div>
  );
}
