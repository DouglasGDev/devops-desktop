# Como contribuir

O DevOps Desktop é aberto a contribuição de qualquer pessoa. Este arquivo diz como fazer uma contribuição que entre —
e a licença sob a qual ela entra.

## Licença de contribuição (inbound = outbound)

Ao enviar uma contribuição (issue com código, patch, PR, ou qualquer mudança enviada a este repositório) você declara
e aceita o seguinte:

1. **Você tem o direito de submeter aquele conteúdo.** O código é seu ou você recebeu permissão do autor/detentor para
   licenciá-lo assim, e nenhum contrato seu (empregador, cliente, NDA, outra licença) impede isso.
2. **A contribuição é licenciada para o projeto sob a mesma licença do projeto — MIT** ([LICENSE](LICENSE)), sem cláusula
   adicional e sem restrição de território ou de uso comercial. É a regra "inbound = outbound": o que entra sob a MIT sai
   sob a MIT.
3. **Você mantém a autoria do que escreveu.** Seu nome continua nos seus commits e no histórico do Git; o projeto não
   toma o crédito do que é seu.
4. **Nada aqui te torna responsável pelo projeto, nem dá ao projeto direitos sobre as suas outras coisas.** A
   contribuição cobre o conteúdo enviado, não o resto do seu trabalho, suas marcas ou suas contas de serviço.
5. **Conteúdo que não é código** (textos, ícones, capturas de tela) entra sob a mesma permissão, e você só envia o que
   tem direito de enviar.

Nenhuma assinatura de CLA, formulário ou cadastro é exigido: **abrir a PR já é a aceitação desses termos**, e a
checklist da PR tem a caixa correspondente. Se você preferir deixar a declaração registrada no commit, use
`git commit -s` (Signed-off-by / [DCO](https://developercertificate.org/)) — é bem-vindo, não obrigatório.

Se você **não** pode aceitar isso (por exemplo, o código pertence ao seu empregador e ele não licencia sob a MIT),
não envie a contribuição; abra uma issue descrevendo o problema e a licença que o código exigiria.

## O que o projeto aceita

Bug corrigido, funcionalidade pedida, documento errado, teste a mais. O que vale como contribuição é o que **funciona de
verdade**: este projeto existe para rodar comandos reais em máquinas reais, então contribuição que simula resultado
(dado fictício, tela que finge conexão, botão sem comando) não entra — nem como "primeiro passo".

Antes de abrir a PR, leia as regras que já derrubaram trabalho aqui:

- **Segredo nunca é commitado.** Token, senha, chave privada, `.env` ou banco local não entram no repositório, em log,
  em teste nem em print de tela. O app lê chave do OpenSSH do usuário sem mover nem copiar; contribuição não muda isso.
- **Nada destrutivo automático.** Operações que apagam, reescrevem histórico ou publicam exigem clique e confirmação.
- **Validação no backend.** Toda entrada que chega da interface é conferida no Rust (caminho, estratégia, nome de
  variável, lista de comandos). Não confie no que a tela já checou.
- **Uma definição, um validador, um motor.** O pipeline de deploy tem um dono (`src-tauri/src/deploy.rs`), e o contrato
  em `src-tauri/src/pipeline.rs` chama as mesmas checagens. Não crie um segundo caminho de execução nem uma segunda
  tabela de definição.
- **Estilo da interface:** controle + estado, sem parágrafo explicando o que o botão faz — o "como usar" mora no
  README. Não reformule o visual junto de uma feature: visual é passada separada.

## Por onde começar

1. Leia [README.md](README.md) (utilização, modelo de segurança e arquitetura) e
   [docs/ENTREGAS.md](docs/ENTREGAS.md) — o segundo arquivo diz **o que está verificado** e o que ainda falta, item por
   item, com o primeiro passo concreto de cada pendência. A lista em §7 do mesmo arquivo é onde as features novas são
   escolhidas.
2. Procure uma issue aberta, ou abra uma descrevendo o problema antes de escrever código. Mudança grande sem issue
   conversada tem chance alta de não entrar.
3. Branch a partir de `main`. Um assunto por branch.

## Preparar o ambiente

Requisitos: [Rust](https://www.rust-lang.org/tools/install) (stable), [Node.js](https://nodejs.org) ≥ 20,
[git](https://git-scm.com) no `PATH`, e as dependências do Tauri 2 descritas no
[guia oficial](https://v2.tauri.app/start/prerequisites/). No Windows, o instalador do Tauri pede também as Build Tools
do Visual Studio e o WebView2 (já vem no Windows 10/11).

```bash
npm install
```

Para mexer na interface, abra o executável compilado (não use `tauri dev` se a porta 1420 já estiver ocupada na sua
máquina):

```bash
npm run tauri build
```

## Checagens que toda PR precisa passar

Rode na ordem; a PR entra quando todas estiverem verdes na sua máquina **e** na descrição constar o resultado real.

```bash
node ./node_modules/typescript/bin/tsc --noEmit     # tipos
npm run build                                       # bundle do frontend
cd src-tauri && cargo test                           # suíte do backend
cd src-tauri && cargo clippy --all-targets -- -D warnings
node scripts/confira-repo.ts                         # conferência de URL de repositório
```

A suíte Rust usa os binários **reais** de `git` e `ssh` da máquina — em uma máquina sem essas ferramentas os testes que
as exercitam falham, e isso não é pulado silenciosamente. Não use `cargo fmt` neste projeto: o formato é o do autor, e a
reorganização inteira do arquivo polui o diff.

Se a mudança mexe em tela, **abra o aplicativo e use a funcionalidade** antes de dizer que está pronta. Diga na PR o
que você viu (inclusive quando não dá para verificar: "não tenho um servidor SSH, o caminho remoto não foi testado" é
uma resposta válida — "por construção não muda" não é).

## Banco local e migrações

O SQLite usa `PRAGMA user_version`, e as chaves estrangeiras **estão ligadas** (o `libsqlite3-sys` embutido é compilado
com `SQLITE_DEFAULT_FOREIGN_KEYS=1`). Consequência que já derrubou dados: `DROP TABLE` numa migração apaga os filhos em
cascata. Para reconstruir tabela-mãe, desligue a checagem no começo do batch e ligue no fim (veja a v5 em
`src-tauri/src/db.rs`), e faça um teste provando que modelo, execução e etapas sobrevivem à migração. Nunca apague nem
recrie a base de ninguém para "consertar" um esquema.

## Reportar problema de segurança

Não abra issue pública. Veja [SECURITY.md](SECURITY.md).

## Licença do que já está aqui

O projeto é MIT. Dependências mantêm a licença delas; adicionar uma dependência com licença copyleft (GPL/AGPL) precisa
de conversa antes — ela muda o que qualquer pessoa pode fazer com o aplicativo.
