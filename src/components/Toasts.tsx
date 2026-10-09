import type { ReactNode } from "react";
import { AlertTriangleIcon, CheckIcon, InfoIcon } from "./Icons";
import { useApp } from "../store/AppContext";

const KIND_STYLES: Record<string, { border: string; icon: ReactNode }> = {
  success: { border: "border-l-emerald-500", icon: <CheckIcon className="mt-0.5 size-4 shrink-0 text-emerald-500" /> },
  error: { border: "border-l-red-500", icon: <AlertTriangleIcon className="mt-0.5 size-4 shrink-0 text-red-500" /> },
  info: { border: "border-l-sky-500", icon: <InfoIcon className="mt-0.5 size-4 shrink-0 text-sky-500" /> },
};

export function Toasts() {
  const { toasts } = useApp();
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[70] flex w-96 max-w-[calc(100vw-2rem)] flex-col gap-2">
      {toasts.map((t) => {
        const style = KIND_STYLES[t.kind] ?? KIND_STYLES.info;
        return (
          <div
            key={t.id}
            className={`pointer-events-auto flex items-start gap-2.5 rounded-lg border border-zinc-200 border-l-4 bg-white p-3 text-sm shadow-lg dark:border-zinc-700 dark:bg-zinc-900 ${style.border}`}
          >
            {style.icon}
            <p className="flex-1 break-words">{t.message}</p>
          </div>
        );
      })}
    </div>
  );
}
