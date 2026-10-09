import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../../lib/api";
import { useApp } from "../../store/AppContext";
import { FolderIcon } from "../Icons";
import { Button, Field, Input, Modal } from "../ui";

/**
 * Clone de um repositório em uma pasta escolhida.
 *
 * Registrar o projeto continua sendo um passo separado, na seção Projetos: a
 * aplicação não cadastra nada automaticamente depois do clone.
 */
export function CloneDialog({ open: isOpen, onClose }: { open: boolean; onClose: () => void }) {
  const { reloadProjects, pushToast } = useApp();
  const [parentPath, setParentPath] = useState("");
  const [url, setUrl] = useState("");
  const [targetName, setTargetName] = useState("");
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);

  const pick = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Onde clonar o repositório" });
    if (typeof dir === "string" && dir.trim() !== "") setParentPath(dir);
  };

  const run = async () => {
    setBusy(true);
    try {
      const created = await api.gitClone({
        parentPath,
        url: url.trim(),
        targetName: targetName.trim() === "" ? null : targetName.trim(),
        branch: branch.trim() === "" ? null : branch.trim(),
      });
      pushToast("success", created);
      onClose();
      setUrl("");
      setTargetName("");
      setBranch("");
      await reloadProjects();
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal open={isOpen} onClose={busy ? () => {} : onClose} title="Clonar repositório">
      <div className="grid gap-3">
        <Field label="Endereço do repositório" hint="https://, ssh:// ou usuário@host:caminho.">
          <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://github.com/usuario/repositorio.git" />
        </Field>
        <Field label="Clonar dentro de" hint="O git cria a pasta do projeto aí dentro.">
          <div className="flex gap-2">
            <Input value={parentPath} onChange={(e) => setParentPath(e.target.value)} placeholder="E:\projetos" />
            <Button onClick={() => void pick()} disabled={busy}>
              <FolderIcon className="size-4" /> Pasta…
            </Button>
          </div>
        </Field>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="Nome da pasta (opcional)">
            <Input value={targetName} onChange={(e) => setTargetName(e.target.value)} placeholder="nome do repositório" />
          </Field>
          <Field label="Branch inicial (opcional)">
            <Input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="main" />
          </Field>
        </div>
      </div>
      <p className="mt-3 text-[11px] text-zinc-500 dark:text-zinc-400">
        A autenticação do clone é a do seu git (gerenciador de credenciais, chaves ou agente SSH). Se ele pedir senha,
        a aplicação não grava nada em disco.
      </p>
      <div className="mt-5 flex justify-end gap-2">
        <Button onClick={onClose} disabled={busy}>
          Cancelar
        </Button>
        <Button variant="primary" disabled={busy || url.trim() === "" || parentPath.trim() === ""} loading={busy} onClick={() => void run()}>
          Clonar
        </Button>
      </div>
    </Modal>
  );
}
