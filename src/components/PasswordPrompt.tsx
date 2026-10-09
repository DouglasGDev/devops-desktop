import { useEffect, useState } from "react";
import type { AuthRequest } from "../types";
import { useApp } from "../store/AppContext";
import { Button, Input, Modal } from "./ui";

// Prompt de senha por operação. A senha existe apenas no estado deste componente
// enquanto o modal está aberto: não vai para o SQLite, para o histórico, para os
// logs nem para disco.
function Prompt({ request, remaining }: { request: AuthRequest; remaining: number }) {
  const { answerAuth, declineAuth } = useApp();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);

  const close = () => {
    setPassword("");
    void declineAuth(request.id);
  };

  const submit = async () => {
    const value = password;
    setPassword("");
    if (!value) return;
    setBusy(true);
    await answerAuth(request.id, value);
    setBusy(false);
  };

  return (
    <Modal open onClose={busy ? () => {} : close} title="Senha do servidor" width="max-w-md">
      <div className="space-y-3">
        <p className="text-sm text-zinc-600 dark:text-zinc-300">
          <span className="font-medium">{request.serverName}</span> autentica por senha. A{" "}
          {request.purpose.toLowerCase()} usa a senha só nesta execução e a descarta no fim da
          operação.
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
          className="space-y-3"
        >
          <Input
            autoFocus
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            placeholder="Senha"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            disabled={busy}
          />
          <p className="text-[11px] text-zinc-500 dark:text-zinc-400">
            {remaining > 0
              ? `A solicitação expira em ${remaining}s. A verificação da chave de host continua obrigatória.`
              : "A solicitação expirou."}
          </p>
          <div className="flex justify-end gap-2 pt-1">
            <Button type="button" onClick={close} disabled={busy}>
              Cancelar
            </Button>
            <Button type="submit" variant="primary" loading={busy}>
              Conectar
            </Button>
          </div>
        </form>
      </div>
    </Modal>
  );
}

export function PasswordPrompt() {
  const { authRequests, declineAuth } = useApp();
  const [remaining, setRemaining] = useState(0);
  const request = authRequests[0];

  useEffect(() => {
    if (!request) return;
    const deadline = Date.now() + request.timeoutSecs * 1000;
    const tick = () => {
      const left = Math.max(0, Math.round((deadline - Date.now()) / 1000));
      setRemaining(left);
      if (left === 0) void declineAuth(request.id);
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [request, declineAuth]);

  if (!request) return null;
  return <Prompt key={request.id} request={request} remaining={remaining} />;
}
