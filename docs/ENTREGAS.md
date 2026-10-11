# Registro de entregas — onde está cada coisa, o que falta e como retomar

Documento de retomada. Ele não substitui o [README](../README.md) (uso do aplicativo) nem o
[SPEC.md](SPEC.md) (o que foi pedido); ele registra **o que já está pronto, onde está o código, como se prova
cada parte e o que ainda não existe**. Escrito em 2026-10-10 sobre o trabalho do modo sem servidor (`66da3a7`) mais a
**etapa 2 do plano CI/CD** — o contrato central de pipeline (`4f005ee`) e a licença de contribuição (`df51de1`), que são
o estado destas linhas.

## 1. Retomar em três passos

```bash
node ./node_modules/typescript/bin/tsc --noEmit     # tipos do frontend
npm run build                                        # tsc + bundle
(cd src-tauri && cargo test)                         # 208 testes de backend
(cd src-tauri && cargo clippy --all-targets -- -D warnings)
node scripts/confira-repo.ts                         # o comparador remoto↔pasta, 9 formas reais de URL

# opcional, para mexer na interface: abrir o exe com porta de debug e pilotar a janela (§4.1)
# WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 → src-tauri/target/release/devops-desktop.exe
node scripts/ponte-cdp.mjs 'return typeof window.__TAURI_INTERNALS__.invoke;'
```

Se os cinco itens acima estiverem verdes, a base está íntegra e dá para começar a feature nova sem medo.
`node scripts/confira-repo.ts` precisa de Node com remoção de tipo nativa (a verificação aqui foi feita com Node
v24.18.0). O instalador: `npm run tauri build` (roda `npm run build` sozinho, LTO + `codegen-units = 1` — leva ~10 min).

Armadilhas já conhecidas nesta máquina:

- **Não rode `npm run tauri dev`**: a porta 1420 está ocupada por um `node.exe` de Vite de sessão anterior. Para ver a
  interface, abra o executável compilado (`src-tauri/target/release/devops-desktop.exe`).
- **Editar fonte depois de iniciar `npm run tauri build` deixa o `dist/` dentro do instalador defasado.** O bundle lê
  `dist` no fim; se mexeu em `src/` durante a compilação, refaça `npm run build` e compile de novo.
- **`TaskStop` não mata os filhos** (`cargo`, `rustc`, `node`): confira `Get-Process` antes de assumir que a compilação
  anterior acabou.
- `cargo fmt` **não** é usado neste projeto — o formato é o do próprio autor, e `cargo fmt` reorganiza tudo.

## 2. Estado verificado em 2026-10-10 (resultado lido, não presumido)

