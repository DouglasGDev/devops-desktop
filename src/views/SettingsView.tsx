import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { GitEditor, ToolVersions } from "../types";
import { useApp } from "../store/AppContext";
import { ExternalLinkIcon } from "../components/Icons";
import { MachineAccess } from "../components/MachineAccess";
import { Badge, Button, Card, Checkbox, Field, Select, SectionTitle, Spinner, Textarea } from "../components/ui";

const EDITORS: GitEditor[] = ["system", "vscode", "cursor"];

/** Nomes dos editores que a aplicação sabe abrir, para não depender da detecção
 * quando o nome precisa aparecer sem o programa estar instalado. */
const NOMES_DOS_EDITORES: Partial<Record<GitEditor, string>> = {
  vscode: "Visual Studio Code",
  cursor: "Cursor",
};

export function SettingsView() {
  const { settings, setSetting, pushToast } = useApp();
  const [tools, setTools] = useState<ToolVersions | null>(null);
  const [toolsLoading, setToolsLoading] = useState(false);
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const loadTools = useCallback(async () => {
    setToolsLoading(true);
    try {
      setTools(await api.getToolVersions());
      setDataDir(await api.getAppDataDir());
    } catch (e) {
      pushToast("error", `Falha ao detectar ferramentas: ${String(e)}`);
    } finally {
      setToolsLoading(false);
    }
  }, [pushToast]);

  useEffect(() => {
    void loadTools();
  }, [loadTools]);

  const toggle = async (key: string, value: boolean) => {
    setBusyKey(key);
    try {
      await setSetting(key, value ? "true" : "false");
    } catch (e) {
      pushToast("error", `Falha ao salvar preferência: ${String(e)}`);
    } finally {
      setBusyKey(null);
    }
  };

  // Regras extras do verificador de segredos (§8). `null` = nada digitado ainda,
  // então o campo mostra o valor salvo no banco.
  const [patterns, setPatterns] = useState<string | null>(null);
  const patternsValue = patterns ?? settings.secret_patterns ?? "";
  const patternsDirty = patterns !== null && patterns !== (settings.secret_patterns ?? "");

  const savePatterns = async () => {
    setBusyKey("secret_patterns");
    try {
      await setSetting("secret_patterns", patternsValue);
      setPatterns(null);
      pushToast("info", "Regras extras aplicáveis à próxima verificação.");
    } catch (e) {
      pushToast("error", `Falha ao salvar as regras: ${String(e)}`);
    } finally {
      setBusyKey(null);
    }
  };

  const theme = settings.theme === "light" ? "light" : "dark";
  const confirmDestructive = settings.confirm_destructive !== "false";
  const prodWarnings = settings.prod_warnings !== "false";
  const historyEnabled = settings.terminal_history_enabled !== "false";
  const editor: GitEditor = EDITORS.includes(settings.git_editor as GitEditor)
    ? (settings.git_editor as GitEditor)
    : "system";
  // A lista é o que existe neste computador, não um catálogo fixo.
  const instalados = (tools?.editors ?? []).filter((e) => e.id !== "system");
  const escolhaAusente = editor !== "system" && !instalados.some((e) => e.id === editor);

  return (
    <div className="max-w-3xl space-y-4">
      <Card className="p-5">
        <SectionTitle>Aparência</SectionTitle>
        <Field label="Tema da interface">
          <Select className="max-w-xs" value={theme} onChange={(e) => void setSetting("theme", e.target.value)}>
            <option value="dark">Escuro</option>
            <option value="light">Claro</option>
          </Select>
        </Field>
      </Card>

      <Card className="p-5">
        <SectionTitle>Segurança e confirmações</SectionTitle>
        <div className="space-y-3">
          <Checkbox
            checked={confirmDestructive}
            onChange={(v) => void toggle("confirm_destructive", v)}
            label={
              <span>
                Confirmar antes de operações potencialmente destrutivas
                {busyKey === "confirm_destructive" ? <Spinner className="ml-1 inline size-3.5" /> : null}
              </span>
            }
          />
          <Checkbox
            checked={prodWarnings}
            onChange={(v) => void toggle("prod_warnings", v)}
            label={
              <span>
                Aviso reforçado para operações em servidores de produção
                {busyKey === "prod_warnings" ? <Spinner className="ml-1 inline size-3.5" /> : null}
              </span>
            }
          />
          <Checkbox
            checked={historyEnabled}
            onChange={(v) => void toggle("terminal_history_enabled", v)}
            label={
              <span>
                Manter histórico local dos comandos executados no terminal
                {busyKey === "terminal_history_enabled" ? <Spinner className="ml-1 inline size-3.5" /> : null}
              </span>
            }
          />
        </div>
        <p className="mt-3 text-xs text-zinc-500 dark:text-zinc-400">
          Estas preferências são lidas pelo backend em todas as etapas futuras (transferências, deploys e Git). Senhas, tokens e
          chaves privadas nunca são armazenados no banco de dados — a autenticação usa o SSH Agent e as chaves existentes. O
          histórico de comandos é gravado já sem segredos detectáveis.
        </p>
      </Card>

      <Card className="p-5">
        <SectionTitle>Git</SectionTitle>
        <Field label="Editor usado para abrir repositórios e arquivos">
          <Select className="max-w-xs" value={editor} onChange={(e) => void setSetting("git_editor", e.target.value)}>
            <option value="system">Editor padrão do sistema</option>
            {instalados.map((e) => (
              <option key={e.id} value={e.id}>
                {e.nome}
              </option>
            ))}
            {escolhaAusente ? (
              <option value={editor}>{`${NOMES_DOS_EDITORES[editor] ?? editor} (não encontrado)`}</option>
            ) : null}
          </Select>
        </Field>
        {tools === null ? (
          <p className="mt-2 flex items-center gap-2 text-xs text-zinc-500 dark:text-zinc-400">
            <Spinner className="size-3.5" /> Procurando os editores instalados nesta máquina…
          </p>
        ) : instalados.length === 0 ? (
          <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
            Nenhum editor dos que a aplicação sabe abrir foi encontrado neste computador. Use o editor padrão do sistema
            ou instale um dos suportados — a lista é refeita a cada “Reverificar”.
          </p>
        ) : null}
        {escolhaAusente ? (
          <p className="mt-2 text-xs text-amber-700 dark:text-amber-400">
            A escolha salva não está instalada aqui. O botão “Abrir no editor” ainda tenta o atalho do programa e, se ele
            não existir, diz que falhou em vez de abrir outra coisa.
          </p>
        ) : null}
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          O botão “Abrir no editor” das seções de Git usa esta escolha. Com o editor padrão do sistema o caminho é
          entregue ao próprio sistema; com um editor detectado o aplicativo abre direto no arquivo do repositório.
        </p>
      </Card>

      <Card className="p-5">
        <SectionTitle>Proteção de segredos</SectionTitle>
        <Field
          label="Regras extras do verificador (um padrão por linha, comparado no nome do arquivo)"
          hint="Some às regras fixas: .env e variantes, chaves privadas, certificados, arquivos de credencial, tokens com prefixo conhecido e atribuições de senha. Usado no commit, no push, no deploy e na varredura da seção Git."
        >
          <Textarea
            className="mt-1 font-mono text-xs"
            rows={4}
            value={patternsValue}
            placeholder={"token-prod\nfirefly-secret"}
            onChange={(e) => setPatterns(e.target.value)}
          />
        </Field>
        <div className="mt-2 flex items-center gap-2">
          <Button size="sm" variant="primary" disabled={!patternsDirty || busyKey === "secret_patterns"} onClick={() => void savePatterns()}>
            Salvar regras
          </Button>
          {patternsDirty ? <Badge tone="warning">ainda não salvo</Badge> : null}
        </div>
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          O verificador bloqueia commit e push enquanto houver candidato a segredo no conteúdo que seria publicado, e o
          deploy recusa o envio dos mesmos arquivos. Em todos os casos a autorização é explícita, feita por você na
          janela do aviso. Estar no <span className="font-mono">.gitignore</span> não tira um arquivo do índice nem do
          histórico; a aplicação oferece a remoção do índice sem apagar a cópia local, mas nunca reescreve histórico
          remoto por conta própria.
        </p>
      </Card>

      <MachineAccess tools={tools} loading={toolsLoading} onRescan={() => void loadTools()} />

      <Card className="p-5">
        <SectionTitle>Dados locais</SectionTitle>
        <p className="text-sm">Diretório de dados da aplicação:</p>
        <p className="mt-1 break-all rounded-lg bg-zinc-100 px-3 py-2 font-mono text-xs dark:bg-zinc-800">{dataDir ?? "—"}</p>
        <p className="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
          O banco SQLite (servidores, projetos, preferências e histórico de auditoria) fica neste diretório. Nenhum dado sai
          deste computador.
        </p>
      </Card>

      <Card className="p-5">
        <SectionTitle>Sobre</SectionTitle>
        <p className="text-sm font-medium">DevOps Desktop</p>
        <p className="text-xs text-zinc-500 dark:text-zinc-400">
          Central de SSH, Git e Deploy · versão 0.1.0 · Rust + Tauri 2 + React
        </p>
        <p className="mt-2 flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
          Código-fonte e documentação
          <ExternalLinkIcon className="size-3.5" />
          disponíveis no repositório público do projeto.
        </p>
      </Card>
    </div>
  );
}
