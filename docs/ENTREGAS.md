# Registro de entregas — onde está cada coisa, o que falta e como retomar

Documento de retomada. Ele não substitui o [README](../README.md) (uso do aplicativo) nem o
[SPEC.md](SPEC.md) (o que foi pedido); ele registra **o que já está pronto, onde está o código, como se prova
cada parte e o que ainda não existe**. A base destas linhas é o modo sem servidor (`66da3a7`), a **etapa 2 do plano
CI/CD** — o contrato central de pipeline (`4f005ee`) e a licença de contribuição (`df51de1`) —, a **etapa 3**, o
construtor visual do grafo (`23beec2`), a **etapa 4**, os executores de runtime (Docker, Docker Compose, PM2) com o
git como passo do pipeline, e a **etapa 5**, a análise de código-fonte — na tela e como nó do mesmo grafo —, registrada
aqui em 2026-10-15.

## 1. Retomar em três passos

```bash
node ./node_modules/typescript/bin/tsc --noEmit     # tipos do frontend
npm run build                                        # tsc + bundle
(cd src-tauri && cargo test)                         # 303 testes de backend
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

## 2. Estado verificado em 2026-10-10 … 15 (resultado lido, não presumido)

| Checagem | Resultado real |
| --- | --- |
| `tsc --noEmit` | saída vazia, código 0 — relido nesta rodada **duas vezes**: uma antes do rebuild e outra depois de `ui.tsx` ganhar o rótulo da categoria *Análise*, e ainda dentro do `npm run build` que antecede o binário das 22:25 |
| `npm run build` | `✓ 85 modules transformed`; `dist/assets/index-DZ1dExmJ.js` **882.41 kB** e `index-nrYLLjTt.css` **61.92 kB**; `✓ built in 7.72s`; único aviso é o de chunk > 500 kB, que é de tamanho de bundle, não de código. Lido **dentro da janela** das 22:25: `document.querySelectorAll("script[src]")` devolve exatamente `["/assets/index-DZ1dExmJ.js"]` — o frontend embutido no binário é o do disco, e é a primeira vez que o hash muda desde a etapa 5, porque o rótulo novo é de tela |
| `cargo test` (todos os alvos) | **`303 passed; 0 failed; 0 ignored; 0 measured; finished in 19,31 s`** em `devops_desktop_lib` (relido duas vezes nesta rodada: `17,72 s` antes da última correção de texto e `19,31 s` depois, sempre sem nenhuma linha de `warning`); `main.rs` e doc-tests com 0 testes. Os dois testes novos são os que travam o texto gravado na auditoria (`a_sonda_grava_o_valor_apontado_quando_a_ferramenta_nao_esta_aqui`, `a_conferencia_gravada_diz_o_estado_e_nunca_presume_ok`), e cada um foi lido pelo nome antes de entrar na contagem. Histórico: `301 passed` (fechamento do gate), `300 passed` (fim da etapa 5), `261 passed` (etapa 4), `229 passed` (etapa 3), `208 passed` (antes da etapa 3) |
| `cargo clippy --all-targets` | **execução forçada**, depois de `cargo clean -p devops-desktop` (369 arquivos / 569.7 MiB removidos nesta rodada) para o clippy não responder do cache: **zero linhas `warning`/`error`**, código 0 — e relido com `cargo clippy --all-targets -- -D warnings`, que transforma aviso em falha, também `Finished` sem nada; refeito com `-D warnings` depois da última mudança, outra vez sem nada. Nenhum `#[allow]` foi acrescentado para chegar aí |
| `node scripts/confira-repo.ts` | `todas as 9 formas conferem`, código 0 (relido nesta rodada, antes do rebuild) |
| Migração `user_version` **9** no banco real | o banco de `AppData/Roaming/com.douglasgdev.devopsdesktop/devops-desktop.db` abriu em `user_version 7` e **foi lido de novo em 9** pela janela real desta sessão. Colunas novas conferidas no arquivo: em `deploy_profiles` — `runtime image_name dockerfile compose_file service_name publish_ports`; em `deploy_runs` — `previous_image`; em `pipeline_nodes` — `git_op git_ref`. Os **oito nós da etapa 3 sobreviveram** ao rebuild da tabela `pipeline_nodes` da migração 8 (lido de volta: `validar, git, coleta, preparo, publicar, gate, copia, historico`) |
| **Fluxo sem servidor na janela real** (CDP, §4.1) | modelo `serverId: null` salvo · pré-vistoria `2 arquivo(s), 72.1 KB` · publicar #1 `success` com as 15 etapas · guarda `.devops-backups/1` com o README antigo **e** o excedente movido · reverter #2 `2 arquivo(s) reposto(s)` (conferido no disco) · publicar #3 `1 copiado · 1 já igual` (CRC32) · destino dentro do projeto **recusado na etapa 1** sem tocar no disco · caminho duplicado recusado (`o projeto já tem um modelo de deploy chamado "…"`) · remover o modelo → `deploy_runs` 5 → 0 e `audit_log` 57 → 58 (o histórico fica na auditoria, como a confirmação diz) · UI do modo local conferida no exe: destino "Nesta máquina (sem SSH)", estratégias só **C**/**D**, rótulo "Guardar o que já está no destino antes de sobrescrever", sem o checkbox de releases, e os quatro diálogos (pré-vistoria → publicar → reverter → remover) lidos na tela |
| As quatro portas do "o deploy gera o próprio artefato" (exe das 11:01) | **W1** artefatos sem seleção + build gerando `saindo-do-build.txt`: 61 copiados, arquivo conferido no destino · **W2** artefatos declarando `gerado-pela-preparacao.txt` que **ainda não existe**: etapa 4 anuncia, preparação gera, 1 copiado, `success` · **W3** caminho que nenhum comando gera: `failed` na etapa 8 com "depois da preparação não sobrou nenhum arquivo para enviar — confira os caminhos" · **W4** `script`: `success` sem copiar nada (destino ficou vazio, como projetado) |
| **Contrato de pipeline na janela real** (CDP, etapa 2 do plano CI/CD) | definição criada como **v1** · execução #23 `success` com `definition_version 1`, `environment "local"`, `destination` o caminho absoluto do destino e `commit_sha f40243ff12c2a6…` **igual ao `git rev-parse HEAD`** lido na pasta · editar a definição → **v2**, salvar de novo sem mudar → **v3**, e `pipeline_revisions` devolve os três retratos (`v3`, `v2`, `v1`) em que só o `v2` carrega `exclusao: ["src/index.html"]` — ou seja, o retrato guarda o que mudou, e `variaveis` sai como `[]` porque o retrato não tem valor nenhum · `pipeline_validate` → `definição “Piloto sem servidor” v3 · artifacts · destino C:\… · nesta máquina` · **Repetir** o #23 criou a #24 com `definition_version 3` enquanto a #23 **continua `v1`** (a procedência antiga não é reescrita por edição posterior) · #25 com um arquivo do destino travado por outro processo: `failed` na etapa 9 com `falha ao guardar README.md antes de sobrescrever: O arquivo já está sendo usado por outro processo. (os error 32)`, etapas 10–15 em `waiting`, `pipeline_retry_policy` → `allowed: false` com o motivo, e `pipeline_retry` **recusa a mesma frase sem abrir execução nenhuma** (as execuções param em #27 — não existe #28 de retry) · reversão da #25 → execução própria com `1`, `2`, `11`, `14`, `15` como `done` e `3`–`10`+`12`–`13` como `skipped`, `1 arquivo(s) reposto(s)`, e o `README.md` do destino **conferido no disco** de volta ao conteúdo anterior · execução de 1,5 GB com o aplicativo **fechado à força no meio**: ao reabrir, a #30 está `cancelled` com `interrompida — o aplicativo foi fechado antes de terminar`, a etapa 9 está `failed` com `interrompida — o aplicativo foi fechado`, e a auditoria ganhou a linha `Execuções interrompidas identificadas · 1 execução(ões) e 1 etapa(s) marcadas como interr…` — a varredura rodou no `setup`, não num teste · tela: `v5` no cabeçalho do modelo e no seletor da aba **Deploy**, **Conferir** imprimindo `definição “Piloto sem servidor” v5 · artifacts · destino C:\… · nesta máquina`, **Versões salvas** listando `v5 v4 v3 v2 v1`, histórico com `· v4 · f40243ff` por execução, painel âmbar do **Repetir** mostrando o motivo da recusa lido na janela |
| **Construtor visual na janela real** (CDP com entrada de mouse real, etapa 3 do plano CI/CD) | clique no nó `preparo` abre o painel dele (`--click-node`, devolve `Nó preparo`) · arraste de `publicar` por (48, 16) → `translate(658, 200)` vira `704, 216`, **salvo** e lido de volta do banco com a mesma posição · **Conferir** imprime `grafo de 8 nó(s) · 6 etapa(s) do motor, 1 de comando, 1 de aprovação` (frase do validador do backend, não da tela) · publicar o grafo inteiro (execução **#33**) → `success`, resumo `grafo · 8 de 8 nós`, oito linhas nomeadas pela **chave do nó** com `touchedDestination` derivado (`false` em validar/git/coleta/preparo/gate, `true` em publicar/copia/historico) e o **modal de aprovação** parado no nó `gate` por 15 s até o **Aprovar** da janela (`04:43:02 → 04:43:17`; a barra de estado da tela mostrava `07 copia aguardando`, `08 historico aguardando` enquanto isso) · trocar o comando do nó `preparo` para `exit 3` na tela e salvar (**v7**) e publicar (**#34**): `preparo failed`, `publicar done` (ramal irmão, dependência dele estava boa), e `gate`, `copia`, `historico` em `skipped` com `uma dependência sua falhou nesta corrida` — cascada de três níveis, nada copiado depois da falha · **Esvaziar** + **Salvar grafo** (**v9**) devolve `Definição voltou à fila de quinze etapas` e publicar (**#35**) escreve as **15 linhas** com `nodeKey` nulo e os nomes da tabela linear, `success` · restaurar o grafo de oito nós → **v10** |
| **Linhas terminais no binário final** (CDP com entrada de mouse real, depois do rebuild das 01:27) | **Rodar sozinho** no nó `preparo` → modal *Variáveis do nó "preparo"* → **Executar nó** → execução **#38** `success`, e a linha lida de `deploy_run`: `preparo done`, `startedAt 05:31:58Z`, `finishedAt 05:32:02Z`, `message "1 comando(s) nesta máquina"` — com `sleep 4` no lugar do comando, a duração de 4 s é a prova de que o início é real e não a hora da linha anterior. **Rodar sozinho** no nó `gate` → **#39** `success`, linha `gate done` com `message "aprovado na janela"` (`05:33:31Z → 05:33:42Z`, os 11 s sendo o tempo até eu clicar **Aprovar**). Antes disto as duas linhas saíam com a mensagem da fase anterior — `preparo` ainda dizendo `em execução` e o `gate` dizendo `esperando aprovação` (defeito D, §5 item 18). Editar o comando na tela para `echo preparo` e **Salvar grafo** → definição **v12**, lida de volta por `pipeline_nodes` (`commands ["echo preparo"]`, `executor local`, oito nós nas mesmas posições) |
| **Git como passo do pipeline na janela real** (CDP, etapa 4 do plano CI/CD) | nó `marca-tag` de tipo **git** criado na aba **grafo** e salvo → **Conferir** devolve `grafo de 9 nó(s) · 6 etapa(s) do motor, 1 de comando, 1 de aprovação, 1 de git`; publicar → execução **#42** com a linha `03 + git tag v0.0.1-janela` no log e a mensagem da corrida **"tag v0.0.1-janela criada nesta máquina — sem remoto cadastrado"** (a resposta é do próprio git, lida em `git tag -l` na pasta do piloto: o commit `f40243f` estava etiquetado de verdade) · execução **#43** com nó de `checkout` → mensagem **"em main"** · escrever `../fora` no campo **Pasta no projeto** do nó git e **Conferir** → recusa `nó "marca-tag": a pasta de trabalho deve ser um caminho relativo dentro do projeto`, e o grafo não salva · remover o nó e salvar → `grafo de 8 nó(s) · … 0 de git` (lido de novo no binário das 14:35, com `pipeline_nodes` devolvendo as oito chaves e nenhum `git_op`) |
| **Executores de runtime na tela** (CDP, janela das 14:05 — mesmo fonte, binário anterior à última linha de tela) | o seletor de executor **não aparece** quando o destino é "nesta máquina" (um executor de runtime é POSIX e roda no servidor — `models.rs` recusa: "…é um executor de runtime e roda no servidor") e **aparece** com um servidor cadastrado, com as opções **Nenhum / Docker / Docker Compose / PM2** · **Docker**: campos *Nome da imagem*, *Dockerfile*, *Nome do container*, *Portas publicadas*; com imagem vazia a tela recusa `o Docker precisa do nome da imagem (ex.: meuapp ou registry.local/meuapp)`; salvo, o perfil 21 ficou `runtime docker, image_name meuapp, service_name site-medicao, publish_ports ["8080:80"], version 1` · **Docker Compose**: exatamente *Arquivo do compose* e *Serviço*, e o salvamento deixou `image_name null, dockerfile null, publish_ports []` — o modelo não herda campo do executor que ele não é · **PM2**: *App no PM2*, recusa `o PM2 precisa do nome do app como ele aparece no pm2 list`, salvo `runtime pm2, service_name api, version 3` · o retrato de versão guarda `executor/imagem/servico/portas` e **Conferir a definição** imprimiu `definição "medição docker" v3 · sftp · destino /srv/medicao · servidor SSH · sem grafo · as quinze etapas do motor, em fila` · tudo que foi criado para medir foi desfeito: modelo 21 removido pela confirmação da tela, servidor temporário apagado (`list_servers` → 0), grafo de volta aos oito nós |
| **Higiene da tela no binário final** (CDP) | o parágrafo "*Rode a pré-vistoria antes de publicar: ela mostra o que subiria e o que a checagem de segredos encontraria. O botão "Publicar" fica bloqueado até aí.*" **não está mais na tela** (`document.body.innerText` não contém a frase) — ele virou documentação no README §9.6. No lugar ficou o **estado**, no `title` de quem passa por cima do botão, lido no DOM: `span[title="Rode a pré-vistoria antes de publicar"]` envolvendo um botão com `disabled: true`. **Por que no pai e não no botão:** `Button` aplica `disabled:pointer-events-none` (`src/components/ui.tsx:44`), e um botão que não recebe ponteiro não mostra dica nenhuma — medido com `getComputedStyle`: botão `pointerEvents: "none"`, span `pointerEvents: "auto"`. O texto não é mais um só: `motivoPublicar` devolve o motivo do estado atual (sem modelo, pré-vistoria pendente, itens
bloqueados sem autorização, execução em andamento, pronto). **Na janela só a variante "pré-vistoria pendente" foi lida de
fato** — as outras quatro são o que o código devolve, e continuam pedindo o aceite visual dele (§6, item 5) |
| Migração `user_version` **10** no banco real | o que prova as tabelas novas **não** é um `PRAGMA` lido agora, e sim o round-trip pelos comandos, na janela aberta hoje: `analysis_corridas` devolve **8** corridas gravadas (ids 1–8, com `project`, `root`, `rules`, `counts`, `blocked`) e `analysis_achados` devolve os achados de uma delas (`runId 6` → 12 linhas com `arquivo`, `linha`, `regra`, `severidade`, `cwe`, `trechoMascarado`); `pipeline_nodes` devolve para cada nó os campos `analyzer` e `severityGate`, que só existem porque a migração 10 os acrescentou em `pipeline_nodes`. O piloto da etapa 5 é o projeto **4** (`%TEMP%\piloto-etapa5`) e o do motor continua o **3** |
| **Análise de código na janela real** (CDP, etapa 5 do plano CI/CD) | sonda da ferramenta lida agora: `analysis_detectar` → `estado "encontrado"`, `caminho C:\Users\dougl\AppData\Local\Temp\semgrep-probe\Scripts\semgrep.exe`, `versao "1.181.0"`, `duracaoMs 2609`, `regras 19`, `origem "configuração"` — ou seja, o semgrep **não está no `PATH`** desta máquina e é exatamente para isso que existe `analise.semgrep.caminho`, lido de `get_settings` com esse mesmo valor · `analysis_regras` → **19** regras embarcadas (`dd-101-shell-true`, `dd-102-sql-montado`, …) · `analysis_conferir` → `estado "validadas"`, `regras 19`, `codigo 0`, com a frase do próprio semgrep `Configuration is valid - found 0 configuration error(s), and 19 rule(s).` · **corrida #6** na tela, com o semgrep real: `status "concluida"`, `filesScanned 3`, `exitCode 0`, `toolErrors 0`, `counts {error 8, warning 4, info 0}`, `severityGate "ERROR"`, **`blocked: true`** — veredito da contagem contra o corte · os **12** achados lidos um a um, cada um com o trecho **relido do disco e mascarado**: `subprocess.call("curl -H 'Authorization: Bearer «valor mascarado»' https://api.example", shell=True)` e, filtrando o JSON inteiro dos 12 por `ghp_|sk-|token=|secret=`, **0** campos batem · a mesma #6 exportada como pacote: `esquema "devops-desktop/analise@1"`, `veredito {classificacao: "barra", bloqueia: true, corte: "ERROR", contagem: {error 8, warning 4, total 12}}`, `decisoes [{acao: "parar-a-publicacao", motivo: "12 achado(s) no corte ERROR ou acima"}]`, `cobertura {arquivosLidos 3, arquivosPulados 0, achadosGuardados 12}`, `execucao.codigoDeSaidaEProva: false` (e `codigoDeSaidaDaFerramenta: 0` escrito ao lado, para ficar claro que ele **não** é a prova), `truncado: false`, `nota: null`, `observacao "nenhum valor de credencial está aqui: os trechos passam pela mascaráção de security.rs antes de serem gravados"`, e um achado com `cwe "CWE-78: OS Command Injection"` |
| **Nó `analise` rodando no mesmo motor** (CDP, etapa 5) | três execuções do **`deploy::start`** com o nó de análise no grafo, lidas hoje em `deploy_run`: **#44** `failed` com a linha `Nó 9:failed` às `19:54:10Z` — e a corrida de análise **do mesmo segundo** (#5) diz `tool "semgrep ausente"`, `status "falhou"`, `filesScanned 0`: **sem leitura não é verde** · **#45** `success` (`20:19:59Z → 20:20:05Z`) com a corrida **#7** `instrumento semgrep`, `versao 1.181.0`, `exitCode 0`, `concluida` · **#46** `success` (`20:22:26Z`) com a corrida **#8** `instrumento embutido`, 2 arquivos lidos, 1 ms, `exitCode null` (o embutido não tem processo externo, e mesmo assim a corrida é `concluida`) — as duas portas, a mesma `analysis::executar` · o `../fora` no nó de análise é recusado pelo **mesmo** validador do nó de comando, lido agora com `pipeline_check_nodes`: `nó “preparo”: a pasta de trabalho deve ser um caminho relativo dentro do projeto`, sem salvar nada · o grafo do piloto voltou ao desenho de origem e está lido assim: `pipeline_nodes` → **8** nós (`validar, git, coleta, preparo, publicar, gate, copia, historico`), `kinds` = `motor 6 · comando 1 · aprovação 1`, e `pipeline_validate` imprimindo `definição “Piloto sem servidor” v18 · artifacts · destino C:\Users\dougl\AppData\Local\Temp\piloto-etapa2\publicado · nesta máquina · grafo de 8 nó(s) · 6 etapa(s) do motor, 1 de comando, 1 de aprovação, 0 de git, 0 de análise` |
| **Binário e instaladores desta rodada (22:18 → 22:25)** | dois builds completos, ambos lidos do log, não do código de saída: o primeiro (log dizendo `Finished release profile [optimized] target(s) in 8m 51s`) gravou `devops-desktop.exe` **19.443.712 B** às **22:13**, `bundle/nsis/…-setup.exe` **5.143.893 B** às 22:12 e `bundle/msi/…_en-US.msi` **8.572.928 B** às 22:13; depois da correção do texto da linha, o rebuild de **7m 11s** regravou os três às **22:25** — exe **19.444.224 B**, nsis **5.144.116 B**, msi **8.572.928 B** (o `.msi` com o mesmo tamanho lido nas duas rodadas; o que mudou entre elas é o `.rs`, não o pacote). **Foi o binário das 22:25 que abriu a janela das medições acima**, e a janela serve `/assets/index-DZ1dExmJ.js`, o hash que o `vite` gravou em `dist/assets/` no mesmo minuto |
| **As duas verificações da tela deixam linha no Histórico** (CDP, pedido direto do Douglas: *"os logs do verificar nao salva nada quando vcc aponta"*) | medido no binário das **22:25**, com o valor que **ele mesmo** tinha apontado em Configurações (`analise.semgrep.caminho` = `E:\ssh-master\dist\`, lido por `get_settings`). **`analysis_detectar`** devolve `estado "ausente"`, `apontado "E:\ssh-master\dist\"`, `origem ""`, `caminho null`, `versao null`, `duracaoMs 1`, e a linha gravada aparece no `query_audit_log` com `id 250`, `level "warning"`, `category "análise"`, `operation "Sonda do semgrep · ausente"` e `detail` exatamente `apontado “E:\ssh-master\dist\” · nenhum binário resolvido a partir dali · 1 ms · o instrumento externo não está nesta máquina; o embutido continua disponível` — é o caso que ele apontou: sem o `apontado` no `Ferramenta`, a linha diria "não achei nada" sem dizer o que foi tentado. **`analysis_conferir`** em seguida devolve `estado "sem-ferramenta"`, `regras 19`, `codigo null`, `mensagem "não há semgrep nesta máquina — as 19 regras embarcadas não foram conferidas por fora"`, e a linha `id 251` é `warning` · `Conferência das regras · sem-ferramenta · 19 regra(s)` · `sem código de saída · …` — ausência de ferramenta **não** é verde. Depois restaurar o caminho que existe de verdade (`C:\Users\dougl\AppData\Local\Temp\semgrep-probe\Scripts\semgrep.exe`, lido do disco antes de escrever): a sonda responde `encontrado`, `versao "1.181.0"`, `origem "configuração"`, `duracaoMs 5620` e grava `id 253` como `info` · `Sonda do semgrep · encontrado · 19 regra(s) embarcada(s)`, e a conferência responde `validadas`, `codigo 0`, `mensagem "Configuration is valid - found 0 configuration error(s), and 19 rule(s)."` gravando `id 254` como `info` · `Conferência das regras · validadas · 19 regra(s)`. Na tela: **Histórico** com a categoria filtrada em *Análise* devolve **13 linhas** e as quatro primeiras são as quatro medições acima, com os rótulos `Info`/`Aviso`; o `select` da categoria ganhou a chave `análise` (sem ela a linha aparecia em "Todas" com o texto cru e **não dava para filtrar por ela**), lido pelo `value` real `análise` e pelo `textContent` `Análise`. A troca do valor em Configurações também deixou rastro próprio (`id 252`, categoria `settings`, `Configuração alterada`). Fechado o §5 item 28. A janela em que tudo isso foi lido, conferida por `Get-Process devops-desktop`: `Id 29840`, `MainWindowTitle = "DevOps Desktop — Central de SSH, Git e Deploy"` — o travessão confirmado por código, não por
renderização: `([int[]][char[]]$p.MainWindowTitle)` devolve `…112,32,8212,32,67,101…`, e `8212` é U+2014 (o console desta
máquina imprime `("-",)` quando lê por `Format-Table`), `Responding = True` |
| **Publicação travada pelo corte da análise** (CDP, fecha o §6 item 7) | o `piloto-etapa5` virou modelo de deploy **para esta medição**: perfil 22 *Gate de análise* (`strategy artifacts`, `serverId: null`, destino `C:\Users\dougl\AppData\Local\Temp\piloto-etapa5-publicado`, `includePaths ["app.py","src","README.md"]`) com grafo de **2 nós** — `varredura` (`kind analise`, `executor local`, `analyzer embutido`, `severityGate ERROR`) e `depois-do-gate` (`kind comando`, `echo depois-do-gate`, depende de `varredura`). **Conferir** devolve `grafo de 2 nó(s) · 0 etapa(s) do motor, 1 de comando, 0 de aprovação, 0 de git, 1 de análise`, e a
definição grava como **v2**, lida de volta por `pipeline_validate`: `definição “Gate de análise” v2 · artifacts · destino
C:\Users\dougl\AppData\Local\Temp\piloto-etapa5-publicado · nesta máquina · grafo de 2 nó(s) · 0 etapa(s) do motor, 1 de
comando, 0 de aprovação, 0 de git, 1 de análise`. Publicado por `deploy_start`: execução **#47** (`startedAt 2026-10-16T00:32:54Z`, 20:32:54 no relógio da máquina) `failed`, `detail "o grafo parou: existe nó com dependência que não se completa"`, com as duas linhas lidas de `deploy_run` — `1 varredura failed` com `command "análise de código (corte ERROR)"` e mensagem **`embutido · erro 9, aviso 4, info 1 — a varredura de C:\\Users\\dougl\\AppData\\Local\\Temp\\piloto-etapa5 barra a publicação`**, e `2 depois-do-gate skipped` com `uma dependência sua falhou nesta corrida`. **A pasta de destino nunca foi criada** (`ls` → `No such file or directory`): o que a tela mostra como bloqueio é o que
aconteceu no disco, nada foi copiado. E o `README.md` declarado em `includePaths` **não existe** no piloto (a pasta tem
`app.py`, `src/legado.php`, `src/servidor.js`) — o que é legítimo, porque a corrida parou no gate antes de qualquer
etapa de cópia, e o caso "caminho declarado que nenhum comando gera" já tem a própria medição na linha das quatro
portas acima. A corrida de análise do mesmo segundo (**#11**) diz `tool "embutido"`, `status "concluida"`, `filesScanned 3`, `counts {error 9, warning 4, info 1}`, `severityGate "ERROR"`, **`blocked: true`** — o veredito veio da **contagem contra o corte**, e o pacote dela (`analysis_pacote` 11) repete `classificacao "barra"`, `bloqueia: true` e a decisão `{acao: "parar-a-publicacao", evidencia: "veredito.contagem", motivo: "14 achado(s) no corte ERROR ou acima"}`, com `cobertura {arquivosLidos 3, arquivosPulados 0, achadosGuardados 14}` e `truncado: false`. Medido de novo no binário das **20:42**: execução **#48** (`startedAt 2026-10-16T00:43:07Z`, 20:43:07) com as **mesmas duas linhas** e a corrida **#12** `blocked: true` com os mesmos 9/4/1 — a única diferença lida é o caminho exibido, que saiu de `\\?\C:\Users\…` para `C:\Users\…` (§5, item 27) |
| Frontend embutido no executável | conferido **dentro da janela**: o pacote das 09:37 servia `index-5zynmBe_.js` enquanto `dist/` já tinha `index-Cko7uGOK.js` (frontend velho dentro de um binário novo). O binário das **14:55** serve `/assets/index-BC07Lf6F.js` lido de `document.querySelectorAll("script[src]")`, o mesmo hash de `dist/assets/` da rodada das 14:48 — incluindo a tela sem o parágrafo de ajuda. **Etapa 5, binário das 16:11:** `document.querySelectorAll("script[src]")` devolve exatamente `["/assets/index-l9oCfRyf.js"]`, e `dist/assets/` tem `index-l9oCfRyf.js` (882.386 B) — o frontend dentro do instalador é o que está no disco. **Rebuild das 20:42** (só backend mudou): `document.querySelectorAll("script[src]")` devolve de novo exatamente `["/assets/index-l9oCfRyf.js"]`, o mesmo hash de `dist/assets/` — nada da tela foi reescrito nesta rodada |
| Instalador | `npm run tauri build` (14:55, saída 0) → `DevOps Desktop_0.1.0_x64-setup.exe` 5.002.154 B (4,77 MiB) e `DevOps Desktop_0.1.0_x64_en-US.msi` 7.200.768 B (6,87 MiB). **Etapa 5 (rodada das 16:11):** as duas linhas lidas **no arquivo de log da compilação**, não no código de saída de um pipe — `bundle/nsis/…-setup.exe` **4.90 MiB** e `bundle/msi/…_en-US.msi` **8.18 MiB**, com `devops-desktop.exe` gravado no mesmo minuto (foi justamente ignorar o pipe que me fez publicar um binário velho nesta sessão; §5, item 25) |
| Instalador, rebuild das **20:42** (o da mediação do gate) | as duas linhas do log do `npm run tauri build`: `Finished 2 bundles at:` com `bundle/nsis/DevOps Desktop_0.1.0_x64-setup.exe` **4.90 MiB** (5.133.902 B contados no arquivo) e `bundle/msi/DevOps Desktop_0.1.0_x64_en-US.msi` **8.18 MiB** (8.572.928 B), `devops-desktop.exe` com mtime **20:42** — foi esse binário que abriu a janela em que a corrida **#48** foi lida |
| **Rebuild das 22:25** (o das verificações gravadas) | lido do arquivo de log da compilação, não do código de saída de um pipe: `Finished \`release\` profile [optimized] target(s) in 7m 11s`, `Built application at: …\target\release\devops-desktop.exe`, `Finished 2 bundles at:` com `bundle/nsis/DevOps Desktop_0.1.0_x64-setup.exe` **4.91 MiB** (5.144.116 B contados no arquivo) e `bundle/msi/DevOps Desktop_0.1.0_x64_en-US.msi` **8.18 MiB** (8.572.928 B); `devops-desktop.exe` com mtime **22:25** lido junto do `date`. A janela que estava aberta (das 22:14) foi **fechada antes do link**, porque o `cargo` não sobrescreve um `.exe` em uso — foi a precaução desta vez, não uma falha lida: o link escreveu 22:25 sem reclamar. Foi esse binário que abriu a janela em que as quatro linhas do §2 (ids 250/251/253/254) foram lidas |
| **Cartão de progresso reconciliado** (CDP, etapa 5) | o **#46** começou e terminou no mesmo segundo (`16:22:26 → 16:22:26`), caso em que a leitura do detalhe chega **depois** do evento de fim — foi assim que ele ficou preso em "em andamento" com a corrida já `success` no banco (defeito #68, visto no print da janela das 16:12 com o **#45**). Lido de novo no binário das 16:11, às **16:33**: a linha do histórico `#46 Piloto Etapa 2 → nesta máquina … 16:22:26 · v17 · f40243ff · grafo · 1 de 1 nós` abre o cartão com `sucesso` e o botão **Repetir** presente, `em andamento` **ausente** do texto visível da tela. Causa e correção no §5, item 24 |
| Executando | `devops-desktop.exe` aberto por `Start-Process`, `MainWindowTitle = "DevOps Desktop — Central de SSH, Git e Deploy"`, `Responding = True`, 10 telas navegáveis por clique. **Etapa 5 (o exe das 16:11, aberto às 16:12, lido no DOM agora):** **11** entradas na navegação — `Dashboard, Servidores, Terminal, Arquivos remotos, Git, GitHub, Projetos, Deploy, Análise, Histórico, Configurações` |
| Git | `git push origin main` → `8e328c8..0308e84`, saída 0, e `git ls-remote origin refs/heads/main` devolve `0308e84c4b00f0c28c5c42e40dfbba0bb8e4d6b9`, o mesmo `git rev-parse HEAD` local — a **etapa 4 está pública** em <https://github.com/DouglasGDev/devops-desktop>. O push foi autorizado por ele nesta conversa, depois de eu registrar que a autorização anterior tinha coberto só a etapa 3. Etapa 3, medida na época: `efe551d..23beec2` |
| Git, etapa 5 (lido às 20:23) | o **primeiro push foi recusado** pelo GitHub: `! [remote rejected] main -> main (push declined due to repository rule violations)`, com `ls-remote` ainda em `df60996…` — o protetor de segredos do repositório viu num fixture de teste uma chave live de verdade (forma, não conteúdo; §5 item 26). Corrigido o fixture na causa, os commits locais reunidos em um só (`git reset --soft`, nenhum trabalho perdido) e então: `git push origin main` → **`df60996..5f4df6f`**, e `git ls-remote origin refs/heads/main` devolve `5f4df6fd882981a0c7c068fa7940f436d9e7ca14`, igual ao `git rev-parse HEAD` local. A **etapa 5 está pública**, autorizada por ele na pergunta desta conversa ("Empurrar agora") |
| Git, fechamento do gate (lido às 20:53) | `git push origin main` → **`5f4df6f..94344ae`** (o commit de docs `5d62e1a` foi junto), e `git ls-remote origin refs/heads/main` devolve `94344ae0a661d94a9f1e7c8a67ddd70934a108b3`, o mesmo `git rev-parse HEAD` local — a medição do §6 item 7 e a correção do §5 item 27 estão em <https://github.com/DouglasGDev/devops-desktop>. Autorizada por ele na mesma frase: "terminando o teste, publique no github" |

O que isso **não** cobre: as asserções automáticas continuam sendo de backend — o piloto de interface (§4.1) dirige a
janela de verdade, mas quem decide o que está certo a cada leitura sou eu, não uma suíte de testes de UI. O caminho por
SSH (`server_id` preenchido) **não** foi reexecutado hoje porque não há servidor cadastrado nesta máquina; por isso as
três correções desta sessão foram escritas só dentro de ramos protegidos por `local_mode()` (ou strategies que já eram
locais), para não mexer no que não pude medir. **E o mesmo vale para os executores de runtime da etapa 4**: um executor
de Docker/Compose/PM2 é um roteiro POSIX que roda **no servidor**, então nenhum `docker build`, `docker compose up` ou
`pm2 reload` foi executado contra um host real — o que foi medido é o roteiro gerado (teste unitário sobre o texto do
script), a recusa da tela quando falta campo, e a recusa do validador quando o modelo não tem servidor. A realidade
desta máquina, **relida em 2026-10-15**: `docker` 29.3.1 **existe** mas a daemon do Docker Desktop **não está de pé**
(`failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine … O sistema não pode encontrar o
arquivo especificado`), e `pm2` **não está instalado** (`pm2: command not found`).
Ou seja: mesmo localmente não haveria como rodar um container para conferir. O registro de auditoria guarda as execuções
de teste (não há como apagar linhas de auditoria por desenho).

E o recorte honesto da **etapa 5**, do mesmo jeito medido:

- **`semgrep` não está no `PATH`** desta máquina (`semgrep: command not found`), mas **existe** fora dele, em
  `C:\Users\dougl\AppData\Local\Temp\semgrep-probe\Scripts\semgrep.exe` (versão **1.181.0**, lida pela sonda da tela). Não
  há correção de documento aqui porque nunca houve documento dizendo o contrário — o que existia era a minha frase em
  conversa, "o semgrep está ausente nesta máquina", e ela estava errada: quem não existe é o `PATH`, não a ferramenta. É
  exatamente o caso que `analise.semgrep.caminho` atende, e por isso a sonda tem `origem: "configuração"`.
- **A linha do motor que *trava a corrida pelo corte* está medida na janela** (era a lacuna aberta desta etapa; §2,
  linha "Publicação travada pelo corte da análise"). Na janela eu li o `blocked: true` da corrida #6 (8 erros com corte
  `ERROR`) e a corrida **falhando por ausência de leitura** (#44, com a corrida de análise do mesmo segundo em
  `status "falhou"`), e faltava o caso em que o **achado** barra a publicação: fechado com o `piloto-etapa5` cadastrado
  como modelo de deploy (perfil 22, grafo `varredura` → `depois-do-gate`), onde a execução **#47** e, no binário
  rebuildado, a **#48** terminaram `failed` com `varredura failed` pela contagem 9/4/1 contra o corte `ERROR`, o nó
  seguinte `skipped` e a pasta de destino **nunca criada**. O que segurou esse fechamento foi justamente medir: a
  primeira leitura (#11) mostrou a raiz da varredura escrita como `\\?\C:\…` na tela e no pacote, e isso virou causa
  corrigida no backend (§5, item 27) antes de a linha entrar aqui como medida. Criar arquivo varrerável dentro do
  piloto da etapa 2 seguiu bloqueado pelo classificador de permissões, e o bloqueio não foi contornado — por isso o
  teste usou o piloto que já existia.
- As seis palavras de classificação do pacote foram **lidas no JSON** para três delas — `barra` (#6), `cobertura-nula`
  (#5, com `bloqueia: true`, `decisoes` `revisar-o-pedido` + `instalar-ou-apontar-a-ferramenta` e a `nota`
  "o semgrep não estava disponível nesta máquina — instale-o ou aponte o caminho em `analise.semgrep.caminho` …") e
  `sem-achados` (#7 e #8, `bloqueia: false`, `decisoes` `prosseguir|nada no corte declarado, com leitura efetiva e sem
  erro de ferramenta`); `revalidar-varredura`, `achados-abaixo-do-corte` e `varredura-interrompida` são o que o código
  devolve e o que os testes unitários asserem, não algo que eu tenha visto na janela.

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

Três armadilhas de leitura que já me fizeram tirar conclusão errada da janela (todas custaram chamadas nesta sessão):

- O texto que você passa é **corpo de função** (`async () => { … }`), não expressão: sem `return` o CDP devolve
  `{"result":{"type":"undefined"}}`, que parece "a janela não respondeu" quando na verdade a sua expressão só não
  devolveu nada.
- As views escondidas **continuam montadas**: `document.querySelector("main")` e `body.innerText` misturam o Deploy com
  a aba Git. Filtro que funciona: `e.offsetParent !== null` por elemento, e ir à aba primeiro clicando no botão dela.
- Os rótulos no DOM estão **em minúsculo** (`capitalize` é CSS): procurar por `"Deploy"` não acha o botão `"deploy"`.
  Compare em caixa-insensível.

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
- **Executores de runtime e git como passo** (etapa 4 do plano CI/CD — `src-tauri/src/runtime.rs`, `src-tauri/src/deploy.rs`,
  `src-tauri/src/models.rs`, `src/components/deploy/{ProfileForm,PipelineGraph}.tsx`): o modelo de deploy passa a dizer
  **como** o app sobe no servidor, não só **o que** copiar. `runtime` vale `docker`, `compose` ou `pm2` (vazio é o
  comportamento antigo, arquivo no destino), e as etapas 8 (build), 11 (subir), 12 (verificar) e 13 (reverter) leem o
  executor do perfil em vez de presumir `systemctl`. **Continua sendo um motor só**: `runtime::Alvo` é derivado da mesma
  `DeployProfile` que a etapa 1 confere (`Alvo::da_definicao` → `Alvo::conferir`, a mesma cadeia do `pipeline_validate`),
  e o que muda é o lote POSIX enviado pela stdin da sessão. Nenhum código decide sucesso pelo código de saída: cada
  roteiro imprime o próprio veredito (`IMAGEM:`, `ANTERIOR:`, `CONTAINER:`, `RAZAO:`) e `runtime::estado_no_ar` lê essa
  saída — se o container não está `running` com a imagem esperada, a etapa falha com o motivo, e a reversão devolve a
  imagem anterior (`deploy_runs.previous_image`, coluna da migração 9, exibida no histórico como
  "imagem anterior …"). O **nó git** (`kind "git"`, migração 8 com `git_op`/`git_ref`) é o quarto tipo do grafo e roda
  **nesta máquina**, com as operações que o motor realmente tem (`pull`, `checkout`, `tag`); sem operação ele é recusado
  como "um desenho no quadro que nenhuma etapa executa", e a resposta que sai na linha do histórico é a do próprio git
  ("tag v0.0.1-janela criada nesta máquina — sem remoto cadastrado", "em main"), nunca um ok genérico. A pasta de
  trabalho local do nó git é conferida pela **mesma** função do nó de comando local (`conferir_pasta_local`), porque é o
  mesmo risco, e em tempo de corrida `dir_do_no` ainda canonicaliza e recusa o que dobra para fora da raiz do projeto.
  Medido na janela real (§2): as três telas de executor, as recusas de campo, o nó git rodando nas execuções **#42** e
  **#43**, e `git tag -l` no disco confirindo a etiqueta que a tela anunciou.
- **Análise de código na tela e no pipeline** (etapa 5 do plano CI/CD — `src-tauri/src/analysis.rs`,
  `src-tauri/regras/dd-seguranca.yaml`, `src-tauri/src/deploy.rs`, `src-tauri/src/pipeline.rs`, `src-tauri/src/models.rs`,
  `src/views/AnalysisView.tsx`, `src/lib/analysis.ts`, `src/components/deploy/PipelineGraph.tsx`): o app passa a ler o
  **código-fonte** de um projeto, não só o que ele publica. **Um analisador, duas portas** — a tela **Análise** e o nó
  `analise` do grafo chamam a mesma `analysis::executar`, e o veredito é sempre a contagem contra o corte escolhido
  (`barra_o_corte`), nunca o código de saída da ferramenta: `codigoDeSaidaEProva: false` está dentro do pacote por
  escrito. O instrumento **embutido** são 19 regras em YAML (`dd-101`…`dd-111`, Python/JS+TS/PHP/genéricas) lidas pelo
  próprio app, sem dependência nova; o instrumento **semgrep** adapta o executável do projeto
  [semgrep/semgrep](https://github.com/semgrep/semgrep) (`--config` com as regras embarcadas entregues, saída `--json`
  convertida em achados nossos), com o caminho configurável em `analise.semgrep.caminho`. A sonda da ferramenta tem
  **prazo próprio** e distingue `encontrado` / `ausente` / `sem-resposta`, porque "não respondeu" não é "não existe".
  Cobertura é gravada junto (`arquivosLidos`, `arquivosPulados`) e uma varredura que não leu nada, ou que perdeu o
  instrumento pedido, **falha** em vez de passar por limpa — `blocked` é o veredito, não um aviso. O trecho de cada
  achado é **relido do disco e mascarado** por `security::mascarar` antes de ser gravado: a mesma dona da máscara dos
  logs do motor e da checagem de commit, com a passada nova de `Bearer` que fecha o buraco medido nesta etapa (um token
  `sk_live_…` apareceu destampado na tela antes da correção). Para apontar uma IA sobre a varredura sem inventar
  contrato: **Exportar pacote** imprime `devops-desktop/analise@1` com `esquema`, `projeto`, `execucao`, `cobertura`,
  `veredito` (classificação em palavras de ação: `barra`, `cobertura-nula`, `revalidar-varredura`, `sem-achados`,
  `achados-abaixo-do-corte`, `varredura-interrompida`), `decisoes` (cada uma com `acao`/`motivo`/`evidencia` apontando
  campo do próprio pacote — quem decide é quem recebe), `porRegra`, `achados`, `truncado`, `nota` e `observacao`; e
  **Exportar SARIF** fala com o que já existe fora do app. No grafo, o quinto tipo de nó (`kind "analise"`, migração 10
  com `analyzer`/`severity_gate` nas colunas de `pipeline_nodes`) é validado pelo mesmo `checar_grafo`, desenhado na
  mesma tela, e executado pelo **mesmo** `deploy::start` — não há segundo motor nem segunda fila.
  Medido na janela real (§2): a sonda achando o semgrep **1.181.0** fora do `PATH` pelo caminho configurado, as 19 regras
  validadas pela própria ferramenta, a corrida **#6** com 12 achados `blocked: true` e o token plantado mascarado em todos
  os trechos, o pacote `devops-desktop/analise@1` lido campo a campo, e o nó `analise` executado pelo motor em **#44**
  (falha por ausência de leitura), **#45** (semgrep real) e **#46** (embutido) — e, no fechamento da pendência do gate,
  **#47/#48**, a primeira corrida em que o motor **barra a publicação pelo corte**: `varredura failed` citando a
  contagem 9/4/1 contra `ERROR`, `depois-do-gate skipped`, e a pasta de destino que nunca chegou a existir.

Total da superfície: **147 comandos Tauri registrados** (contados por `#[tauri::command]` nos 24 arquivos de
`src-tauri/src`), 24 arquivos Rust, 11 views, **44.458** linhas de TS/TSX/Rust (77 arquivos, `node_modules`, `target`,
`dist` e a pasta `Python/` à parte) — contadas por `find` com essas exclusões + `wc -l`, relidas nesta rodada: as
verificações gravadas no Histórico (§5, item 28) são `git diff --numstat` lido, não estimado — **171** linhas novas em
`analysis.rs`, **30** em `commands.rs` (4 velhas substituídas) e **1** em `ui.tsx`, ou **198** líquidas.

Persistência (SQLite local, migrações em `src-tauri/src/db.rs`) — as treze tabelas que existem hoje:
`settings`, `servers`, `projects`, `audit_log`, `routines`, `terminal_history`, `deploy_profiles`, `deploy_runs`,
`deploy_run_steps`, `pipeline_revisions`, `pipeline_nodes`, `analysis_runs`, `analysis_findings`. Não há tabela de série
histórica de métrica: o painel lê do servidor na hora
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
19. **Um executor de runtime não acredita no código de saída — ele lê o estado que o próprio roteiro imprime.**
    `docker compose up -d` sai 0 com um container em `restarting`, e `pm2 reload` sai 0 de um app que morreu ao subir. Por
    isso cada roteiro termina imprimando o veredito (`IMAGEM:`, `ANTERIOR:`, `CONTAINER:`, `RAZAO:`) e
    `runtime::estado_no_ar` é o que decide a etapa: nada de "código zero então sucesso". É também a razão de a reversão
    guardar `previous_image` — sem a imagem anterior registrada **na execução**, "reverter" teria que adivinhar qual tag
    estava no ar.
20. **No `docker compose`, o argumento vazio do serviço é "todos os serviços", e `''` é um nome de serviço que o docker
    recusa.** `servico_argumento()` devolve string vazia quando o modelo não declarou serviço, e o roteiro monta
    `up -d` / `ps -q` **sem** argumento; montar `up -d ''` era o comportamento anterior, visto no roteiro gerado e
    corrigido na causa. É o mesmo princípio de "detectar realidade, não listas fixas": a ausência tem de chegar ao docker
    como ausência.
21. **Todo texto que entra num roteiro POSIX remoto passa por `shell_quote`, inclusive o que parece seguro.** O filtro
    `docker ps --filter name='^{nome}$'` interpolava o nome do container direto; um nome com apóstrofo escapava das aspas
    e virava **comando no servidor**. Hoje o argumento inteiro é quotado (`--filter 'name=^d'\''app$'`), coberto por
    `o_nome_do_container_nao_escapa_das_aspas_do_filtro`. Concatenação com entrada solta não é aceitável em script que
    viaja por SSH.
22. **Executor de runtime é servidor, por decisão, não por acaso.** `runtime` preenchido num modelo sem `server_id` é
    recusado ("é um executor de runtime e roda no servidor; este modelo publica nesta máquina"), porque os roteiros são
    POSIX e porque o motor não roda metade de um roteiro. Consequência de tela: o seletor de executor **só aparece** quando
    há servidor cadastrado, e some quando o destino é "nesta máquina" (lido na janela, §2).
23. **O nó git é local e só oferece operação que o motor já executa.** `kind "git"` roda `pull`/`checkout`/`tag` **nesta
    máquina**, na pasta do projeto; a tela não mostra executor para ele porque não existe opção remota, e o validador
    recusa nó git sem operação ("sem operação" — seria um desenho que nenhuma etapa executa) e nó git dividindo ramal com
    irmão (`só um nó de comando pode dividir ramal`), porque a pasta de trabalho do git é uma só. A pasta de um nó git é
    conferida pela **mesma** função do nó de comando local (`conferir_pasta_local`) — um `../fora` escrito na tela é
    recusado antes de salvar, e `dir_do_no` ainda canonicaliza na corrida. A resposta da linha é a do git, lida na janela:
    "tag v0.0.1-janela criada nesta máquina — sem remoto cadastrado", em vez de um sucesso mudo.
24. **Um cartão de progresso se reconcilia com o estado do motor, não só com os eventos dele.** A tela de Deploy só
    fechava o cartão pelo evento `deploy-done`; quando a corrida terminava **antes** de a leitura do detalhe chegar — ou
    quando a janela estava em outra tela — o cartão ficava preso em "em andamento" para sempre, com o **Repetir**
    inacessível (defeito #68, visto no print da janela com a corrida **#45** já `success` no banco). O motor é o
    dono do estado, então a tela agora pergunta a ele: enquanto `running`, uma sonda a cada 2 s chama `deploy_active` e,
    se a corrida não está mais em voo, relê `deploy_run` e converge o cartão (`src/views/DeployView.tsx`, efeito com
    bandeira `vivo` para não escrever estado depois do desmonte). Lido na janela com o **#46**, que começou e terminou no
    mesmo segundo: o cartão mostrou o status real e o **Repetir**. O princípio vale para qualquer tela que desenha
    trabalho longo: **evento é aceleração, estado é a verdade**.
25. **A saída de uma compilação se lê no arquivo de log, não no código de saída de um pipe.** `npm run tauri build` foi
    lançado como `… | tail -25` enquanto o executável antigo ainda estava aberto: o `tail` escondeu o
    `failed to remove file …devops-desktop.exe — Acesso negado (os error 5)`, o comando pareceu terminar bem, e eu
    quase documentei como novo um binário que não tinha sido gravado. Só a comparação entre o hash servido pela janela
    (`script[src]`) e o `dist/assets/` do disco descobriu o engano. Hoje: a compilação escreve log, o log é lido por
    inteiro, e o hash dentro do `.exe` é conferido contra o disco antes de qualquer afirmação no documento.
26. **Fixture de credencial tem de ser visivelmente falso.** O push desta etapa foi **recusado** pelo protetor de
    segredos do GitHub (`! [remote rejected] main -> main (push declined due to repository rule violations)`) porque um
    teste de mascaráção usava `sk_live_` seguido de 24 caracteres alfanuméricos de alto calibre: para o scanner do remoto
    não importa a intenção — a *forma* é a de uma credencial real. A mecânica da máscara depende do **formato em volta**
    do valor (`Bearer `, `chave = "…"`), nunca da entropia dele, então o fixture passou a
    `sk_live_NAO-E-UMA-CHAVE-REAL-2026` e o token opaco a `um-token-opaco-sem-prefixo-de-especie`; os **300** testes
    continuam verdes, conferidos **depois** da troca. Como o literal estava no diff do commit, corrigir em commit novo não
    bastava: os commits locais foram reunidos em um só (`git reset --soft`, nenhum trabalho perdido) antes de empurrar.
27. **A contenção mede no caminho canônico; a tela mostra o caminho que outro programa abre.** Na primeira medição do
    gate (§2) a linha do motor saiu dizendo `a varredura de \\?\C:\Users\dougl\…\piloto-etapa5 barra a publicação`, e o
    campo `root` do pacote carregava o mesmo prefixo. Não é enfeite de exibição: `\\?\` é o formato *verbatim* que o
    `canonicalize()` do Windows devolve, e nenhum outro programa, editor ou terminal o reconhece como caminho. A régua
    de segurança, porém, **precisa** continuar no canônico — é por `alvo.starts_with(&raiz_real)` que a varredura não
    sai da pasta cadastrada. Corrigido na causa, nos dois sentidos: `analysis::executar` confere a contenção com os
    caminhos canônicos e só então passa o alvo por `git_probe::simple`, a mesma função que já desfaz `\\?\` e
    `\\?\UNC\` para os nós de comando; o teste `a_raiz_gravada_e_a_mesma_rgua_da_contencao_sem_o_prefixo_verbatim` trava
    as duas metades (sem prefixo **e** igual à raiz usada na contenção), e a corrida **#12**, no binário rebuildado, sai
    com `C:\Users\dougl\AppData\Local\Temp\piloto-etapa5`. Os **301** testes foram relidos depois da troca.
28. **Verificação sem rastro não é verificação — e quem decide o nível é o módulo, não a tela.** `analysis_detectar` e
    `analysis_conferir` respondiam à pergunta e deitavam a resposta fora: nada em `audit_log`, nada no Histórico. Ele
    apontou exatamente isso depois de escrever um caminho errado em Configurações. Agora os dois comandos gravam, com a
    linha construída em `analysis.rs` (`LinhaVerificacao`, `linha_da_sonda`, `linha_da_conferencia`) porque é ali que cada
    estado tem significado: `encontrado`/`validadas` = `info`, `ausente`/`sem-ferramenta` = `warning`,
    `sem-resposta`/`inválidas` = `error` — ausência de ferramenta externa nunca é verde, e o comando que falha vira
    `error` com `análise · falhou`. Para o caso `ausente` a sonda passou a carregar `apontado`, o valor cru que alguém
    escreveu em Configurações antes de qualquer resolução: sem ele a linha não nomearia o que foi tentado, já que aí
    `caminho` é nulo e `origem` é vazia. E sem binário resolvido a linha **não fala de origem** — diz "nenhum binário
    resolvido a partir dali", porque "origem lugar nenhum" ao lado de um valor apontado se contradiz.
29. **Registrar é uma metade; encontrar é a outra.** A categoria `análise` já existia no banco desde a varredura, mas o
    `select` do Histórico não a conhecia: a linha aparecia em "Todas" com o texto cru e não dava para filtrar por ela.
    `CATEGORY_LABELS` recebeu a chave `análise` — medida na tela: filtro *Análise* devolve **13 linhas**, todas com o
    rótulo `Análise`, e as quatro primeiras são as quatro verificações desta rodada.

## 6. Pendências assumidas (com o primeiro passo de cada)

| # | Pendência | Primeiro passo concreto |
| --- | --- | --- |
| 0 | **Dados do piloto das etapas 2, 3, 4 e 5, ainda cadastrados de propósito** | O projeto **Piloto Etapa 2** (`%TEMP%\piloto-etapa2`) com a definição *Piloto sem servidor* **v18** e as execuções **#23–#46** continuam no cadastro para ele abrir a janela e ver as telas conferidas — a v18 é o grafo de oito nós lido hoje (`validar, git, coleta, preparo, publicar, gate, copia, historico` · `motor 6 · comando 1 · aprovação 1 · git 0 · análise 0`, com `preparo` de volta a `echo preparo`), a #34 é a corrida em que `preparo` falha e trava o resto, a #38/#39 são as duas execuções de nó sozinho lidas no binário final (`preparo` → "1 comando(s) nesta máquina", `gate` → "aprovado na janela"), a **#42/#43** são as duas execuções do nó git (uma `tag` que criou `v0.0.1-janela` de verdade no piloto e uma `checkout main` que respondeu "em main") e a **#44/#45/#46** são as três execuções do nó `analise` pelo mesmo motor (falha por ferramenta ausente, sucesso com o semgrep 1.181.0, sucesso com o embutido). O projeto **Piloto Etapa 5** (`%TEMP%\piloto-etapa5`, com `app.py`, `src/legado.php` e `src/servidor.js` — três arquivos,
que é o `filesScanned` lido das corridas dele) é o que tem código varrerável, e desde a medição do gate ele **tem**
modelo de deploy: perfil 22 *Gate de análise*, destino `%TEMP%\piloto-etapa5-publicado` (que ficou inexistente de
propósito — a publicação foi barrada antes de copiar), definição **v2** com o grafo de dois nós `varredura` →
`depois-do-gate`, e as execuções **#47** e **#48**, as duas `failed` pelo corte `ERROR`. As corridas de análise do
cadastro hoje vão até a **#12**: no projeto 4 existem **#2, #3, #4, #6, #11 e #12**, e a **#6** é a das 12 linhas
achadas com o semgrep real e o token mascarado, enquanto **#11/#12** são as duas do nó `analise` que barrou a
publicação (9 erros, 4 avisos, 1 info). O que a etapa 4 criou para medir **foi desfeito e conferido**: o servidor temporário foi apagado (`list_servers` → 0), o modelo *medição docker* 21 foi removido pela confirmação da tela, a etiqueta `v0.0.1-janela` saiu do repositório do piloto (`git tag -l` → 0) e o grafo voltou aos oito nós. Quando ele decidir, o primeiro passo é **Projetos → Piloto Etapa 2 → Remover do cadastro** (e o mesmo para o Piloto Etapa 5; a confirmação avisa quantos modelos saem junto; nenhum arquivo local é apagado) e depois apagar as duas pastas em `%TEMP%`. O `analise.semgrep.caminho` salvo hoje aponta para `%TEMP%\semgrep-probe\Scripts\semgrep.exe` — se a pasta cair, a sonda passa a dizer `ausente` e o instrumento embutido continua funcionando. O arquivo de teste de 1,5 GB já foi removido do projeto e do destino |
| 1 | **Client ID na máquina do Douglas** | Colar o **Client ID** da OAuth App (o campo está em **GitHub → Configuração**) e gerar um secret novo no GitHub — o valor hexadecimal que estava salvo no banco era o **secret**, não o ID. Sem isso a aba GitHub não conecta: é limite de conta, não do código |
| 2 | **Passada de visual (prioridade quando ele chamar)** | Só depois das funções; começar pelo print da tela que incomoda. Não refazer estilo por conta própria junto de feature |
| 3 | **Pacotes Linux** | Os alvos `deb`/`rpm`/`AppImage` nunca foram gerados aqui. Rodar `npm run tauri build` numa imagem Linux e ajustar `bundle.targets` + paths (o código já usa `dirs`/caminhos relativos, mas isso precisa ser provado, não suposto) |
| 4 | **Teste de interface automatizado** | Não há runner com asserções. O que existe hoje é o piloto `scripts/ponte-cdp.mjs` (§4.1), que dirige a janela real mas exige leitura humana a cada passo. Se um runner entrar, o mínimo é o fluxo do SPEC §17; a cobertura automática continua `tsc` + suíte Rust + `scripts/confira-repo.ts` |
| 5 | **Aceite visual das últimas entregas** | `Git → Árvore`, `GitHub → repositório → Local`, `Git → Guardados`, os botões *Reproduzir este commit* / *Reverter este commit* / *Voltar para este commit*, o **fluxo de deploy sem servidor** (modelo com destino "Nesta máquina", pré-vistoria, publicar, reverter), o **contrato de pipeline** — em **Projetos → Modelos de deploy** o badge `v{n}`, o botão **Conferir** e a lista de **Versões salvas**; em **Deploy** a linha de procedência da execução (`definição v{n} · ambiente · destino · commit`), o `· v{n} · {sha8}` no histórico e o **Repetir** com o painel âmbar quando a política recusa — o **construtor visual** (aba **grafo**: nós arrastáveis, painel do nó, **Conferir**/**Salvar grafo**/**Esvaziar**, **Rodar sozinho** / **Com dependências**, estado vivo por nó durante a corrida e o modal de **Aprovação** com Aprovar/Rejeitar), e a **etapa 4**: o campo **Executor de runtime** do modelo com as três telas (Docker com *Nome da imagem* / *Dockerfile* / *Nome do container* / *Portas publicadas*, Compose com *Arquivo do compose* / *Serviço*, PM2 com *App no PM2*), o nó **git** do grafo (*operação*, referência e **Pasta no projeto**), a linha **"imagem anterior …"** no histórico de uma reversão de runtime, e o botão **Publicar** parado com o motivo no rótulo de quem passa por cima — e a **etapa 5**: a tela **Análise** com o
projeto *Piloto Etapa 5* e a corrida **#6** na lista (12 achados, corte `ERROR`, selo de *barra a publicação*), o seletor
de instrumento com **Embutido / Semgrep / Ambos**, **Sondar ferramenta** devolvendo `encontrado · 1.181.0`, **Conferir
regras** e **Ver/Ocultar regras embarcadas (19)**, o campo **Caminho do semgrep** já preenchido com o caminho desta
máquina, **Exportar pacote** / **Exportar SARIF**, e no grafo do Deploy o nó **análise de código** com o seletor de
instrumento e o corte de severidade no painel do nó. **Acrescentado nesta rodada:** as duas verificações dessa tela agora
têm linha no **Histórico** — clicar **Sondar ferramenta** e **Conferir regras** e ver `Sonda do semgrep · …` e
`Conferência das regras · …` com o filtro *Análise* ligado, inclusive a linha que cita o caminho apontado quando ele não
é o executável. E o **fechamento do gate**, que é a tela mais fácil de julgar à
força bruta: **Deploy → Piloto Etapa 5 → modelo "Gate de análise" → Publicar** devolve o cartão em *falhou* com a linha
`varredura` citando **9 erros contra o corte `ERROR`** e `depois-do-gate` em *pulada*, e o destino combinado
(`%TEMP%\piloto-etapa5-publicado`) **não existe no disco** — nada foi copiado. Eu abri cada uma dessas telas e li o texto na janela real, mas **ele**
ainda não viu nada disso nesta máquina — e o tooltip do botão desabilitado é justamente o tipo de coisa que só o olho
confere |
| 7 | **Ver o motor *bloquear* uma publicação pelo corte da análise, na janela — FECHADA em 2026-10-15** | Estava listada porque a regra só tinha prova de teste unitário. Foi medida na janela real: perfil 22 *Gate de análise* no `piloto-etapa5`, grafo `varredura` (nó `analise`, corte `ERROR`) → `depois-do-gate`, execuções **#47** e **#48** `failed` com a contagem 9/4/1 citada na linha do nó, o nó seguinte `skipped` e a pasta de destino **nunca criada** — tudo lido em `deploy_run`/`analysis_corridas`/`analysis_pacote`, linha por linha no §2. O que ela revelou de quebra (o prefixo `\\?\` na mensagem e no pacote) virou o §5, item 27, com teste novo |
| 6 | **Revalidar o deploy por SSH num host real — e pela primeira vez com um executor de runtime** | Além de `build_step`/`identify_step`, a etapa 2 mexeu no caminho com servidor: `validate_remoto` agora chama `deploy::checar_definicao(..., Some(&servidor))` e `insert_run` passou a receber a procedência. As checagens remotas foram movidas verbatim e `server_ref()` continua sendo a primeira delas, então por construção o remoto não muda — mas "por construção" sem medir num host é presunção. **A etapa 4 amplia essa pendência em vez de criar outra:** Docker, Compose e PM2 só rodo no servidor, e o que foi conferido aqui é o roteiro gerado e a recusa da tela, nunca um container subindo de verdade. Primeiro passo: cadastrar um servidor, repetir C/D com `server_id` preenchido conferindo que a etapa 1 recusa o que recusava antes, e então publicar **uma** aplicação real com `docker` (o caminho completo: build da imagem no servidor, container novo no ar, `previous_image` gravada, reversão voltando a imagem anterior) e, se houver, uma com `pm2`. Nesta máquina isso não tem como: o `docker` 29.3.1 existe mas a daemon do Docker Desktop está parada, e `pm2` não está instalado |

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