| Checagem | Resultado real |
| --- | --- |
| `tsc --noEmit` | saída vazia, código 0 |
| `npm run build` | `✓ built in 6.62s`; único aviso é o de chunk > 500 kB (`dist/assets/index-Cp6MoOD2.js`, 847.124 B) |
| `cargo test` (todos os alvos) | `208 passed; 0 failed; 0 ignored` em `devops_desktop_lib` (19,39 s); `main.rs` e doc-tests com 0 testes |
| `cargo clippy --all-targets -- -D warnings` | `Finished \`dev\` profile` sem aviso (com `-D warnings`, qualquer aviso derrubaria o comando) |
| `node scripts/confira-repo.ts` | `todas as 9 formas conferem`, código 0 |
| Migração `user_version` 6 no banco real | cabeçalho lido em `AppData/Roaming/com.douglasgdev.devopsdesktop/devops-desktop.db`: `user_version 6`, `page_size 4096`. As colunas novas e a tabela `pipeline_revisions` existem de verdade porque as leituras abaixo as devolveram preenchidas, vindas do mesmo arquivo |
| **Fluxo sem servidor na janela real** (CDP, §4.1) | modelo `serverId: null` salvo · pré-vistoria `2 arquivo(s), 72.1 KB` · publicar #1 `success` com as 15 etapas · guarda `.devops-backups/1` com o README antigo **e** o excedente movido · reverter #2 `2 arquivo(s) reposto(s)` (conferido no disco) · publicar #3 `1 copiado · 1 já igual` (CRC32) · destino dentro do projeto **recusado na etapa 1** sem tocar no disco · caminho duplicado recusado (`o projeto já tem um modelo de deploy chamado "…"`) · remover o modelo → `deploy_runs` 5 → 0 e `audit_log` 57 → 58 (o histórico fica na auditoria, como a confirmação diz) · UI do modo local conferida no exe: destino "Nesta máquina (sem SSH)", estratégias só **C**/**D**, rótulo "Guardar o que já está no destino antes de sobrescrever", sem o checkbox de releases, e os quatro diálogos (pré-vistoria → publicar → reverter → remover) lidos na tela |
| As quatro portas do "o deploy gera o próprio artefato" (exe das 11:01) | **W1** artefatos sem seleção + build gerando `saindo-do-build.txt`: 61 copiados, arquivo conferido no destino · **W2** artefatos declarando `gerado-pela-preparacao.txt` que **ainda não existe**: etapa 4 anuncia, preparação gera, 1 copiado, `success` · **W3** caminho que nenhum comando gera: `failed` na etapa 8 com "depois da preparação não sobrou nenhum arquivo para enviar — confira os caminhos" · **W4** `script`: `success` sem copiar nada (destino ficou vazio, como projetado) |
| **Contrato de pipeline na janela real** (CDP, etapa 2 do plano CI/CD) | definição criada como **v1** · execução #23 `success` com `definition_version 1`, `environment "local"`, `destination` o caminho absoluto do destino e `commit_sha f40243ff12c2a6…` **igual ao `git rev-parse HEAD`** lido na pasta · editar a definição → **v2**, salvar de novo sem mudar → **v3**, e `pipeline_revisions` devolve os três retratos (`v3`, `v2`, `v1`) em que só o `v2` carrega `exclusao: ["src/index.html"]` — ou seja, o retrato guarda o que mudou, e `variaveis` sai como `[]` porque o retrato não tem valor nenhum · `pipeline_validate` → `definição “Piloto sem servidor” v3 · artifacts · destino C:\… · nesta máquina` · **Repetir** o #23 criou a #24 com `definition_version 3` enquanto a #23 **continua `v1`** (a procedência antiga não é reescrita por edição posterior) · #25 com um arquivo do destino travado por outro processo: `failed` na etapa 9 com `falha ao guardar README.md antes de sobrescrever: O arquivo já está sendo usado por outro processo. (os error 32)`, etapas 10–15 em `waiting`, `pipeline_retry_policy` → `allowed: false` com o motivo, e `pipeline_retry` **recusa a mesma frase sem abrir execução nenhuma** (as execuções param em #27 — não existe #28 de retry) · reversão da #25 → execução própria com `1`, `2`, `11`, `14`, `15` como `done` e `3`–`10`+`12`–`13` como `skipped`, `1 arquivo(s) reposto(s)`, e o `README.md` do destino **conferido no disco** de volta ao conteúdo anterior · execução de 1,5 GB com o aplicativo **fechado à força no meio**: ao reabrir, a #30 está `cancelled` com `interrompida — o aplicativo foi fechado antes de terminar`, a etapa 9 está `failed` com `interrompida — o aplicativo foi fechado`, e a auditoria ganhou a linha `Execuções interrompidas identificadas · 1 execução(ões) e 1 etapa(s) marcadas como interr…` — a varredura rodou no `setup`, não num teste · tela: `v5` no cabeçalho do modelo e no seletor da aba **Deploy**, **Conferir** imprimindo `definição “Piloto sem servidor” v5 · artifacts · destino C:\… · nesta máquina`, **Versões salvas** listando `v5 v4 v3 v2 v1`, histórico com `· v4 · f40243ff` por execução, painel âmbar do **Repetir** mostrando o motivo da recusa lido na janela |
| Frontend embutido no executável | conferido **dentro da janela**: o pacote das 09:37 servia `index-5zynmBe_.js` enquanto `dist/` já tinha `index-Cko7uGOK.js` (frontend velho dentro de um binário novo). O binário desta sessão (18.255.872 B, 19:49) serve `/assets/index-Cp6MoOD2.js`, o mesmo hash de `dist/index.html` |
| Instalador | `npm run tauri build` → `DevOps Desktop_0.1.0_x64-setup.exe` (4.873.491 B) e `.msi` (7.016.448 B) |
| Executando | `devops-desktop.exe` aberto por `Start-Process`, `MainWindowTitle = "DevOps Desktop — Central de SSH, Git e Deploy"`, `Responding = True`, 10 telas navegáveis por clique |
| Git | `main` em sincronia com `origin/main`; público em <https://github.com/DouglasGDev/devops-desktop> |

