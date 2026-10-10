import { apontaPara, chaveDoRemoto } from "../src/lib/repo.ts";

const casos: [string, string, boolean][] = [
  ["https://github.com/DouglasGDev/devops-desktop.git", "DouglasGDev/devops-desktop", true],
  ["https://github.com/DouglasGDev/devops-desktop", "DouglasGDev/devops-desktop", true],
  ["git@github.com:DouglasGDev/devops-desktop.git", "DouglasGDev/devops-desktop", true],
  ["ssh://git@github.com/DouglasGDev/devops-desktop.git", "DouglasGDev/devops-desktop", true],
  ["https://github.com/DouglasGDev/devops-desktop/", "DouglasGDev/devops-desktop", true],
  ["https://gitea.interno.local/devops/devops-desktop.git", "devops/devops-desktop", true],
  ["https://github.com/outra-pessoa/devops-desktop.git", "DouglasGDev/devops-desktop", false],
  ["https://github.com/DouglasGDev/outro-projeto.git", "DouglasGDev/devops-desktop", false],
  ["", "DouglasGDev/devops-desktop", false],
];

let falhas = 0;
for (const [url, slug, esperado] of casos) {
  const obtido = apontaPara(url, slug);
  if (obtido !== esperado) {
    falhas += 1;
    console.log(`FALHA  ${url} x ${slug} => ${obtido}, esperado ${esperado}`);
  } else {
    console.log(`ok     ${url || "(vazio)"} => ${String(chaveDoRemoto(url))}`);
  }
}
console.log(falhas === 0 ? `todas as ${casos.length} formas conferem` : `${falhas} divergência(s)`);
process.exit(falhas === 0 ? 0 : 1);
