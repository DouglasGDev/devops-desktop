import { useCallback, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { apontaPara } from "./repo";
import { useApp } from "../store/AppContext";
import type { Project } from "../types";

/**
 * Aponta a pasta desta máquina que é o clone de um repositório: o seletor nativo
 * escolhe, o `add_project` sonda com o git real, e o veredito é o que o git
 * respondeu — não uma suposição de que a pasta "deve" ser daquele repo.
 */
export function useMapearPasta() {
  const { pushToast, reloadProjects } = useApp();
  const [mapeando, setMapeando] = useState(false);

  const mapear = useCallback(
    async (slug: string): Promise<Project | null> => {
      const dir = await open({ directory: true, multiple: false, title: "Onde este repositório está nesta máquina" });
      if (typeof dir !== "string") return null;
      setMapeando(true);
      try {
        const p = await api.addProject(dir, null);
        await reloadProjects();
        if (apontaPara(p.gitRemote, slug)) {
          pushToast("success", `Mapeado: ${p.name} está em ${p.localPath}`);
        } else if (!p.isGitRepo) {
          pushToast("info", `${p.localPath} não é um repositório git — ficou em Projetos, não como clone daqui.`);
        } else {
          pushToast("info", `${p.name} tem origin ${p.gitRemote ?? "nenhum"}, que não é ${slug}.`);
        }
        return p;
      } catch (e) {
        pushToast("error", String(e));
        return null;
      } finally {
        setMapeando(false);
      }
    },
    [pushToast, reloadProjects],
  );

  return { mapear, mapeando };
}