O que isso **não** cobre: as asserções automáticas continuam sendo de backend — o piloto de interface (§4.1) dirige a
janela de verdade, mas quem decide o que está certo a cada leitura sou eu, não uma suíte de testes de UI. O caminho por
SSH (`server_id` preenchido) **não** foi reexecutado hoje porque não há servidor cadastrado nesta máquina; por isso as
três correções desta sessão foram escritas só dentro de ramos protegidos por `local_mode()` (ou strategies que já eram
locais), para não mexer no que não pude medir. O registro de auditoria guarda as execuções de teste (não há como apagar
linhas de auditoria por desenho).

### 4.1 Como a janela foi dirigida (reproduzível)

```bash
taskkill //IM devops-desktop.exe //F
export WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223"
powershell -NoProfile -Command "Start-Process -FilePath 'E:\ssh-master\src-tauri\target\release\devops-desktop.exe'"
sleep 9 && curl -s http://127.0.0.1:9223/json/list   # a página tauri.localhost e o webSocketDebuggerUrl
node scripts/ponte-cdp.mjs 'const inv=window.__TAURI_INTERNALS__.invoke; return JSON.stringify(await inv("list_projects"));'
```

`scripts/ponte-cdp.mjs` roda uma expressão dentro da janela real: dá para clicar (`b.click()`) e chamar qualquer
comando do backend com o mesmo payload que a interface manda. A porta de debug **só** existe com a variável de
ambiente acima — o instalador normal não a abre.

## 3. Etapas do SPEC §16 — entregues, com o código e a prova

| Etapa | O que funciona | Onde | Como ver na tela |
| --- | --- | --- | --- |
| 1. Estrutura desktop, UI, persistência | shell com 10 telas, tema claro/escuro, SQLite local com migrações, auditoria, busca global | `src/App.tsx`, `src/store/AppContext.tsx`, `src-tauri/src/{db,models,audit}.rs` | **Topbar** (busca) e **Histórico** (o que foi gravado) |
| 2. Descoberta dos acessos SSH | lê `~/.ssh/config` e `known_hosts` sem mover nem copiar chave | `src-tauri/src/ssh_config.rs` | **Servidores → Descobrir** |
| 3. Teste de conexão e gerenciamento | diagnóstico por camada (DNS, porta, autenticação, host key), cadastro/edição/exclusão, ambiente por servidor | `src-tauri/src/ssh_test.rs`, `auth.rs` | **Servidores → Testar** |
| 4. Terminal SSH | PTY múltiplo, redimensionamento, histórico de comandos local, rotinas salvas | `src-tauri/src/terminal.rs`, comandos `*_terminal_session`, `*_routine` | **Terminal** |
| 5. Gerenciador de arquivos remoto | listar, criar pasta, renomear, chmod, ler/editar texto, subir/descarregar arquivo e pasta com progresso e cancelamento | `src-tauri/src/files.rs`, `files_transfer.rs` | **Arquivos** |
| 6. Git completo | status, stage por arquivo e por hunk, diff linha a linha, commit/amend, gráfico, árvore rastreada + conteúdo do HEAD, ramos, tags, remotos, merge/rebase com conflito por lado, revert/reset/cherry-pick, guardar e recuperar trabalho em andamento (`stash`), fetch/pull/push, clone, abrir no editor | `src-tauri/src/git.rs`, `src/views/GitView.tsx`, `src/components/git/*` | **Git** (abas Alterações, Árvore, Histórico, Guardados, Ramos, Tags, Remotos, Segredos) |
| 7. Projetos e perfis de deploy | cadastro de pasta com sondagem real de git, perfil por projeto/estratégia | `src-tauri/src/git_probe.rs`, `deploy.rs`, `commands.rs` (`*_deploy_profile`) | **Projetos**, **Deploy** |
| 8. Motor de deploy com pipeline visual | estratégias A–E, preflight de ferramentas no servidor, etapas com comando/duração/código de saída, cancelamento cooperativo, rollback; **e o mesmo pipeline sem servidor** (cópia local por CRC32, guarda em `.devops-backups`, reversão que repõe até os arquivos da limpeza) | `src-tauri/src/deploy.rs`, `sshexec.rs` | **Deploy** |
| 9. Segredos e proteção de commit | varredura do conteúdo **que o Git publicaria** (`git show :path` / `HEAD:path`), bloqueio antes de commit e push, `git rm --cached` assistido, regras extras | `src-tauri/src/security.rs` | **Git → Segredos**; o bloqueio aparece ao commitar/pushar |
| 10. Dashboard, histórico, refinamentos | métricas reais lidas do servidor quando você pedir, deploys recentes, falhas, transferências, alertas de configuração | `src-tauri/src/infra.rs`, `src/components/InfraPanel.tsx` | **Painel** |

