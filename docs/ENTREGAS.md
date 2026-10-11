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
(cd src-tauri && cargo test)                         # 229 testes de backend
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

## 2. Estado verificado em 2026-10-10/11 (resultado lido, não presumido)

| Checagem | Resultado real |
| --- | --- |
| `tsc --noEmit` | saída vazia, código 0 |
| `npm run build` | `✓ 83 modules transformed`; `dist/assets/index-DT3bM82P.js` 861.54 kB (gzip 234.38 kB) e `index-CVC3fftf.css` 60.42 kB; único aviso é o de chunk > 500 kB. Na rodada das 01:04 o tempo lido foi `✓ built in 6.03s`; a rodada final (01:22) deu **o mesmo hash**, porque a fonte do frontend não mudou entre as duas |
| `cargo test` (todos os alvos) | `229 passed; 0 failed; 0 ignored` em `devops_desktop_lib` (16,91 s, perfil `release`, lido na rodada final depois do rebuild das 01:27); `main.rs` e doc-tests com 0 testes. Na rodada anterior desta sessão: `229 passed` (17,08 s) e, antes da etapa 3, `208 passed` (19,39 s) |
| `cargo clippy --release --all-targets -- -D warnings` | `Finished \`release\` profile` sem aviso (com `-D warnings`, qualquer aviso derrubaria o comando). Na rodada final, depois do rebuild: código 0, 0,70 s |
| `node scripts/confira-repo.ts` | `todas as 9 formas conferem`, código 0 |
| Migração `user_version` 6 no banco real | cabeçalho lido em `AppData/Roaming/com.douglasgdev.devopsdesktop/devops-desktop.db`: `user_version 6`, `page_size 4096`. As colunas novas e a tabela `pipeline_revisions` existem de verdade porque as leituras abaixo as devolveram preenchidas, vindas do mesmo arquivo |
| **Fluxo sem servidor na janela real** (CDP, §4.1) | modelo `serverId: null` salvo · pré-vistoria `2 arquivo(s), 72.1 KB` · publicar #1 `success` com as 15 etapas · guarda `.devops-backups/1` com o README antigo **e** o excedente movido · reverter #2 `2 arquivo(s) reposto(s)` (conferido no disco) · publicar #3 `1 copiado · 1 já igual` (CRC32) · destino dentro do projeto **recusado na etapa 1** sem tocar no disco · caminho duplicado recusado (`o projeto já tem um modelo de deploy chamado "…"`) · remover o modelo → `deploy_runs` 5 → 0 e `audit_log` 57 → 58 (o histórico fica na auditoria, como a confirmação diz) · UI do modo local conferida no exe: destino "Nesta máquina (sem SSH)", estratégias só **C**/**D**, rótulo "Guardar o que já está no destino antes de sobrescrever", sem o checkbox de releases, e os quatro diálogos (pré-vistoria → publicar → reverter → remover) lidos na tela |
| As quatro portas do "o deploy gera o próprio artefato" (exe das 11:01) | **W1** artefatos sem seleção + build gerando `saindo-do-build.txt`: 61 copiados, arquivo conferido no destino · **W2** artefatos declarando `gerado-pela-preparacao.txt` que **ainda não existe**: etapa 4 anuncia, preparação gera, 1 copiado, `success` · **W3** caminho que nenhum comando gera: `failed` na etapa 8 com "depois da preparação não sobrou nenhum arquivo para enviar — confira os caminhos" · **W4** `script`: `success` sem copiar nada (destino ficou vazio, como projetado) |
| **Contrato de pipeline na janela real** (CDP, etapa 2 do plano CI/CD) | definição criada como **v1** · execução #23 `success` com `definition_version 1`, `environment "local"`, `destination` o caminho absoluto do destino e `commit_sha f40243ff12c2a6…` **igual ao `git rev-parse HEAD`** lido na pasta · editar a definição → **v2**, salvar de novo sem mudar → **v3**, e `pipeline_revisions` devolve os três retratos (`v3`, `v2`, `v1`) em que só o `v2` carrega `exclusao: ["src/index.html"]` — ou seja, o retrato guarda o que mudou, e `variaveis` sai como `[]` porque o retrato não tem valor nenhum · `pipeline_validate` → `definição “Piloto sem servidor” v3 · artifacts · destino C:\… · nesta máquina` · **Repetir** o #23 criou a #24 com `definition_version 3` enquanto a #23 **continua `v1`** (a procedência antiga não é reescrita por edição posterior) · #25 com um arquivo do destino travado por outro processo: `failed` na etapa 9 com `falha ao guardar README.md antes de sobrescrever: O arquivo já está sendo usado por outro processo. (os error 32)`, etapas 10–15 em `waiting`, `pipeline_retry_policy` → `allowed: false` com o motivo, e `pipeline_retry` **recusa a mesma frase sem abrir execução nenhuma** (as execuções param em #27 — não existe #28 de retry) · reversão da #25 → execução própria com `1`, `2`, `11`, `14`, `15` como `done` e `3`–`10`+`12`–`13` como `skipped`, `1 arquivo(s) reposto(s)`, e o `README.md` do destino **conferido no disco** de volta ao conteúdo anterior · execução de 1,5 GB com o aplicativo **fechado à força no meio**: ao reabrir, a #30 está `cancelled` com `interrompida — o aplicativo foi fechado antes de terminar`, a etapa 9 está `failed` com `interrompida — o aplicativo foi fechado`, e a auditoria ganhou a linha `Execuções interrompidas identificadas · 1 execução(ões) e 1 etapa(s) marcadas como interr…` — a varredura rodou no `setup`, não num teste · tela: `v5` no cabeçalho do modelo e no seletor da aba **Deploy**, **Conferir** imprimindo `definição “Piloto sem servidor” v5 · artifacts · destino C:\… · nesta máquina`, **Versões salvas** listando `v5 v4 v3 v2 v1`, histórico com `· v4 · f40243ff` por execução, painel âmbar do **Repetir** mostrando o motivo da recusa lido na janela |
| **Construtor visual na janela real** (CDP com entrada de mouse real, etapa 3 do plano CI/CD) | clique no nó `preparo` abre o painel dele (`--click-node`, devolve `Nó preparo`) · arraste de `publicar` por (48, 16) → `translate(658, 200)` vira `704, 216`, **salvo** e lido de volta do banco com a mesma posição · **Conferir** imprime `grafo de 8 nó(s) · 6 etapa(s) do motor, 1 de comando, 1 de aprovação` (frase do validador do backend, não da tela) · publicar o grafo inteiro (execução **#33**) → `success`, resumo `grafo · 8 de 8 nós`, oito linhas nomeadas pela **chave do nó** com `touchedDestination` derivado (`false` em validar/git/coleta/preparo/gate, `true` em publicar/copia/historico) e o **modal de aprovação** parado no nó `gate` por 15 s até o **Aprovar** da janela (`04:43:02 → 04:43:17`; a barra de estado da tela mostrava `07 copia aguardando`, `08 historico aguardando` enquanto isso) · trocar o comando do nó `preparo` para `exit 3` na tela e salvar (**v7**) e publicar (**#34**): `preparo failed`, `publicar done` (ramal irmão, dependência dele estava boa), e `gate`, `copia`, `historico` em `skipped` com `uma dependência sua falhou nesta corrida` — cascada de três níveis, nada copiado depois da falha · **Esvaziar** + **Salvar grafo** (**v9**) devolve `Definição voltou à fila de quinze etapas` e publicar (**#35**) escreve as **15 linhas** com `nodeKey` nulo e os nomes da tabela linear, `success` · restaurar o grafo de oito nós → **v10** |
| **Linhas terminais no binário final** (CDP com entrada de mouse real, depois do rebuild das 01:27) | **Rodar sozinho** no nó `preparo` → modal *Variáveis do nó "preparo"* → **Executar nó** → execução **#38** `success`, e a linha lida de `deploy_run`: `preparo done`, `startedAt 05:31:58Z`, `finishedAt 05:32:02Z`, `message "1 comando(s) nesta máquina"` — com `sleep 4` no lugar do comando, a duração de 4 s é a prova de que o início é real e não a hora da linha anterior. **Rodar sozinho** no nó `gate` → **#39** `success`, linha `gate done` com `message "aprovado na janela"` (`05:33:31Z → 05:33:42Z`, os 11 s sendo o tempo até eu clicar **Aprovar**). Antes disto as duas linhas saíam com a mensagem da fase anterior — `preparo` ainda dizendo `em execução` e o `gate` dizendo `esperando aprovação` (defeito D, §5 item 18). Editar o comando na tela para `echo preparo` e **Salvar grafo** → definição **v12**, lida de volta por `pipeline_nodes` (`commands ["echo preparo"]`, `executor local`, oito nós nas mesmas posições) |
| Frontend embutido no executável | conferido **dentro da janela**: o pacote das 09:37 servia `index-5zynmBe_.js` enquanto `dist/` já tinha `index-Cko7uGOK.js` (frontend velho dentro de um binário novo). O binário final desta sessão (18.644.480 B, 01:27) serve `/assets/index-DT3bM82P.js` lido de `document.querySelectorAll("script[src]")` na janela aberta, o mesmo hash de `dist/index.html` |
| Instalador | `npm run tauri build` (01:27, saída 0) → `DevOps Desktop_0.1.0_x64-setup.exe` 4.959.838 B (4.73 MiB) e `DevOps Desktop_0.1.0_x64_en-US.msi` 7.151.616 B (6.82 MiB) |
| Executando | `devops-desktop.exe` aberto por `Start-Process`, `MainWindowTitle = "DevOps Desktop — Central de SSH, Git e Deploy"`, `Responding = True`, 10 telas navegáveis por clique |
| Git | `git push origin main` → `efe551d..23beec2`, saída 0, e `git ls-remote origin refs/heads/main` devolve `23beec23a4e069e2c1adb32560d8b1568876f392`, o mesmo `git rev-parse HEAD` local — etapa 3 pública em <https://github.com/DouglasGDev/devops-desktop> |

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

Para o grafo da aba Deploy, expressão não basta: `setPointerCapture` recusa um ponteiro que o navegador não criou,
então selecionar e arrastar um nó exige entrada real. O piloto tem três modos para isso, e todos lêem a posição do nó
**na mesma sessão** em que clicam — entre duas chamadas separadas a janela é redimensionada ou o canvas rola, e o
clique cairia no nó errado:

```bash
node scripts/ponte-cdp.mjs --mouse pressed 747 428      # pressed | released | moved | wheel, em px do viewport
node scripts/ponte-cdp.mjs --click-node preparo         # seleciona o nó pelo nodeKey e devolve o painel aberto
node scripts/ponte-cdp.mjs --drag-node publicar 48 16   # arrasta e devolve o transform antes e o painel depois
```

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
- **Construtor visual do pipeline** (etapa 3 do plano CI/CD — `src-tauri/src/deploy.rs`, `src-tauri/src/pipeline.rs`,
  `src/components/deploy/PipelineGraph.tsx`): o grafo desenhado na tela é o que o motor percorre, e **não há um segundo
  motor** — um nó de `etapa do motor` chama `etapa_do_motor(n)` e cai na mesma função das quinze etapas; um nó de
  `comando` usa o mesmo executor do contexto (`sshexec::run_local_with_env` aqui, `ctx.remote` no servidor); um nó de
  `aprovação` espera a mesma decisão explícita da janela, e sem resposta o resultado é `sem resposta dentro do prazo`,
  nunca "aprovado". Três tipos de nó, arestas por `dependsOn`, arraste com o mouse (SVG próprio, `setPointerCapture`,
  grade de 8 px — nenhuma biblioteca de grafo, nenhuma dependência nova), painel do nó com etapa/executor/comandos/pasta
  no projeto/ambiente/condição/prazo/repetição/aprovação/paralelismo e as dependências marcadas por checkbox.
  O escalonador (`deploy::grafo`) roda o que estiver `pronto`, mantém ramais de comando ao mesmo tempo (`Ctx::branch`,
  que clona `files`/`release`/`backup` — por isso paralelo só vale para nó de comando, e o validador recusa o resto),
  para quando uma dependência falhou (`bloqueado` → a linha do dependente é `skipped` com o motivo, nunca sucesso) e
  respeita condição e prazo por nó. A validação é uma só (`pipeline::checar_grafo`, chamada por salvar, por conferir e
  pela etapa 1 do motor): ciclo nomeia os nós que o formam, aresta para nó inexistente e auto-dependência são recusadas,
  40 nós é o teto, `ssh` sem servidor cadastrado é recusado, e **uma etapa do motor sem ancestral que produza o que ela
  lê é recusada com a lista do que falta** (`prequisitos`: 5–8 exigem 4; 9–15 exigem 3, 4 e 8 — reconhecido por fechamento
  transitivo, não só pelo pai direto). `touches_destination` é **derivado** (motor a partir da 8, qualquer `ssh`), nunca
  digitado, e é essa marca — não a posição no grafo — que a política de repetição da etapa 2 consulta. Cada nó salvo
  vira retrato da definição (`spec.nos`), com x/y e arestas, para a execução apontar para o desenho que era.
  Compatibilidade total: definição sem nós = fila das quinze etapas, e a conferência devolve
  `sem grafo · as quinze etapas do motor, em fila`. Diagnóstico é o mesmo motor entrando pela porta do grafo:
  `pipeline_run_node` com `alvo` e `com_dependencias` (sozinho = só aquele nó; dependências fora do escopo não travam).
  Na tela: **Deploy → aba grafo**, com **Conferir**/**Salvar grafo**/**Esvaziar** no topo, **Nó** para acrescentar,
  **Rodar sozinho** e **Com dependências** no painel do nó, estado vivo por nó (a corrida em `deploy-step` pinta a linha
  do desenho) e o modal de aprovação com **Aprovar**/**Rejeitar** quando um nó pede autorização. Tudo isso foi lido na
  janela real com entrada de mouse (§2, execuções **#32–#39**, definição **v12**) — inclusive o arraste persistindo no
  banco e a fila de quinze etapas voltando quando o grafo é esvaziado.

Total da superfície: **136 comandos Tauri registrados**, 22 arquivos Rust em `src-tauri/src`, 10 views,
36.951 linhas de TS/TSX/Rust.

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
13. **A linha do histórico de um nó do grafo é nomeada pelo nó, nunca pela posição.** `executar_no` marca o fim do nó
    depois de limpar `ctx.node_row`, e `mark()` — que é o caminho das etapas do motor — resolve o nome pela tabela das
    quinze etapas quando não há nó em execução: a posição do grafo (índice salvo + 1) não é número de etapa, e um nó de
    comando chamado `preparo` no índice 3 escrevia a linha 4 com o nome "Identificar arquivos alterados" e continuava
    com `node_key` velho, porque `upsert_step` não reescreve essa coluna. Medido na janela real (execução #31, nó
    `preparo` rodando sozinho), e corrigido na causa: as marcas finais de um nó passam por `registra(&Linha::from(&row))`,
    com o nome e a chave do nó. Uma execução linear (`node_row` ausente de verdade) continua usando a tabela.
14. **Nada de biblioteca de grafo.** O construtor é SVG desenhado à mão neste arquivo (`PipelineGraph.tsx`): 40 nós no
    teto, arestas em curva Bézier, arraste por `setPointerCapture`. O pedido foi não substituir o frontend e só trazer
    dependência se fosse necessária — não foi. O canvas cresce com o desenho (`largura`/`altura` a partir dos x/y dos nós):
    com a largura presa em 920 px um grafo de sete colunas era cortado do lado direito, medido na janela com os nós
    chegando a x=1484.
15. **Na corrida de um grafo, falha trava a corrente inteira — não só o filho direto.** O escalonador marcava o nó
    bloqueado como `Pulado`, e `Pulado` é "terminou bem o suficiente": o neto do nó que falhou ficava `pronto` e rodava.
    Num desenho com `coleta → publicar/preparo → gate → copia → historico`, isso significava **copiar para o destino
    depois de uma etapa anterior ter falhado**, com a política de repetição ainda por cima dizendo que o destino não foi
    tocado. Hoje `Sit::Bloqueado` é estado próprio, `bloqueado()` aceita `Falho` **e** `Bloqueado`, a cascada é fechada
    num laço dentro da mesma iteração (sem isso, um nível por passagem bastaria marcar o filho direto e a passagem
    seguinte cairia no ramo de beco sem saída, devolvendo "o grafo parou" no lugar da falha real), e o
    resumo da corrida separa `pulado(s) por condição` de `travado(s) por falha`. Condição não atendida continua
    deixando o dependente rodar — é o caso do `ramo=` que não é o atual. Coberto por
    `falha_trava_a_corrente_inteira_e_a_condicao_nao_propaga_nada`.
16. **Um nó de comando marca a própria linha como "em execução".** Nas quinze etapas em fila é `step()` quem faz essa
    marca, e é ela que grava `started_at`; um nó de comando não passa por `step`, então a linha do histórico nascia sem
    hora de início nem duração (lido na execução #33: `preparo` com `startedAt: null` e `finishedAt` preenchido).
    `rodar_comandos` marca agora no início, cobrindo `local` e `ssh` — `ctx.mark` pela posição do grafo é o mesmo
    caminho que a checagem de condição já usava, e o bitset `touched` daquela posição não é lido em corrida de grafo.
17. **Comando que falha sem escrever nada não deixa a linha sem motivo.** `tail(stderr, stdout)` devolve string vazia
    quando o processo só sai com código diferente de zero, e essa vazia virava a mensagem da etapa **e** o `detail` da
    corrida (execução #34: `preparo failed` com `message: ""` e a corrida `failed` com `detail: ""` — na tela, "sem
    detalhes"). Agora o fallback é `ExecResult::summary()` ("código de saída 3"), o mesmo que a ramo `ssh` já fazia, e o
    texto de tempo limite de `sshexec` parou de dizer "remoto" porque é usado pelos dois executores.
18. **A marca terminal do nó manda na mensagem da linha.** A linha `done` de um nó ainda trazia a mensagem da marca
    anterior: `preparo` concluía com `em execução` (lido em #37: `st "done"`, `msg "em execução"`) e o `gate` aprovado
    continuava com `esperando aprovação` (lido em #33). Dois motivos, os dois corrigidos: `upsert_step` só aplicava
    `COALESCE` na mensagem, então uma marca terminal não tinha como corrigir a linha — hoje `done`/`failed`/`skipped`
    **substituem** o que estava lá, e nas marcas do meio do caminho (`waiting`/`running`) continua valendo o `COALESCE`:
    quem traz mensagem escreve a sua (`running` com "em execução", que é o que grava `started_at`), quem traz `NULL` deixa
    a que já estava; e `executar_no` reescrevia a linha final de **todo** nó, inclusive
    os de `motor`, apagando a mensagem que a própria etapa escreveu e transformando uma `pulada` em `concluída` — agora
    quem tem etapa por trás termina por ela, e só comando e aprovação escrevem o próprio motivo ("N comando(s) nesta
    máquina/no servidor", "aprovado na janela"). Coberto por
    `a_marca_terminal_do_no_e_quem_manda_na_mensagem_da_linha` e lido no binário final (§2).

## 6. Pendências assumidas (com o primeiro passo de cada)

| # | Pendência | Primeiro passo concreto |
| --- | --- | --- |
| 0 | **Dados do piloto das etapas 2 e 3, ainda cadastrados de propósito** | O projeto **Piloto Etapa 2** (`%TEMP%\piloto-etapa2`) com a definição *Piloto sem servidor* **v12** e as execuções **#23–#39** continuam no cadastro para ele abrir a janela e ver as telas conferidas — a v12 é o grafo de oito nós (`validar → git → coleta → preparo/publicar → gate → copia → historico`, com `preparo` de volta a `echo preparo`), a #34 é a corrida em que `preparo` falha e trava o resto, e a #38/#39 são as duas execuções de nó sozinho lidas no binário final (`preparo` → "1 comando(s) nesta máquina", `gate` → "aprovado na janela"). Quando ele decidir, o primeiro passo é **Projetos → Piloto Etapa 2 → Remover do cadastro** (a confirmação avisa quantos modelos saem junto; nenhum arquivo local é apagado) e depois apagar a pasta `%TEMP%\piloto-etapa2`. O arquivo de teste de 1,5 GB já foi removido do projeto e do destino |
| 1 | **Client ID na máquina do Douglas** | Colar o **Client ID** da OAuth App (o campo está em **GitHub → Configuração**) e gerar um secret novo no GitHub — o valor hexadecimal que estava salvo no banco era o **secret**, não o ID. Sem isso a aba GitHub não conecta: é limite de conta, não do código |
| 2 | **Passada de visual (prioridade quando ele chamar)** | Só depois das funções; começar pelo print da tela que incomoda. Não refazer estilo por conta própria junto de feature |
| 3 | **Pacotes Linux** | Os alvos `deb`/`rpm`/`AppImage` nunca foram gerados aqui. Rodar `npm run tauri build` numa imagem Linux e ajustar `bundle.targets` + paths (o código já usa `dirs`/caminhos relativos, mas isso precisa ser provado, não suposto) |
| 4 | **Teste de interface automatizado** | Não há runner com asserções. O que existe hoje é o piloto `scripts/ponte-cdp.mjs` (§4.1), que dirige a janela real mas exige leitura humana a cada passo. Se um runner entrar, o mínimo é o fluxo do SPEC §17; a cobertura automática continua `tsc` + suíte Rust + `scripts/confira-repo.ts` |
| 5 | **Aceite visual das últimas entregas** | `Git → Árvore`, `GitHub → repositório → Local`, `Git → Guardados`, os botões *Reproduzir este commit* / *Reverter este commit* / *Voltar para este commit*, o **fluxo de deploy sem servidor** (modelo com destino "Nesta máquina", pré-vistoria, publicar, reverter), o **contrato de pipeline** — em **Projetos → Modelos de deploy** o badge `v{n}`, o botão **Conferir** e a lista de **Versões salvas**; em **Deploy** a linha de procedência da execução (`definição v{n} · ambiente · destino · commit`), o `· v{n} · {sha8}` no histórico e o **Repetir** com o painel âmbar quando a política recusa — e o **construtor visual** (aba **grafo**: nós arrastáveis, painel do nó, **Conferir**/**Salvar grafo**/**Esvaziar**, **Rodar sozinho** / **Com dependências**, estado vivo por nó durante a corrida e o modal de **Aprovação** com Aprovar/Rejeitar). Eu abri cada uma dessas telas e li o texto na janela real, mas **ele** ainda não viu nada disso nesta máquina |
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
