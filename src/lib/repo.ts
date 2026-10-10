/**
 * O git aceita várias formas para o mesmo remoto (`https://…/repo.git`,
 * `git@…:repo.git`, `ssh://git@…/repo`). Aqui tudo vira `dono/repositorio` para
 * comparison, sem presumir o host.
 */
export function chaveDoRemoto(url: string | null | undefined): string | null {
  if (!url) return null;
  const partes = url
    .trim()
    .split(/[/\\:]+/)
    .filter((p) => p !== "");
  if (partes.length < 3) return null;
  const dono = partes[partes.length - 2];
  const nome = partes[partes.length - 1].replace(/\.git$/i, "");
  if (dono === "" || nome === "") return null;
  return `${dono}/${nome}`.toLowerCase();
}

export function apontaPara(url: string | null | undefined, slug: string): boolean {
  return chaveDoRemoto(url) === slug.trim().toLowerCase();
}