## 4. Frentes que foram além do SPEC (o "canivete")

- **GitHub real** (`src-tauri/src/github.rs`, `src/views/GitHubView.tsx`, `src/components/github/*`): fluxo de dispositivo
  OAuth com token **só na memória**, bandeja, PR com diff real pelo mesmo parser do diff local, revisão, mesclar com
  método escolhido, conversa de comentários, issues, execuções do Actions com reexecução, abrir PR de um ramo local,
  buscar o ramo de um PR (`refs/pull/<n>/head`), criar repositório.
- **Deploy sem servidor** (perfil com `server_id = NULL`): o destino passa a ser uma pasta desta máquina, com as
  estratégias **artefatos** e **script**. As quinze etapas são as mesmas; a conexão vira *Abrir a pasta de destino* e o
  envio vira *Copiar os arquivos para o destino* (`STEPS_LOCAIS`). A comparação é por CRC32 (`crc_of_file`), o comando
  roda por `cmd /C` / `sh -c` com o mesmo prazo e cancelamento do remoto (`sshexec::run_local_with_env`), os valores das
  variáveis entram pelo **ambiente do processo** e nunca pela linha de comando, a guarda fica em
  `<destino>/.devops-backups/<run_id>/`, e *apagar excedentes* **move** para a guarda em vez de remover. `validate_local`
  recusa raiz de disco, caminho relativo, `..` e destino que colide com a pasta do projeto nos dois sentidos. Depois dos
  comandos de build **e** dos de preparação, a seleção é refeita e volta pelo portão de segredos (`prepare_local` +
  `build_step`), porque a etapa 4 varre o disco antes de qualquer comando rodar — sem isso, o artefato gerado pelo próprio
  deploy nunca seria copiado. Se o caminho declarado ainda não existe porque é exatamente o que o comando vai gerar, a
  etapa 4 não morre mais: ela anuncia "nenhum arquivo existe ainda nos caminhos configurados" e a cobrança vem depois
  dos comandos. Em `script` a reescolha é deliberadamente pulada (ver §5, item 9).
- **Publicar sem servidor e sem conta** (`git_publish`): roda os comandos locais do projeto, `git push` do ramo e tag, com
  checagem de segredo antes; **Despublicar** uma tag; **Testar acesso** com `git ls-remote` e prazo de 25 s.
- **Aba Local no GitHub**: a seção Git inteira (`GitView`) embutida sobre a pasta mapeada do repositório — mesmos painéis,
  mesmos comandos, sem duplicar código nem botões enfeitados.
- **Mapear pasta local** (`src/lib/useMapearPasta.ts`, `src/lib/repo.ts`): aponta o clone que já existe no disco, compara
  `dono/repo` do que o git respondeu (host-agnóstico) e mostra o caminho exato + **Mostrar a pasta** no Explorador.
- **Máquina** (`src-tauri/src/tools.rs`, `src/components/MachineAccess.tsx`): detecta `ssh`/`git`/`gh`/`ssh-add`,
  identidade do git, chaves no agente, editores no disco, e diz se esta máquina já está liberada para publicar.
- **Operação parada lida das marcas do `.git`** (`git::operation_in_progress`): `rebase-merge`/`rebase-apply`,
  `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` viram `status.operation`, e a barra de conflitos só oferece
  *Cancelar* / *Continuar* / *Pular este passo* coerentes com o que o Git realmente está fazendo. Antes a tela adivinhava
  "rebase" por cabeça detached — o que levava a um *Cancelar o merge* que falharia num cherry-pick ou numa reversão.
