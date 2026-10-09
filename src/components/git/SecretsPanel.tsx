import { useState } from "react";
import type { GitRepo } from "../../lib/useGitRepo";
import type { SecretFinding } from "../../types";
import { AlertTriangleIcon, KeyIcon, RefreshIcon, SearchIcon, XIcon } from "../Icons";
import { Badge, Button, Card, EmptyState, SectionTitle, Spinner } from "../ui";

type Scan = { label: string; findings: SecretFinding[] };

const KIND_LABELS: Record<string, string> = {
  nome: "nome do arquivo",
  extensao: "extensão",
  conteudo: "conteúdo",
  rastreado: "já no índice",
  ignorado: "ignorado",
};

/**
 * Painel da §8: varre a pasta do projeto e o que o Git está prestes a publicar.
 * Nada aqui é simulado — cada lista vem do conteúdo lido do índice, do HEAD ou
 * dos arquivos do disco no momento do clique.
 */
export function SecretsPanel({ git }: { git: GitRepo }) {
  const [scan, setScan] = useState<Scan | null>(null);
  const [running, setRunning] = useState<string | null>(null);

  const run = async (label: string, fn: () => Promise<SecretFinding[] | null>) => {
    setRunning(label);
    const found = await fn();
    setRunning(null);
    if (found !== null) setScan({ label, findings: found });
  };

  const blockCount = scan ? scan.findings.filter((f) => f.severity === "block").length : 0;
  const busy = git.busy !== null || running !== null;

  return (
    <div className="space-y-3">
      <Card className="p-3">
        <SectionTitle>Verificação de segredos</SectionTitle>
        <p className="mt-1 text-[11px] text-zinc-600 dark:text-zinc-400">
          O verificador julga nome, extensão, conteúdo e o que já está no índice do Git. Ele é usado
          automaticamente antes de cada commit e de cada envio — aqui você roda a mesma checagem quando
          quiser. Regras extras (um padrão por linha) ficam em Configurações → Proteção de segredos.
        </p>
        <div className="mt-2 flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="primary"
            disabled={busy}
            onClick={() => void run("Pasta do projeto", git.scanSecrets)}
          >
            {running === "Pasta do projeto" ? <Spinner className="size-3.5" /> : <SearchIcon className="size-3.5" />}
            Varrer a pasta
          </Button>
          <Button size="sm" disabled={busy} onClick={() => void run("Índice (próximo commit)", () => git.secretCheck("index"))}>
            {running === "Índice (próximo commit)" ? <Spinner className="size-3.5" /> : <RefreshIcon className="size-3.5" />}
            Checar o índice
          </Button>
          <Button size="sm" disabled={busy} onClick={() => void run("Conteúdo do HEAD", () => git.secretCheck("push"))}>
            {running === "Conteúdo do HEAD" ? <Spinner className="size-3.5" /> : <AlertTriangleIcon className="size-3.5" />}
            Checar o que o envio publicaria
          </Button>
        </div>
      </Card>

      {scan === null ? (
        <Card>
          <EmptyState
            icon={<KeyIcon className="size-8" />}
            title="Nenhuma verificação nesta visita"
            description="Escolha uma das checagens acima. Os resultados vêm do Git e dos arquivos reais desta pasta."
          />
        </Card>
      ) : (
        <Card className="p-3">
          <div className="flex flex-wrap items-center gap-2">
            <SectionTitle>{scan.label}</SectionTitle>
            {scan.findings.length === 0 ? (
              <Badge tone="success">nada suspeito</Badge>
            ) : (
              <>
                <Badge tone={blockCount > 0 ? "error" : "warning"}>{blockCount} bloqueante(s)</Badge>
                <Badge>{scan.findings.length - blockCount} aviso(s)</Badge>
              </>
            )}
            <Button size="sm" variant="ghost" className="ml-auto" disabled={busy} onClick={() => setScan(null)}>
              <XIcon className="size-3.5" />
            </Button>
          </div>
          {scan.findings.length === 0 ? (
            <p className="mt-2 text-xs text-zinc-600 dark:text-zinc-400">
              Nenhum arquivo correspondeu às regras de credencial, chave ou cache.
            </p>
          ) : (
            <ul className="mt-2 divide-y divide-zinc-100 dark:divide-zinc-800">
              {scan.findings.map((f) => (
                <li key={`${f.path}#${f.kind}#${f.reason}`} className="flex items-start gap-2 py-2">
                  <div className="min-w-0 flex-1">
                    <p className="flex items-center gap-2">
                      <span className="truncate font-mono text-[11px]" title={f.path}>
                        {f.path}
                      </span>
                      <Badge tone={f.severity === "block" ? "error" : "warning"}>
                        {KIND_LABELS[f.kind] ?? f.kind}
                      </Badge>
                    </p>
                    <p className="mt-0.5 text-[11px] text-zinc-600 dark:text-zinc-400">{f.reason}</p>
                    <p className="text-[11px] text-zinc-500 dark:text-zinc-500">{f.suggestion}</p>
                  </div>
                  {f.severity === "block" ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      title="git rm --cached: o arquivo local continua na pasta"
                      onClick={() => void git.untrackFile(f.path)}
                    >
                      <XIcon className="size-3.5" /> Tirar do índice
                    </Button>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
          <p className="mt-3 text-[11px] text-zinc-500 dark:text-zinc-400">
            Estar no <span className="font-mono">.gitignore</span> não remove um arquivo do índice nem do
            histórico. “Tirar do índice” roda <span className="font-mono">git rm --cached</span> e lista o
            caminho no <span className="font-mono">.gitignore</span>; o histórico só muda com reescrita
            explícita, que esta aplicação nunca faz sozinha.
          </p>
        </Card>
      )}
    </div>
  );
}
