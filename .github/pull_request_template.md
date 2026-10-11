## O que muda

Descreva a mudança e o problema que ela resolve. Link da issue: `#`.

## Licença da contribuição

Ao marcar esta caixa você declara os termos do [`CONTRIBUTING.md`](https://github.com/DouglasGDev/devops-desktop/blob/main/CONTRIBUTING.md):
o código é seu (ou você tem direito de enviá-lo), e ele entra sob a licença **MIT** do projeto, sem cláusula adicional.

- [ ] Aceito que esta contribuição é enviada sob a licença MIT deste projeto (inbound = outbound).

## Checagens rodadas (resultado real, não "deveria passar")

- [ ] `node ./node_modules/typescript/bin/tsc --noEmit` — saída vazia
- [ ] `npm run build` — bundle gerado
- [ ] `cargo test` — `N passed; 0 failed` → quantos: ``
- [ ] `cargo clippy --all-targets -- -D warnings` — sem aviso
- [ ] `node scripts/confira-repo.ts` — `todas as 9 formas conferem`
- [ ] Sistema operacional usado: ``

## Uso na tela (se a mudança toca interface)

Abri o aplicativo compilado e usei a funcionalidade:

- [ ] sim — o que eu vi: ``
- [ ] não — motivo: ``

## O que ficou de fora

Funcionalidade que depende de coisa que você não tem (servidor SSH, conta GitHub, outra plataforma) deve ser dita aqui,
em vez de ficar subentendida.

- ``