- **Senha por prompt, nunca salva** (`src-tauri/src/auth.rs`, `PasswordPrompt.tsx`): `SSH_ASKPASS` por operação.
- **Contrato central de pipeline** (`src-tauri/src/pipeline.rs`, etapa 2 do plano CI/CD): não é um segundo motor — cada
  operação termina em `deploy::start`, `deploy::cancel` e `deploy::run`. O que ele acrescenta é o que faltava para o
  pipeline ser uma coisa só com história: a **definição versionada** (`deploy_profiles.version` sobe a cada salvamento e
  deixa o retrato do que era em `pipeline_revisions`, com os **nomes** das variáveis e nunca os valores), a
  **procedência de cada execução** (`definition_version`, `destination`, `environment` e `commit_sha` lido do git da
  pasta no instante em que a execução abre), o **validador único** `deploy::checar_definicao` — as mesmas checagens da
  etapa 1 do motor, chamadas também por `pipeline_validate`, para a tela nunca dizer "ok" a algo que o motor recusaria —,
  a **política de nova tentativa** (`pipeline_retry_policy`/`pipeline_retry`: só repete quando nenhuma etapa a partir da 9
  trabalhou, porque a partir daí o resultado no destino é desconhecido; `success` pode ser republicado; execução sem
  etapa registrada é bloqueada, e o "não" nunca vem mudo — vem com o que conferir), e a **varredura de execuções órfãs**
  na abertura (`recover_open_runs`: `running` que sobreviveu ao fechamento do aplicativo vira `cancelled` com o motivo,
  e nada é reexecutado sozinho). Na tela: **Projetos → Modelos de deploy** mostra `v{n}`, o botão de conferência e as
  versões salvas; **Deploy** mostra a procedência na execução e o botão **Repetir**, que pergunta a política antes de
  abrir as variáveis.

Total da superfície: **129 comandos Tauri registrados**, 22 arquivos Rust em `src-tauri/src`, 10 views,
33.677 linhas de TS/TSX/Rust.

Persistência (SQLite local, migrações em `src-tauri/src/db.rs`) — as dez tabelas que existem hoje:
`settings`, `servers`, `projects`, `audit_log`, `routines`, `terminal_history`, `deploy_profiles`, `deploy_runs`,
`deploy_run_steps`, `pipeline_revisions`. Não há tabela de série histórica de métrica: o painel lê do servidor na hora
pedida.

## 5. Decisões que não devem ser revertidas

Estas vieram do SPEC e de correções explícitas do Douglas; reverter qualquer uma desfaz a entrega:

1. **Nada simulado.** Nenhum botão existe sem comando real; lista vazia aparece como vazia; erro do git/GitHub aparece
   com a frase dele. Proibido dado fictício, conexão falsa, resposta de exemplo.
2. **Segredo nunca sai.** Token só em memória; senhas nunca em SQLite/log; auditoria não grava corpo de mensagem nem
   URL com credencial; a checagem antes de commit/push lê o índice e o HEAD, não o disco.
3. **Nada destrutivo automático.** `amend`, `reset`, merge, publicar, apagar tag do remoto, fechar issue, mesclar PR,
   criar repositório: sempre clique + confirmação. A aplicação **não** reescreve histórico remoto e não faz `push` sozinha.
4. **Host key nunca é aceita pela aplicação**; desconhecido/alterado vira pergunta, e a sessão interativa no terminal é o
   caminho para registrar no `known_hosts`.
5. **Sem prose explicativa na interface.** Controle + estado; o "como fazer" mora no README, não em parágrafo embaixo do
   campo. (Regra que já motivou remoção de texto em tela.)
6. **Uma classe por mecânica**: cada responsabilidade tem seu arquivo em `src/components/git` / `github`, com trabalho
   fundo e completo, sem stub e sem botão enfeitado. Painéis recebem o hook (`git: GitRepo`) para serem reutilizados —
   foi assim que a aba Local nasceu.
7. **O SQLite embutido aplica chaves estrangeiras.** `libsqlite3-sys` (bundled) é compilado com
   `SQLITE_DEFAULT_FOREIGN_KEYS=1`, então `PRAGMA foreign_keys` já nasce **ligado** em toda conexão (medido: `fk=1`), e
   `init()` reafirma isso por explícito. Consequência que já derrubou dados: um `DROP TABLE` numa migração apaga em
   cascata os filhos — reconstruir tabela-mãe exige `PRAGMA foreign_keys=OFF` no começo do batch e `ON` no fim, e é o
   procedimento documentado do SQLite. Quem assumir que "cascatas não rodam" sem medir a flag vai apagar o histórico do
   usuário.
8. **`deploy_run_steps` tem índice ÚNICO em `(run_id, position)`.** `upsert_step` grava com
   `ON CONFLICT(run_id, position)`, e sem índice único nessas colunas o SQLite recusa a instrução inteira no prepare.
   O erro era engolido por `let _ =` em `mark()`, o resultado era um histórico de deploy sem nenhuma etapa gravada.
   `mark()` hoje mostra a falha de gravação no log da execução.
9. **A estratégia `script` não copia arquivo nenhum, de propósito** (etapa 4 pula a seleção e a etapa 10 informa
   "só comandos"). Qualquer código que reescolha os arquivos depois de um comando local precisa respeitar isso: encher
   `ctx.files` num modelo `script` faz a confirmação (etapa 14) cobrar no destino um arquivo que a estratégia nunca
   enviou — medido hoje, um deploy Modo D que estava `success` passou a `failed` por causa exatamente disso antes de a
   reescolha ser restrita às estratégias que copiam.
10. **O que o próprio deploy gera precisa ser reescaneado antes da cópia — e a etapa 4 não pode atrapalhar.** A seleção
   nasce na etapa 4, e build (7) e preparação (8) rodam depois. Três portas estavam fechadas, todas medidas na janela
   real: (a) `prepare_local` nunca refazia a lista; (b) sem servidor, caminhos de inclusão vazios significam "projeto
   inteiro", e essa lista também não era refeita depois do build; (c) quando o artefato declarado ainda **não existe**
   (porque é justamente o que o comando vai gerar), a etapa 4 morria em "nenhum arquivo corresponde aos caminhos" antes
   de qualquer comando rodar. Hoje a etapa 4 segura o erro quando o modelo tem comando de build/preparação nesta
   máquina, e a ausência vira falha na etapa 7/8 — depois da chance de produzir. Os reescaneys chamam `gate()` de novo,
   então segredo gerado por build ou preparação continua barrado.
   *Rasura honesta desta sessão:* eu tinha registrado "(a) resolvido, 1 arquivo copiado" com base num teste em que o
   arquivo **já existia na raiz do projeto**, sobra de uma execução anterior; o teste limpo falhou na etapa 4. É por
   isso que os destinos de teste são apagados antes de cada battery e o `git status` é conferido depois.
11. **O binário embute o `dist/` do momento em que o Rust leu a pasta**, não do momento em que ele terminou. Um
    `tauri build` com edição de `src/` no meio gera um instalador com frontend velho e tamanho idêntico ao novo, o que
    não dá para perceber pelo arquivo em si. A conferência é dentro da janela: o hash de `/assets/index-*.js` carregado
    tem de ser o mesmo de `dist/assets/` (hoje `index-Cp6MoOD2.js`; o pacote das 09:37 servia `index-5zynmBe_.js`).
    **E não adianta procurar texto de interface dentro do `.exe`:** os assets vão comprimidos (brotli) no binário, então
    um `grep` de string de tela não acha nada **mesmo quando o frontend está atualizado** — foi exatamente assim que eu
    declarei, errado, que o binário trazia o frontend velho. A única prova é o hash lido na janela.
12. **Um contrato, um validador, um motor** (etapa 2 do plano CI/CD). `deploy_profiles` é a definição; `pipeline.rs` não
    redefine nada — ele lê, conferem e delega. `deploy::checar_definicao` é o único validador e é chamado pela etapa 1 do
    motor **e** por `pipeline_validate`, de propósito: se a tela dissesse "ok" a algo que o motor recusa, a confiança nas
    outras checagens cai junto. Rodar continua sendo `deploy::start` (inclusive `pipeline_retry`), e o retrato de versão
    grava os **nomes** das variáveis, nunca os valores. A política de repetição só libera antes da etapa 9 porque a
    partir dela o resultado no destino é desconhecido; o "não" sempre vem com o que conferir. A varredura de abertura
    **nunca reexecuta** nada: só marca o que ficou pendurado.

## 6. Pendências assumidas (com o primeiro passo de cada)

| # | Pendência | Primeiro passo concreto |
| --- | --- | --- |
| 0 | **Dados do piloto da etapa 2, ainda cadastrados de propósito** | O projeto **Piloto Etapa 2** (`%TEMP%\piloto-etapa2`) com a definição *Piloto sem servidor* v5 e as execuções #23–#30 continuam no cadastro para ele abrir a janela e ver as telas conferidas. Quando ele decidir, o primeiro passo é **Projetos → Piloto Etapa 2 → Remover do cadastro** (a confirmação avisa quantos modelos saem junto; nenhum arquivo local é apagado) e depois apagar a pasta `%TEMP%\piloto-etapa2`. O arquivo de teste de 1,5 GB já foi removido do projeto e do destino |
| 1 | **Client ID na máquina do Douglas** | Colar o **Client ID** da OAuth App (o campo está em **GitHub → Configuração**) e gerar um secret novo no GitHub — o valor hexadecimal que estava salvo no banco era o **secret**, não o ID. Sem isso a aba GitHub não conecta: é limite de conta, não do código |
| 2 | **Passada de visual (prioridade quando ele chamar)** | Só depois das funções; começar pelo print da tela que incomoda. Não refazer estilo por conta própria junto de feature |
| 3 | **Pacotes Linux** | Os alvos `deb`/`rpm`/`AppImage` nunca foram gerados aqui. Rodar `npm run tauri build` numa imagem Linux e ajustar `bundle.targets` + paths (o código já usa `dirs`/caminhos relativos, mas isso precisa ser provado, não suposto) |
| 4 | **Teste de interface automatizado** | Não há runner com asserções. O que existe hoje é o piloto `scripts/ponte-cdp.mjs` (§4.1), que dirige a janela real mas exige leitura humana a cada passo. Se um runner entrar, o mínimo é o fluxo do SPEC §17; a cobertura automática continua `tsc` + suíte Rust + `scripts/confira-repo.ts` |
| 5 | **Aceite visual das últimas entregas** | `Git → Árvore`, `GitHub → repositório → Local`, `Git → Guardados`, os botões *Reproduzir este commit* / *Reverter este commit* / *Voltar para este commit*, o **fluxo de deploy sem servidor** (modelo com destino "Nesta máquina", pré-vistoria, publicar, reverter) e o **contrato de pipeline** — em **Projetos → Modelos de deploy** o badge `v{n}`, o botão **Conferir** e a lista de **Versões salvas**; em **Deploy** a linha de procedência da execução (`definição v{n} · ambiente · destino · commit`), o `· v{n} · {sha8}` no histórico e o **Repetir** com o painel âmbar quando a política recusa. Eu abri cada uma dessas telas e li o texto na janela real, mas **ele** ainda não viu nada disso nesta máquina |
| 6 | **Revalidar o deploy por SSH num host real** | Além de `build_step`/`identify_step`, a etapa 2 mexeu no caminho com servidor: `validate_remoto` agora chama `deploy::checar_definicao(..., Some(&servidor))` e `insert_run` passou a receber a procedência. As checagens remotas foram movidas verbatim e `server_ref()` continua sendo a primeira delas, então por construção o remoto não muda — mas "por construção" sem medir num host é presunção. Primeiro passo: cadastrar um servidor e repetir C/D com `server_id` preenchido, conferindo que a etapa 1 recusa o que recusava antes |

## 7. Lacunas honestas do "canivete suíço" (ainda NÃO existe)

Listadas para escolher a próxima frente — nada aqui está implementado, e nada aqui foi inventado como "quase pronto":

**Git** — `bisect`, `blame` por linha, busca no histórico por autoria/texto, `submodule`, `worktree` múltiplo por projeto,
assinatura de commit (`gpg.sign`), `git fsck`/manutenção. (`stash` e `cherry-pick` saíram desta lista em 2026-10-10:
estão na aba **Guardados** e no card de detalhe do commit, com os comandos `git_stash_*` e `git_cherry_pick`.)

**SSH / servidor** — túnel e redirecionamento de porta (`LocalForward`), gerência de `authorized_keys`, gestão de serviço
(`systemctl`), visualizador de logs remotos (`journalctl`), cron/`at`, Docker "ao vivo" (listar/parar containers) fora do
deploy, edição remota de `.env` com diff, comparação de arquivos entre dois servidores.

**GitHub** — releases e assets, labels/milestones/revisores pedidos, notificações reais de não lidas, status checks por
commit, download de artefatos do Actions, secrets do Actions do repositório, branch protection, busca de código entre
repositórios, sync de forquilha.

**Produto** — perfis/multi-conta, credenciais por vault, agendamento de rotinas, importação/exportação da base,
comparação de ambiente entre dois servidores (diff de versão de ferramenta), testes de carga, dashboards com histórico de
métrica (hoje a métrica é lida na hora e não é armazenada em série).

Cada uma dessas é implementável dentro da arquitetura atual (comando Tauri → módulo Rust → hook/`api.ts` → painel), sem
servidor intermediário e sem conta de serviço — que é o critério do canivete.
