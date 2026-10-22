# Registro de entregas — onde está cada coisa, o que falta e como retomar

Documento de retomada. Ele não substitui o [README](../README.md) (uso do aplicativo) nem o
[SPEC.md](SPEC.md) (o que foi pedido); ele registra **o que já está pronto, onde está o código, como se prova
cada parte e o que ainda não existe**. A base destas linhas é o modo sem servidor (`66da3a7`), a **etapa 2 do plano
CI/CD** — o contrato central de pipeline (`4f005ee`) e a licença de contribuição (`df51de1`) —, a **etapa 3**, o
construtor visual do grafo (`23beec2`), a **etapa 4**, os executores de runtime (Docker, Docker Compose, PM2) com o
git como passo do pipeline, e a **etapa 5**, a análise de código-fonte — na tela e como nó do mesmo grafo —, registrada
aqui em 2026-10-15. Fecha a lista a rodada de **2026-10-15 à noite**: as duas verificações da tela Análise passaram a
deixar linha no Histórico (`a63bf00`, com o texto lido e medido na janela), §1.1 é o estado em um parágrafo. A
**etapa 6** do plano CI/CD: cada linha de saída de etapa **sobrevive no banco**, por tentativa, já
mascarada, e é relida depois de fechar e reabrir o aplicativo — medida nas corridas **#49 a #53**, no §2. Fecha a lista a
rodada de **2026-10-21**: primeiro as três leituras de Git que rodavam sem prazo (`6b7459e`), depois as **seis** leituras
de caminho que sobraram fora da aba Git e a sonda do `ssh-add` da descoberta de chaves — todas agora com prazo, com os
três estados, e com a contenção do destino finalmente valendo para a pasta que **ainda não existe**. Medido na janela real
no §2; as duas pendências que a rodada anterior deixou escritas (§6, linhas 9 e 10) estão fechadas ali com a contagem
corrigida. Fecha a lista a rodada seguinte, pedida pela queixa **"a análise de código não salva nada na pasta"**: as duas
exportações da tela Análise escreviam sozinhas dentro dos dados do aplicativo e a tela só mostrava o caminho em texto
miúdo; agora o aplicativo **pergunta onde** e um caminho que não serve não escreve arquivo nenhum (§5, item 36).
Fecha a lista a rodada seguinte, aberta pelo pedido **"revisar e validar a própria validação"**: a máscara de segredo
deixou de poder escapar de uma linha e o veredito da análise deixou de poder inventar leitura — os dois defeitos
reproduzidos antes de corrigir, com o texto cru da reprodução no §5, itens **38** e **39**. Nada desta rodada foi visto
na janela; o que lá está é medido por teste, por sonda de linha de comando e pelo banco, e o que não foi medido continua
dito como não medido.

## 1. Retomar em três passos

```bash
node ./node_modules/typescript/bin/tsc --noEmit     # tipos do frontend
npm run build                                        # tsc + bundle
(cd src-tauri && cargo test)                         # 336 testes de backend
(cd src-tauri && cargo clippy --all-targets -- -D warnings)
node scripts/confira-repo.ts                         # o comparador remoto↔pasta, 9 formas reais de URL
node scripts/sonda-mascara.mjs                       # 24 linhas da máscara, reproduzidas fora do app (21×3 + 3 da cauda)

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
- **`cargo build --release` sozinho não produz o aplicativo que se abre.** Compilado fora do `tauri` CLI, o `exe` serve a
  `devUrl` do `tauri.conf.json` em vez do `dist` embutido: a janela abre e o CDP devolve
  `chrome-error://chromewebdata/` com `ERR_CONNECTION_REFUSED` apontando para `http://localhost:1420`, e quem mede sem
  olhar a URL conclui que "a leitura do Git travou" quando na verdade não há aplicativo nenhum ali. O instalador é
  **`npm run tauri build`**, e a primeira linha da medição tem de ser `location.href` — que no binário certo devolve
  `http://tauri.localhost/`. Lido assim nesta rodada, às custas de um `exe` de 19:33 que não servia para nada.
- **Editar fonte depois de iniciar `npm run tauri build` deixa o `dist/` dentro do instalador defasado.** O bundle lê
  `dist` no fim; se mexeu em `src/` durante a compilação, refaça `npm run build` e compile de novo.
- **`TaskStop` não mata os filhos** (`cargo`, `rustc`, `node`): confira `Get-Process` antes de assumir que a compilação
  anterior acabou.
- `cargo fmt` **não** é usado neste projeto — o formato é o do próprio autor, e `cargo fmt` reorganiza tudo.

### 1.1 Estado em um parágrafo (2026-10-21, 22:17; a posição do Git relida às 22:51, depois do push; a superfície e o Git relidos de novo às 00:0x de 2026-10-22, depois da rodada de revisão)

Tela e backend: **148** `#[tauri::command]`, 24 arquivos Rust, 11 telas, **336** testes de backend (relidos às 00:0x:
`test result: ok. 336 passed; 0 failed; 0 ignored; 0 measured`), SQLite em
`user_version` **11** com **14** tabelas (a mais nova é `deploy_step_logs`), **46.681** linhas TS/TSX/Rust em **76**
arquivos (`find src src-tauri/src scripts` com `node_modules`, `dist`, `src-tauri/target` e `Python/` de fora, lido às
00:0x; o par 46.051/78 da medição anterior diferia pelo escopo do `find`, não pelo código), sem contar `node_modules`,
`target`, `dist` nem `Python/`, que é um venv **não rastreado** — nunca `git add`
nele. **A posição do Git, relida às 00:0x:** `git rev-parse HEAD` →
`c2c1877bc98dc548becad2eafa352e77637970da` (a correção da máscara e do veredito, **local**), `git ls-remote origin
refs/heads/main` → `ed69148ca102b959aae670d6b680de39adf950e1` (o `docs(entregas)` do push dos cinco), `git status -sb` →
`## main...origin/main [ahead 1]` — **um** commit local esperando a própria autorização na frase, como em toda rodada,
nunca por extensão de um "sim" anterior. O registro da rodada do push continua valendo: os **cinco** hashes que a frase
dele nomeou (`1bb0590`, `6b7459e`, `2738e41`, `e0c2753`, `32008f5`) estão no remoto, empurrados com **refspec explícito**
(`git push origin 32008f5:refs/heads/main`) e não `origin main`, porque o commit que grava a correção da tela (`26e6ea1`)
era um **sexto** fora da lista — empurrar `main` teria levado algo que ele não nomeou. O binário mais novo é
`src-tauri/target/release/devops-desktop.exe` das **22:12:08**, **19.576.320 bytes**, com `Finished 2 bundles at:` lido no
log da compilação (código de saída 0) e as duas embalagens conferidas no arquivo (`…_x64-setup.exe` **5.169.366 B** =
4,93 MiB às 22:11:57, `…_en-US.msi` **8.630.272 B** = 8,23 MiB às 22:12:02); ele era posterior às edições da rodada das
exportações e serve `http://tauri.localhost/` com `/assets/index-BLZoTx6l.js` — **885.308 B**, das 22:06:16, hash **novo**,
porque naquela rodada a tela Análise mudou. **Na desta revisão ele deixou de ser o retrato do fonte:** as edições que
fecharam em `c2c1877` são das 23:4x–23:5x, posteriores ao binário das 22:12, que não foi recompilado — nada desta rodada
foi visto na janela, e é por isso que o §2 e os itens 38 e 39 do §5 dizem o que foi medido por teste, por sonda e no
banco, e não o que foi conferido em tela. No banco real, **relido pela janela desta vez**: **3** projetos (2 =
`E:\ssh-master` em `main`, 3 = piloto com Git, 4 = `piloto-etapa5` **sem** Git), **32** corridas de deploy — exatamente
**#23 a #54**, e as duas abertas pelas sondas desta rodada (**#55** e **#57**) saíram junto com os modelos temporários no
`ON DELETE CASCADE`, que é por isso que a lista voltou ao tamanho de antes — e as corridas de análise do projeto 4 são
agora **9** (`#2, #3, #4, #6, #11, #12, #13, #16, #19`), a mais nova sendo a **#19**: `concluida`, raiz
`C:\Users\dougl\AppData\Local\Temp\piloto-etapa5\src` **sem** prefixo `\\?\`, 2 arquivos, contagem 5 erros / 3 avisos /
0 info.

A rodada que fecha aqui é a correção que a queixa dele pediu: **"o verificar do Git para ver o histórico fica
verificando" num HD/SSD externo — gira para sempre, sem erro.** Três leituras rodavam sem prazo nenhum: o
`git show <rev>:<caminho>` de cada blob (`.output()`, chamado uma vez por arquivo preparado na checagem de segredos), a
sondagem da pasta (perguntava `.git` no disco e rodava `git` sem prazo) e a resolução do caminho, que fazia
`metadata()` **e** `canonicalize()` — as duas chamadas que esperam o volume **dentro do kernel** — na thread do runtime
assíncrono. Hoje: todo `git` sai de uma porta só (`git::abrir`) e é colhido com prazo (`sshexec::collect_bytes`);
`sonde`, `blob_at`, `index_blob` e `is_repo` têm **três estados** — respondeu um valor, respondeu que não tem, **não
respondeu** — e o `Err` é propagado, nunca convertido em "sem branch", "não é repositório" ou varredura de segredos limpa;
a espera pelo disco (`git_probe::com_prazo`, 20 s) roda em thread auxiliar e é abandonada com o motivo; normaliza-se com
`std::path::absolute`, que é léxica, em vez de `canonicalize()`; e o trabalho de disco saiu da thread assíncrona
(`git_read`/`git_write`, `scan_project_secrets`, `git_clone`, `analysis_executar`, `open_in_editor` resolvem a pasta
**dentro** do `spawn_blocking`). Detalhe que era o defeito escondido: a colheita dormia um passo fixo de 200 ms, e um
prazo de 1 ms acordava depois dele e colhia o processo como se tivesse respondido a tempo — agora dorme **só o que
falta** para o prazo. §5, item 34; texto e medições no §2.

A rodada seguinte fecha as duas pendências que aquele parágrafo deixou escritas (§6, linhas 9 e 10), e a primeira coisa
que ela corrigiu foi a própria linha 9: não eram **três** `canonicalize()` nus em **dois** lugares — eram **seis**
chamadas em **três** funções: `analysis::executar` (a raiz da varredura **e** o alvo), `deploy::dir_do_no` → `dir_local`
(a pasta de trabalho do nó; o nome `publicar_local` que a linha citava é outra função) e `deploy::checar_definicao` (a
origem **e** o destino da contenção, as duas que a linha nem contava). Cada uma passa agora pela mesma porta,
`git_probe::canon` (`git_probe.rs:96`), que devolve **três estados** — `Resolveu` já sem o prefixo `\\?\`, `Ausente` com o
texto do sistema operacional, `SemResposta` com o motivo e a duração — e, onde o caminho ainda não foi criado por
ninguém, por `canon_ou_ancestral` (`git_probe.rs:112`), que resolve até a pasta mais funda que o disco consegue responder
e acrescenta o resto. Foi aí que a medição achou um defeito que não tinha nada a ver com disco externo: a guarda
`alvo.starts_with(&origem)` de `checar_definicao` comparava uma origem **canônica** com um destino **digitado**, e como o
destino da primeira publicação não existe, o `canonicalize()` dele falhava e a comparação não era feita — a proteção
"não publique dentro do próprio projeto" valia exatamente para o caso em que ela não precisava valer. O outro prazo é o
da sonda do agente: `ssh_config::read_agent` saiu do `.output()` sem fim para `sshexec::collect` com **5 s** (não os 2 s
que a linha 10 propunha — 5 s é o prazo que a sonda de ferramentas já usa, e é o mesmo número nos dois lugares), e os
quatro vereditos agora se distinguem: não está no PATH, **não respondeu**, vivo com chaves, vivo sem chaves. §5, item 35;
texto e medições no §2.

A rodada seguinte fecha a queixa **"a análise de código não salva nada na pasta, aliás tudo que tem a ver com isso não
salva"**. Lido antes de mexer: os dois arquivos da corrida 18 **existiam** —
`%APPDATA%\com.douglasgdev.devopsdesktop\analises\corrida-18.json` (123.822 B, 20:53) e `corrida-18.sarif` (789.728 B,
20:48) —, mas escritos por `analysis::exportar`, que criava a pasta dentro dos dados do aplicativo e escolhia o nome
sozinha, com o caminho exibido em texto miúdo na tela. Visto de fora, nada ia para a pasta em que ele olha. Não havia
**nenhum** ponto no aplicativo em que se pudesse escolher onde gravar: `save` do `@tauri-apps/plugin-dialog` não era
importado em tela alguma (só `open`, para escolher pasta de entrada), e não havia download por `Blob` em lugar nenhum
(lido com a varredura dos `src/`). Hoje `analysis_exportar` recebe o caminho escolhido, e `analysis::exportar` vale o que
vale: caminho relativo é recusado, extensão que não é a do formato é recusada, a pasta que não existe devolve **o motivo
do sistema operacional**, e nos três casos nenhum arquivo é escrito; o `create_dir_all` silencioso na pasta de dados saiu
do caminho. A tela Análise pergunta antes de invocar o comando, com nome sugerido (`analise-corrida-{id}.json` /
`.sarif`) e filtro por formato; cancelar não escreve nada e não deixa rótulo velho na tela. §5, item 36; texto e medições
no §2.

O que não pode ser revertido ao mexer aqui (lista completa no §5): **não existe segundo motor de deploy** — a tela e o
nó do grafo chamam a mesma `analysis::executar` / `deploy::start`, e o veredito é contagem contra `severity_gate`,
**nunca** o código de saída da ferramenta; o que é desenhado na tela é o que o backend executa; a fila linear de 15
etapas continua compatível com o grafo; o frontend não foi substituído; UI tem **controles e estado**, sem prosa
explicativa (caminhos e passo a passo vivem neste documento e no README); segredos são mascarados em toda saída e
**valores de variáveis nunca vão para disco, log ou Histórico** — retratos de versão carregam só nomes; **a gravação do
log vem depois da máscara, nunca antes** (§5, item 31); cada tentativa é um bloco separado, e nada é apagado quando o
nó repete (§5, item 30); o corte é declarado, não escondido (§5, item 32); sonda nomeia "sem resposta" separadamente de
"não encontrado" e tem prazo próprio, e **nenhuma leitura de Git ou de caminho roda sem prazo** — o "não respondeu" é
propagado como erro, jamais lido como "não existe", "sem branch" ou varredura limpa (§5, item 34); a contenção de caminho
compara os **dois** lados na forma que o disco respondeu — nunca uma origem canônica contra um destino digitado, porque o
destino da primeira publicação ainda não existe e a guarda passaria por cima dele (§5, item 35); **exportar é escrever no
arquivo que a pessoa escolheu** — o aplicativo não escolhe pasta por ela, e um caminho que não serve devolve o motivo e
não escreve nada (§5, item 36); nada é simulado, nenhum botão é enfeite; uma mecânica por módulo — e uma mecânica
servida por **duas portas** (a tela e o nó do grafo), não por duas implementações; ferramentas funcionam **sem servidor
e sem conta de serviço**; `working_dir` local fica contido na raiz do projeto; aprovação manual nunca é presumida; senha
SSH via `SSH_ASKPASS` por operação, jamais persistida; aviso de compilador é corrigido na causa, sem `#[allow(...)]`.

Como se trabalha neste repositório: o loop da etapa é `cargo test --lib` → `cargo clippy --all-targets` (zero avisos) →
`tsc --noEmit` → `npm run build` → `npm run tauri build`, lendo **cada** log antes de dizer que passou — nenhuma
varredura é chamada de verde por presunção. Fechar um `git commit` não autoriza o próximo `git push`: **push é ação
compartilhada e a pergunta é refeita a cada commit**, porque um "sim" anterior não se estende. Dois fatos mecânicos desta
máquina: `git commit && git push` na mesma linha é bloqueado (são chamadas separadas), e um diff com literal de
**forma** de credencial faz o GitHub recusar o push inteiro — corrige-se o fixture, não o scanner, e os commits locais
reunem-se com `git reset --soft`. A janela é pilotada por `scripts/ponte-cdp.mjs` (§4.1); `npm run tauri dev` não roda
(porta 1420 presa por um Vite de sessão anterior).

O que falta, no curto prazo: **repetir esta medição no disco externo de verdade** — nesta máquina não há volume USB
(`Get-PhysicalDisk` devolve três SSDs internos: `KINGSTON SNV3S1000G` de 1 TB, `SA400S37240G` de 240 GB e
`SA400S37120G` de 120 GB), então o que se mediu na janela foi "toda leitura responde, e responde com motivo"; a frase do
abandono (`não respondeu em 20 s`) só existe provada por teste de unidade
(`uma_sondagem_lenta_e_abandonada_com_o_motivo`) e precisa ser lida na tela dele, no HD que ele usa para testar (§6,
linha 8). As duas linhas que a rodada anterior deixou escritas como "fora do escopo pedido" — os `canonicalize()` nus de
`analysis.rs`/`deploy.rs` (§6, linha 9) e o `ssh-add -l` sem prazo da descoberta de chaves (§6, linha 10) — **foram
corrigidas e medidas nesta rodada**, e no lugar delas entraram três achados novos, lidos ao varrer o resto do código
(§6, linhas 11 a 13): **34** chamadas `.is_dir()`/`.exists()` ainda sem prazo em 8 arquivos, das quais duas importam
(`pipeline.rs:445`, o `is_dir()` da raiz dentro de `pipeline_validate`, antes das leituras com prazo; `analysis.rs:944`, o
`read_dir` da varredura embutida, que um disco adormecido segura sem que `analysis_interromper` alcance), o
`pipeline_validate` rodando na thread assíncrona sem `spawn_blocking`, e o `percorrer` do analisador embutido sem prazo
nem verificação de cancelamento dentro do laço. A quarta suspeita da lista anterior — um `starts_with` sensível a
maiúsculas dobrando a contenção — foi **medida e não se confirma**: dois destinos digitados em maiúsculas foram recusados
pelas duas frases certas, porque `canon_ou_ancestral` devolve o pedaço que existe na grafia do disco. A queixa nova,
**"a análise de código não salva nada na pasta"**, foi corrigida e medida na janela (o pacote e o SARIF da corrida 18
escritos nos caminhos escolhidos em 7 ms, as três recusas lidas com motivo, a pasta de dados intocada) — o que falta dela
é o olho dele em **Análise → Exportar pacote**: o diálogo nativo `Exportar pacote` abriu, e isso foi lido pela janela do
processo, mas **o que o diálogo mostra por dentro ninguém mediu**, porque entrada sintética de sistema não é usada neste
projeto; fica para ele conferir o nome sugerido, o filtro e o cancelamento. Ficou também achado novo, lido no pacote que
ele exportou (§6, linha 14): a varredura do projeto `E:\ssh-master` leu **2.306** arquivos porque o `Python/` da raiz é
uma distribuição CPython portável (**2.330** arquivos, 136 MB, 1.166 `.py`) que não está no Git e não está na lista de
pastas puladas — as **cinco** corridas desse projeto (`#9, #10, #15, #17, #18`) fecham em `barra` com a mesma contagem
**676/42/640** por código que não é dele, inclusive a **#17**, rodada com `ambos`. Falta ainda o aceite
visual da etapa 6: abrir **Deploy → uma execução do
histórico → Saída da etapa** e julgar se os blocos por tentativa e a busca servem, além dos aceites anteriores em
**Análise → Sondar ferramenta / Conferir regras → Histórico com o filtro** *Análise*, a corrida **#6**, os cartões de
Deploy **#45/#46** e a corrida travada pelo gate **#47/#48**. As pendências do plano (triggers, ambientes com aprovação
registrada, retry configurável, revalidação SSH e os executores Docker/Compose/PM2 contra um daemon/host real) estão em
§6; as lacunas honestas do "canivete", em §7.

## 2. Estado verificado em 2026-10-10 … 15 (resultado lido, não presumido)

| Checagem | Resultado real |
| --- | --- |
| `tsc --noEmit` | saída vazia, código 0 — relido nesta rodada (etapa 6) com o componente novo na árvore, e outra vez dentro do `npm run build` que antecede o binário das 15:05 |
| `npm run build` | `✓ built in 5.27s`; `dist/assets/index-C287853D.js` **884.95 kB** (gzip 240.18 kB) e `index-DjAxcwFb.css` **62.10 kB**; o único aviso é o de chunk > 500 kB, de tamanho de bundle, não de código. Conferido **dentro da janela**: `document.querySelectorAll("script[src]")` devolve exatamente `["/assets/index-C287853D.js"]` — o frontend dentro do `.exe` das 15:05 é o do disco |
| `cargo test` (todos os alvos) | **`308 passed; 0 failed; 0 ignored; 0 measured; finished in 17,16 s`** em `devops_desktop_lib`, relido **duas vezes** nesta rodada: uma com a suíte em 307 e a leitura ainda com tope global (aí o teste novo **falhou** — `left: 1999, right: 2001`), e outra depois de corrigir o tope por bloco. Os cinco testes novos são `cada_tentativa_de_uma_etapa_e_um_bloco_que_nao_se_sobrescreve` e `o_valor_da_variavel_nao_chega_ao_log_gravado` (db.rs), `a_mascara_nao_achata_as_linhas_da_saida` (audit.rs), `o_teto_de_linhas_declara_o_corte_uma_só_vez` (deploy.rs) e `a_porta_de_leitura_do_log_ve_as_tentativas_em_ordem` (pipeline.rs), cada um lido pelo nome antes de entrar na contagem. Histórico: `303 passed` (rodada das verificações), `301 passed` (fechamento do gate), `300 passed` (fim da etapa 5), `261 passed` (etapa 4), `229 passed` (etapa 3) |
| `cargo clippy --all-targets` | `Finished \`dev\` profile … in 4.15s` com **zero linhas `warning`/`error`** sob `-D warnings`, que transforma aviso em falha. Foi assim que os dois achados desta etapa apareceram: o `explicit_auto_deref` do `*t` e, sobretudo, o teste que não passava com o tope global — nenhum `#[allow]` foi acrescentado para chegar ao verde |
| `node scripts/confira-repo.ts` | `todas as 9 formas conferem`, código 0 — relido nesta rodada antes do rebuild |
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
| Migração `user_version` **11** no banco real (etapa 6) | lida **nos dois sentidos**: o banco de `AppData/Roaming/com.douglasgdev.devopsdesktop/devops-desktop.db` abriu em `user_version 10` (lido antes do binário novo, com `deploy_step_logs` **inexistente** e a última corrida sendo a **#48**) e, aberto pelo binário das 15:05, foi relido em `user_version 11` com a tabela `deploy_step_logs` criada — 8 colunas (`run_id, position, attempt, seq, stream, ts, text, id`), `CHECK (stream IN ('stdout','stderr','system'))`, índice único `(run_id, position, attempt, seq)` e índice de leitura `(run_id, position, seq)`, com `ON DELETE CASCADE` para `deploy_runs`. Na janela, lida de novo no fim da rodada, a tabela tem **2.012 linhas** em **6** blocos: `(49,10,1,2) (50,10,1,2) (51,9,1,3) (51,9,2,2) (52,11,1,2001) (53,4,1,2)` — o bloco novo é o da **#53**, escrita depois do reopen. O `DELETE FROM deploy_runs` que apaga a corrida de teste leva o log junto no mesmo `id` (testes `cada_tentativa…` e `o_valor_da_variavel…` conferem a cascata e o índice único) |
| **O valor da variável não chega ao banco** (CDP + leitura crua do arquivo, etapa 6) | corrida **#49** (binário das 14:52, antes da correção): o nó de comando `echo envio %CHAVE_PILOTO%` deixou **`envio %CHAVE_PILOTO%` literal** nas duas linhas gravadas — o `Secrets` do `Ctx` era montado só com as chaves, sem os valores fornecidos, e um nó rodado sozinho nunca passava pela etapa que os injeta. Corrigido na causa (`Secrets::new(&profile.secret_keys, &secret_values)` na construção do `Ctx`), a corrida **#50** grava **`envio ***`** — lido pela porta `pipeline_step_logs`. E a prova negativa, feita no **arquivo**, não pela tela: `valor-de-teste-7h4d` (o valor que eu mesmo forneci) está **ausente** de `devops-desktop.db` (2.150.400 B), do `-wal` (4.136.512 B) e do `-shm` (32.768 B) — varridos byte a byte com o banco já migrado e escrito pelo binário novo. O nome da variável continua aparecendo (`%CHAVE_PILOTO%` é o comando, não o valor), e é assim que o `secret_keys` funciona: retrato carrega nomes, nunca valores |
| **Tentativa é bloco, não sobrescrita** (CDP, etapa 6) | nó `repete` com `retry_limit` 1 e `echo aqui & exit /b 3`: corrida **#51**, `failed`, saída 3, e **cinco linhas em dois blocos** lidas pela porta — `a1#0000 stdout: + echo aqui & exit /b 3`, `a1#0001 stdout: aqui`, `a1#0002 stderr: aqui — repetindo o nó (tentativa 1/1)`, `a2#0000 stdout: + echo aqui & exit /b 3`, `a2#0001 stdout: aqui`. O `seq` **recomeça em 0** na tentativa 2, o aviso de repetição fica no bloco da tentativa que falhou (porque `ctx.attempt` só sobe na primeira instrução do laço), e nada da tentativa 1 foi reescrito. Na tela, lido no DOM, a mesma divisão: **`tentativa 1 · 3 linha(s)`** e **`tentativa 2 · 2 linha(s)`**, com a linha de `stderr` no tom vermelho e a de `stdout` no tom neutro |
| **O teto declara o próprio corte** (CDP, etapa 6) | nó `muito` com `for /L %i in (1,1,2500) do @echo linha %i`: corrida **#52**, **2.001 linhas** no bloco da tentativa 1 — 2.000 de `stdout` e a última, `a1#2000 system: … truncado em 2000 linha(s) por tentativa — 501 linha(s) além do teto não foram gravadas`. Lida de duas formas: tope pedido 9.999 devolve as 2.001 e **termina na linha do corte** (é para isso que o teto de leitura é `TETO + 1`, não `TETO`), e a leitura padrão devolve **500** — um tope, não uma promessa. Na tela, o painel mostra `tentativa 1 · 500 linha(s)` (de `0000+ for /L …` até `0499linha 499`) e, digitando **`truncado`** no campo de busca, **uma** linha volta: exatamente a que declara o corte, `2000… truncado em 2000 linha(s) por tentativa — 501 linha(s)…` |
| **Fechar e reabrir é o teste** (CDP, pedido dele: *"corrida #49 no Piloto → fechar → reabrir → ler linhas do banco"*) | o aplicativo das 14:52 foi **fechado** (`taskkill /IM devops-desktop.exe`, "sinal de encerramento enviado ao processo … PID 11140") e o binário das 15:05 reaberto (`Id 29016`, `Responding True`, título `DevOps Desktop — Central de SSH, Git e Deploy`). Na janela nova, `pipeline_step_logs` devolve **#49 com as suas 2 linhas**, **#50 com `envio ***`**, **#51 com os 5/2 blocos** e **#52 com o corte** — a mesma linha que a janela mostrou na hora, relida do banco depois de o processo ter morrido e outro nascido. E a escrita continua depois do reopen: `preparo` rodado sozinho virou a corrida **#53** (`04 preparo done · 1 comando(s) nesta máquina`) e as duas linhas dela (`a1#0000 stdout: + echo preparo`, `a1#0001 stdout: preparo`) foram lidas pela mesma porta |
| **Corrida anterior à migração não inventa linha** (CDP, etapa 6) | a **#48** (15/10, antes da `user_version 11`) abre o cartão, os dois passos aparecem com **Saída da etapa**, e o painel diz **`nenhuma linha gravada nesta etapa`** com **0** linhas numeradas. Não há log retroativo fingido: o que não foi gravado na hora simplesmente não existe, e a tela diz isso em vez de imprimir vazio silencioso |
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
| **Git, rodada das portas com prazo (lido às 21:5x, antes do commit desta linha)** | `git log --oneline -4` → `2738e41` sobre `6b7459e` sobre `1bb0590` sobre `1985c39`; `git status -sb` → `## main...origin/main [ahead 3]`, o mesmo `ahead 3` que a janela leu em `git_status(2)`; `git ls-remote origin refs/heads/main` → `1985c391dd9b71a11b5f3a570abf0c0dfc7c3172`, lido **depois** dos três, conferindo o que a linha afirma. **Nada foi empurrado** — nem a etapa 6, nem as duas rodadas de prazo — e o commit que grava este texto fica local até a autorização ser pedida na frase, por commit, nunca por extensão de um "sim" anterior |
| Frontend embutido no executável | conferido **dentro da janela**: o pacote das 09:37 servia `index-5zynmBe_.js` enquanto `dist/` já tinha `index-Cko7uGOK.js` (frontend velho dentro de um binário novo). O binário das **14:55** serve `/assets/index-BC07Lf6F.js` lido de `document.querySelectorAll("script[src]")`, o mesmo hash de `dist/assets/` da rodada das 14:48 — incluindo a tela sem o parágrafo de ajuda. **Etapa 5, binário das 16:11:** `document.querySelectorAll("script[src]")` devolve exatamente `["/assets/index-l9oCfRyf.js"]`, e `dist/assets/` tem `index-l9oCfRyf.js` (882.386 B) — o frontend dentro do instalador é o que está no disco. **Rebuild das 20:42** (só backend mudou): `document.querySelectorAll("script[src]")` devolve de novo exatamente `["/assets/index-l9oCfRyf.js"]`, o mesmo hash de `dist/assets/` — nada da tela foi reescrito nesta rodada. **Etapa 6, binário das 15:05:** `["/assets/index-C287853D.js"]`, o hash que o `vite` gravou nesta rodada (884.95 kB, com o `StepLogs` dentro) — o `.exe` novo carrega a tela nova |
| Instalador | `npm run tauri build` (14:55, saída 0) → `DevOps Desktop_0.1.0_x64-setup.exe` 5.002.154 B (4,77 MiB) e `DevOps Desktop_0.1.0_x64_en-US.msi` 7.200.768 B (6,87 MiB). **Etapa 5 (rodada das 16:11):** as duas linhas lidas **no arquivo de log da compilação**, não no código de saída de um pipe — `bundle/nsis/…-setup.exe` **4.90 MiB** e `bundle/msi/…_en-US.msi` **8.18 MiB**, com `devops-desktop.exe` gravado no mesmo minuto (foi justamente ignorar o pipe que me fez publicar um binário velho nesta sessão; §5, item 25) |
| Instalador, rebuild das **20:42** (o da mediação do gate) | as duas linhas do log do `npm run tauri build`: `Finished 2 bundles at:` com `bundle/nsis/DevOps Desktop_0.1.0_x64-setup.exe` **4.90 MiB** (5.133.902 B contados no arquivo) e `bundle/msi/DevOps Desktop_0.1.0_x64_en-US.msi` **8.18 MiB** (8.572.928 B), `devops-desktop.exe` com mtime **20:42** — foi esse binário que abriu a janela em que a corrida **#48** foi lida |
| **Rebuild das 15:05 (etapa 6, o das medições acima)** | lido do arquivo de log da compilação, não do código de saída de um pipe: `Finished \`release\` profile [optimized] target(s) in 5m 27s`, `Built application at: …\target\release\devops-desktop.exe`, `Finished 2 bundles at:` com `bundle/nsis/DevOps Desktop_0.1.0_x64-setup.exe` **4.92 MiB** (5.159.455 B contados no arquivo, mtime 15:05) e `bundle/msi/DevOps Desktop_0.1.0_x64_en-US.msi` **8.20 MiB** (8.597.504 B, mtime 15:06); `devops-desktop.exe` **19.505.664 B** com mtime **15:05**. A precaução de fechar a janela antes do link funcionou: o `cargo` não sobrescreve um `.exe` em uso, e eu fechei o processo 11140 com `taskkill /IM` enquanto o build ainda compilava — o link escreveu 15:05 sem reclamar, e foi esse binário que abriu a janela PID 29016 em que #49 a #53 foram lidas |
| **Rebuild das 22:25** (o das verificações gravadas) | lido do arquivo de log da compilação, não do código de saída de um pipe: `Finished \`release\` profile [optimized] target(s) in 7m 11s`, `Built application at: …\target\release\devops-desktop.exe`, `Finished 2 bundles at:` com `bundle/nsis/DevOps Desktop_0.1.0_x64-setup.exe` **4.91 MiB** (5.144.116 B contados no arquivo) e `bundle/msi/DevOps Desktop_0.1.0_x64_en-US.msi` **8.18 MiB** (8.572.928 B); `devops-desktop.exe` com mtime **22:25** lido junto do `date`. A janela que estava aberta (das 22:14) foi **fechada antes do link**, porque o `cargo` não sobrescreve um `.exe` em uso — foi a precaução desta vez, não uma falha lida: o link escreveu 22:25 sem reclamar. Foi esse binário que abriu a janela em que as quatro linhas do §2 (ids 250/251/253/254) foram lidas |
| **Cartão de progresso reconciliado** (CDP, etapa 5) | o **#46** começou e terminou no mesmo segundo (`16:22:26 → 16:22:26`), caso em que a leitura do detalhe chega **depois** do evento de fim — foi assim que ele ficou preso em "em andamento" com a corrida já `success` no banco (defeito #68, visto no print da janela das 16:12 com o **#45**). Lido de novo no binário das 16:11, às **16:33**: a linha do histórico `#46 Piloto Etapa 2 → nesta máquina … 16:22:26 · v17 · f40243ff · grafo · 1 de 1 nós` abre o cartão com `sucesso` e o botão **Repetir** presente, `em andamento` **ausente** do texto visível da tela. Causa e correção no §5, item 24 |
| Executando | `devops-desktop.exe` aberto por `Start-Process`, `MainWindowTitle = "DevOps Desktop — Central de SSH, Git e Deploy"`, `Responding = True`, 10 telas navegáveis por clique. **Etapa 5 (o exe das 16:11, aberto às 16:12, lido no DOM agora):** **11** entradas na navegação — `Dashboard, Servidores, Terminal, Arquivos remotos, Git, GitHub, Projetos, Deploy, Análise, Histórico, Configurações`. **Etapa 6 (o exe das 15:05, reaberto para a medição):** `Get-Process devops-desktop` → `Id 29016`, `Responding True`, e a navegação foi usada de verdade: clique real do mouse em **Deploy** (112, 357), na linha **#52** do histórico (662, 358) e em **Saída da etapa** (348, 522) — o painel abriu, leu e renderizou os blocos |
| Git | `git push origin main` → `8e328c8..0308e84`, saída 0, e `git ls-remote origin refs/heads/main` devolve `0308e84c4b00f0c28c5c42e40dfbba0bb8e4d6b9`, o mesmo `git rev-parse HEAD` local — a **etapa 4 está pública** em <https://github.com/DouglasGDev/devops-desktop>. O push foi autorizado por ele nesta conversa, depois de eu registrar que a autorização anterior tinha coberto só a etapa 3. Etapa 3, medida na época: `efe551d..23beec2` |
| **Git, verificações gravadas (esta rodada)** | `git push origin main` → **`4acfe75..00b8c81`**, saída 0, e `git ls-remote origin refs/heads/main` devolve `00b8c81325755dc89d48a22aece513d19aa7ce79`, igual ao `git rev-parse HEAD` local — os três commits da rodada (`a63bf00` código, `fa425fa` + `00b8c81` docs) estão públicos em <https://github.com/DouglasGDev/devops-desktop>. O push foi autorizado por ele na pergunta desta conversa ("Empurrar agora"), depois de eu registrar que o sim da etapa 5 não se estendia. Um commit depois, o que escreve esta linha: `git push origin main` → `00b8c81..50e4832`, `ls-remote` devolvendo `50e483293db46b7ea457fc15e1ba055f6f335c89`, autorizado na mesma conversa. O terceiro: `git push origin main` → **`50e4832..7a0d865`**, saída 0, levando os dois commits de linha (`56e7721`, `7a0d865` = o §1.1), com `ls-remote` devolvendo `7a0d865c21b09f09ff6777a812c6caa1fa24319b` — igual ao `git rev-parse HEAD` local, e autorizado pela frase "empurre os dois". O commit que grava este parágrafo fica local até a próxima pergunta |
| Git, etapa 5 (lido às 20:23) | o **primeiro push foi recusado** pelo GitHub: `! [remote rejected] main -> main (push declined due to repository rule violations)`, com `ls-remote` ainda em `df60996…` — o protetor de segredos do repositório viu num fixture de teste uma chave live de verdade (forma, não conteúdo; §5 item 26). Corrigido o fixture na causa, os commits locais reunidos em um só (`git reset --soft`, nenhum trabalho perdido) e então: `git push origin main` → **`df60996..5f4df6f`**, e `git ls-remote origin refs/heads/main` devolve `5f4df6fd882981a0c7c068fa7940f436d9e7ca14`, igual ao `git rev-parse HEAD` local. A **etapa 5 está pública**, autorizada por ele na pergunta desta conversa ("Empurrar agora") |
| Git, etapa 6 (lido às 15:12, antes do commit desta rodada) | `git rev-parse HEAD` e `git ls-remote origin refs/heads/main` devolvem o mesmo `1985c391dd9b71a11b5f3a570abf0c0dfc7c3172` — o empurrão que ele autorizou na mensagem de aprovação da etapa chegou, e a conferência foi feita **depois** do push, como pedido. O commit que grava a etapa 6 (código + testes + estas linhas) **fica local** até a autorização ser pedida na frase; nada aqui foi empurrado por extensão de um "sim" anterior |
| Git, fechamento do gate (lido às 20:53) | `git push origin main` → **`5f4df6f..94344ae`** (o commit de docs `5d62e1a` foi junto), e `git ls-remote origin refs/heads/main` devolve `94344ae0a661d94a9f1e7c8a67ddd70934a108b3`, o mesmo `git rev-parse HEAD` local — a medição do §6 item 7 e a correção do §5 item 27 estão em <https://github.com/DouglasGDev/devops-desktop>. Autorizada por ele na mesma frase: "terminando o teste, publique no github" |
| **Loop desta rodada** (lido 19:2x → 19:5x) | `cargo test --lib` → **`314 passed; 0 failed; 0 ignored; 0 measured; finished in 20.09s`** (relido depois de todas as edições, inclusive da mudança de três estados no `is_repo`; eram **308** antes da rodada). Os seis testes novos, cada um lido pelo nome antes de entrar na contagem: `a_leitura_de_blob_devolve_conteudo_ausencia_e_sem_resposta` e `a_sondagem_devolve_ausencia_e_nunca_confunde_com_sem_resposta` (`git.rs:1963`/`git.rs:2004`), `uma_sondagem_lenta_e_abandonada_com_o_motivo` e `sonda_distinge_pasta_sem_git_de_pasta_que_nao_respondeu` (`git_probe.rs:108`/`:130`), `uma_leitura_impossivel_para_a_varredura_em_vez_de_devolver_limpo` (`security.rs:709`) e `uma_espera_e_dita_na_unidade_que_faz_sentido` (`sshexec.rs:449`). `cargo clippy --all-targets -- -D warnings` → `Finished dev profile` **sem uma linha de `warning`**; foi assim que saíram os dois achados da rodada (o `iter().copied().collect()` do `sonde` e o `match`→`unwrap_or_default()` da colheita de bytes), nenhum com `#[allow]`. O frontend foi verificado **dentro** do `npm run tauri build`: `tsc --noEmit && vite build` → `✓ built in 5.36s`, `dist/assets/index-C287853D.js` **884.95 kB** (gzip 240.18 kB) às 19:41, com o único aviso sendo o de chunk > 500 kB. `node scripts/confira-repo.ts` → `todas as 9 formas conferem`. O binário: `Finished release profile [optimized] target(s) in 5m 44s`, `Built application at: …\release\devops-desktop.exe`, `Finished 2 bundles at:` (nsis **4.92 MiB**, msi **8.21 MiB**), `CODIGO=0`, arquivo de **19.543.040 bytes** às **19:47** — posterior à última edição de fonte. Diff desta rodada: **7 arquivos, +467/−133** (`git diff --cached --shortstat`): `git.rs` +171/−52, `git_probe.rs` +106/−28, `sshexec.rs` +63/−11, `commands.rs` +55/−28, `security.rs` +36/−7, `README.md` +11/−6, `docs/ENTREGAS.md` |
| **Nenhuma leitura de Git ou de caminho sem prazo, na janela real** (CDP no exe das 19:47) | As nove chamadas do `reload()` da aba Git e as duas checagens de segredo **resolvem todas**, com o tempo lido dentro da própria janela: `git_status(2)` **1.031 ms** (14 arquivos, `ahead 1` sobre `origin/main`), `git_graph(2, 200, true)` **412 ms** com **43** linhas, `git_identity` 618 ms, `git_branches` 618 ms, `git_tags` 413 ms, `git_remotes` 822 ms, `git_stash_list` 411 ms, `git_secret_check(index)` **1.860 ms** lendo os **7 blobs preparados** (7 achados) e `git_secret_check(push)` **3.508 ms** (8 achados). A porta dos blobs, que era a que rodava sem prazo: `git_tree(2)` 413 ms com 154 entradas · `git_file_content(git.rs)` **411 ms**, `text` com **124.004** caracteres e `truncated: false` · `git_file_content(icon.png)` 409 ms devolvendo `text: null` com a nota `arquivo binário — não há texto para mostrar` · `git_file_content(nao-existe.txt)` 411 ms devolvendo **ausência respondida** — `ssh-master: o Git não tem nao-existe.txt no HEAD atual` — e não um conteúdo vazio · `git_commit_detail(2, HEAD)` 1.022 ms. A sondagem com prazo pelo caminho do projeto: `refresh_project(3)` **626 ms** → `is_git_repo true`, `git_branch "main"`, `git_remote null`. Pasta que não é repositório: `git_status(4)` → `Piloto Etapa 5 não é um repositório Git — use a seção de projetos para abrir uma pasta com repositório`, ou seja, o "não" que o git respondeu. Caminhos que não há o que consultar: `add_project("Z:\repo-externo")` → **3 ms** com `a pasta Z:\repo-externo não pôde ser acessada: O sistema não pode encontrar o caminho especificado. (os error 3)`; `add_project("\\nao-existe-devops\repo")` → **1.273 ms** com `O caminho da rede não foi encontrado. (os error 53)`; nenhuma das duas inseriu projeto (`list_projects` continua em **3**), porque a sondagem vem antes do `insert_project` — e é por isso que o caso "existe, mas não acorda" segue em §6 linha 8. **A tela que ele descreveu:** clique em **Atualizar** com o projeto 2 selecionado → o rótulo `Lendo o repositório…` está no DOM 250 ms depois do clique e **sumiu** quatro segundos depois; a seção **Histórico** renderiza as linhas com hash (`1bb0590`, `1985c39`, `7a0d865`) sem nenhum `Lendo o…`/`Atualizando as…` preso. O spinner nunca foi o defeito — o `finally` do `guard()` sempre limpou; o que não existia era a promessa resolver. **Rasura honesta, lida na rodada seguinte:** "sempre limpou" valia para o binário de release, que foi onde medi. Na janela de **desenvolvimento** o `finally` limpa **nada**, porque o `alive` do `useGitRepo` era de uma via só — §5, item 37, com o print dele como queixa. Nota de leitura para a próxima vez que alguém rodar a checagem aqui: os 7 achados do índice são `sk_live_`, `ghp_`, `AKIA…`, bloco de chave privada e cabeçalho Bearer em `security.rs` (os **fixtures do próprio scanner**) mais `docs/ENTREGAS.md` (que cita essas formas) e `commands.rs` ("atribui um valor a `password`") — falsos positivos de forma, não desta rodada, e o commit desta correção foi fechado por `git` na linha de comando, onde o painel não atua |
| **Git, rodada das leituras com prazo (lido às 20:04)** | `git log --oneline -3` → `6b7459e` (a correção: 7 arquivos, `+467/−133`) sobre `1bb0590` (a etapa 6) sobre `1985c39` (o que está público); `git status -sb` → `## main...origin/main [ahead 2]`; `git ls-remote origin refs/heads/main` → `1985c391dd9b71a11b5f3a570abf0c0dfc7c3172`, lido **depois** de fechar o commit, conferindo o que a linha afirma. **Nada foi empurrado**: nem a etapa 6 nem esta correção — o push é ação compartilhada e a autorização é pedida por commit, na frase, nunca por extensão de um "sim" anterior.  O commit que grava este texto fica local até a próxima pergunta |
| **A varredura para na porta, com prazo** (CDP no exe das 21:16, §4.1) | primeira linha da medição, `location.href` → `http://tauri.localhost/`, o binário certo. `analysis_executar` no projeto 4 com `path "src"` → **12 ms**, corrida **#19** `concluida` com `root C:\Users\dougl\AppData\Local\Temp\piloto-etapa5\src` conferido caractere a caractere **sem** o prefixo `\\?\` (a barra foi montada na própria janela por `String.fromCharCode(92)`, para a comparação não depender de escape de shell — escutar `\\\\` no `bash` desta máquina comia as barras e devolvia `C:Usersdougl…`) · `filesScanned 2` · contagem `{error 5, warning 3, info 0}`. `analysis_executar` com `path "nao-existe"` → **2 ms** com `a pasta varrida "C:\…\piloto-etapa5\nao-existe" não existe dentro do projeto`: **ausência respondida**, não "não respondeu". As duas coisas continuam sendo frases diferentes no teste `a_varredura_para_na_porta_quando_a_pasta_nao_acorda` (`analysis.rs:2590`), que pede um prazo de **0 ms** e exige `não respondeu em 0 ms` no lugar da ausência |
| **A contenção do destino vale quando o destino ainda não existe** (CDP, mesma janela) | as duas definições do piloto passam, e isso é a regressão: `pipeline_validate(20)` **3 ms** → `definição "Piloto sem servidor" v22 · artifacts · destino C:\Users\dougl\AppData\Local\Temp\piloto-etapa2\publicado · nesta máquina · grafo de 8 nó(s) · 6 etapa(s) do motor, 1 de comando, 1 de aprovação, 0 de git, 0 de análise`; `pipeline_validate(22)` **2 ms** → a mesma forma para *Gate de análise* **v2** com `grafo de 2 nó(s)`. A guarda foi medida do lado que não tinha prova, criando três modelos **temporários** no projeto 4 pela própria porta e apagando cada um logo depois com `delete_deploy_profile`: destino dentro do projeto e **pasta ainda não criada** → **3 ms** com `a pasta de destino (C:\…\piloto-etapa5\publicado-agora) está dentro do projeto (C:\…\piloto-etapa5) — publique em uma pasta fora do código-fonte` — o caso que **nunca disparava** antes, porque o `canonicalize()` de um destino inexistente falhava e a comparação nem acontecia; destino = **a própria raiz** → **2 ms** com `é o projeto ou um lugar de onde ele estaria dentro`; destino **fora** (`E:\ssh-master`) → **2 ms** aceitando `sem grafo · as quinze etapas do motor, em fila`. A grafia diferente não dobra a guarda, medido: `C:\USERS\dougl\…\piloto-etapa5\publicado-agora` e a mesma pasta **tudo em maiúsculas** devolvem as duas recusas acima em **3 ms** e **2 ms**, porque `canon_ou_ancestral` resolve o pedaço que existe na grafia do disco. Terminadas as sondas: `list_deploy_profiles` → **22 v2** e **20 v22**, `secret_keys` `[]` nos dois, projeto 4 continua `git=false` — nada do piloto foi escrito por estas medições, só os modelos temporários e as suas corridas |
| **A pasta de trabalho de um nó, medida dos dois lados** (CDP, mesma janela) | `pipeline_save_nodes` com `workingDir` `..\fora` → **2 ms** recusando `nó "sai-do-projeto": a pasta de trabalho deve ser um caminho relativo dentro do projeto`. É a primeira de **duas** camadas e é **léxica**, aplicada ao salvar: pelo aplicativo o `"sai do projeto"` do `dir_local` do motor não é alcançável com esse valor, e fica provado só em teste (`a_pasta_de_trabalho_de_um_no_e_lida_com_prazo_e_a_falta_e_dita`, `deploy.rs:4923`). O caso que a tela alcança é a pasta que não existe: nó `ausente` com `workingDir "pasta-que-nao-existe"`, salvo, **Rodar sozinho** por `pipeline_run_node` → corrida **#57** `failed` em **377 ms** com a linha lida de `deploy_run`: `a pasta de trabalho "pasta-que-nao-existe" do nó "Sonda de pasta ausente" não existe dentro do projeto: O sistema não pode encontrar o arquivo especificado. (os error 2)` — o motivo do sistema operacional **junto**, não um "falhou" seco. `pipeline_step_logs(57, posição 1)` devolve **zero linhas**: o nó nunca começou a executar comando, e não se inventa saída para ele — a mesma regra da linha "Corrida anterior à migração não inventa linha" acima. Apagado o modelo temporário, a #57 saiu junto no `ON DELETE CASCADE` e `deploy_runs` voltou a **32** (`#23` a `#54`) |
| **A sonda do agente com prazo** (CDP + linha de comando, mesma janela) | `discover_ssh` → **211 ms** com `agente {available: false, keys: [], note: "SSH Agent não está em execução (ou não há socket disponível)."}` — o veredito do **código 2** do `ssh-add -l`, não mais a nota "ssh-add não encontrado no PATH" que um `Err` genérico imprimia. Lido na linha de comando para a frase ter de onde sair: o serviço `ssh-agent` está **STOPPED**, `ssh-add -l` do Git devolve código **2** em **49 ms** e o de `System32` código **2** em **39 ms** — agente que responde rápido e não tem chave nenhuma, que é justamente o estado que antes se confundia com "não existe". Travado por `os_quatro_vereditos_do_agente_sao_coisas_diferentes` (`ssh_config.rs:633`), por `uma_sonda_de_agente_que_nao_termina_e_abandonada_dentro_do_prazo` (`ssh_config.rs:664`, que segura um `ping -n 40` real contra um prazo de 1 s e abandona em menos de 10 s) e por `a_leitura_real_do_agente_volta_com_um_veredito` (`ssh_config.rs:683`, que roda o `ssh-add` desta máquina) |
| **Loop desta rodada** (lido 21:1x → 21:4x) | `cargo test --lib` → **`322 passed; 0 failed; 0 ignored; 0 measured; finished in 18.37s`** — **8** a mais que os 314 da rodada anterior, cada um lido pelo nome antes de entrar na contagem: `canon_separa_nao_existe_de_nao_respondeu` e `a_regua_do_destino_resolve_o_lado_ausente_pelo_ancestral` (`git_probe.rs:206`/`:255`), `a_varredura_para_na_porta_quando_a_pasta_nao_acorda` (`analysis.rs:2590`), `a_pasta_de_trabalho_de_um_no_e_lida_com_prazo_e_a_falta_e_dita` e `a_regua_do_destino_compara_os_dois_lados_na_mesma_forma` (`deploy.rs:4923`/`:4952`), `os_quatro_vereditos_do_agente_sao_coisas_diferentes`, `uma_sonda_de_agente_que_nao_termina_e_abandonada_dentro_do_prazo` e `a_leitura_real_do_agente_volta_com_um_veredito` (`ssh_config.rs:633`/`:664`/`:683`). Os cinco testes que dependem de um prazo de **0 ms** — os quatro de caminho acima mais
`a_regua_do_destino_resolve_o_lado_ausente_pelo_ancestral` — foram relidos em loop antes desta linha entrar aqui: **10
execuções de cada, 50 no total, 0 falhas**, porque um teste que às vezes passa não é prova de determinismo.
`cargo clippy --all-targets -- -D warnings` → `Finished dev profile [unoptimized + debuginfo] target(s) in 0.76s` com **zero** linhas de `warning` — foi assim que saiu o `enum_variant_names` da sonda do agente, corrigido renomeando a enumeração para `Sonda`, sem `#[allow]`. `node ./node_modules/typescript/bin/tsc --noEmit` → saída vazia, `CODIGO=0`. `node scripts/confira-repo.ts` → `todas as 9 formas conferem`. Diff: **4 arquivos, +478/−61** (`analysis.rs` 59/15, `deploy.rs` 116/21, `git_probe.rs` 168/0, `ssh_config.rs` 135/25). Binário das **21:16:46**, **19.575.808 B**, posterior à última edição de fonte das 21:08:36, `Finished release profile [optimized] target(s) in 6m 01s` e `CODIGO=0` lidos no log do build |
| **Exportar escreve no arquivo que a pessoa escolheu** (CDP no exe das 22:12, §4.1) | primeira linha da medição, `location.href` → `http://tauri.localhost/`. A tela **Análise** com o projeto 2 (`ssh-master`) selecionado lista as corridas `#18, #17, #15, #10, #9`, todas `concluida` + `barra` com **2.306 / 2.305 arquivos** e contagem **676/42/640** — a #17 com instrumento `ambos`, e mesmo assim a mesma contagem. Medido pelo comando, com caminho absoluto escolhido: `analysis_exportar(18, "pacote", …\Temp\analise-corrida-18.json)` devolve o próprio caminho em **7 ms**, lido de volta com 123.822 B, `esquema devops-desktop/analise@1`, `execucao.id 18`, `veredito.classificacao "barra"`, `truncado true`; `analysis_exportar(18, "sarif", …\Temp\para-a-ci.sarif)` escreve 789.728 B com `version 2.1.0` e **1.358** `results` (o total, não as 300 linhas do pacote truncado). As três recusas, cada uma sem deixar arquivo: caminho relativo → `“analise.json” não é um arquivo com caminho completo — escolha a pasta onde a exportação vai ficar`; extensão de outro formato → `um arquivo de pacote termina em .json; …\analise.html não termina`; pasta que não existe → `não foi possível gravar …\nao-existe-dd-9z4k\x.sarif: O sistema não pode encontrar o caminho especificado. (os error 3)` — o motivo do sistema operacional **junto**, a mesma regra da linha "A pasta de trabalho de um nó" acima. A pasta de dados **não recebeu nada novo**: `…\com.douglasgdev.devopsdesktop\analises` continua com os dois arquivos das 20:48/20:53, lidos antes e depois. O caminho devolvido não tem prefixo `\\?\` (`git_probe::simple`, a mesma régua da rodada anterior). **O que a janela prova e o que não prova:** depois do clique real no botão **Exportar pacote** da corrida #18, `EnumWindows` do PID 34600 lista `[Exportar pacote] · #32770` ao lado da `Tauri Window` — o diálogo nativo abriu, e a tela não mostrou caminho nenhum nem toast (a promessa está parada esperando a escolha, que é o estado certo). O que há **dentro** do diálogo — o nome sugerido, o filtro, o cancelamento — não foi medido: entrada sintética de sistema não é usada neste projeto, e isso fica para o olho dele. Os arquivos e a janela de medição foram meus e foram fechados: `TaskStop` na instância que eu abri e os dois arquivos de `%TEMP%` apagados; os `corrida-18.*` dele, não. **Fechado por ele mesmo às 22:45, sem mim:** o arquivo `E:\ssh-master\dist\analise-corrida-21.json` **123.823 B** chegou anexado na conversa, e é a escolha da janela dele no diálogo — corrida **#21**, projeto 2, caminho dentro do projeto, `esquema devops-desktop/analise@1`, `execucao.id 21`, `duracaoMs 13850`. O que isso prova a mais do que a minha medição: o clique real produz o arquivo **no caminho que ele apontou**, com o nome que a tela sugeriu editado por ele (`analise-corrida-21.json`), e o conteúdo é o pacote lido campo a campo (`veredito.classificacao "barra"`, `truncado true`, `observacao` da mascaráção presente) |
| **Loop desta rodada** (lido 22:0x → 22:1x) | `cargo test --lib` → **`323 passed; 0 failed; 0 ignored; 0 measured; finished in 18.70s`** — um a mais que os 322 da rodada anterior; os três testes que tocam a exportação, lidos pelo nome: `exportar_escreve_o_mesmo_texto_que_a_tela_leu` (agora afirma o nome escolhido **e** que `dados/analises` não existe mais), `o_sarif_exportado_sai_das_linhas_gravadas` (escreve `varredura.sarif`) e o novo `um_caminho_que_nao_serve_nao_escreve_arquivo_nenhum` (`analysis.rs`), que pede relativo, extensão errada e pasta inexistente e confere `exists()` de cada um. `cargo clippy --all-targets -- -D warnings` → **zero** linhas de `warning`. `node ./node_modules/typescript/bin/tsc --noEmit` → **`CODIGO=2` na primeira vez**, com `src/views/AnalysisView.tsx(230,27): error TS2304: Cannot find name 'save'` — o import faltava, e é o tipo de coisa que o loop pega justamente porque não se presume nada; corrigido o import, `CODIGO=0`. `npm run build` → `✓ built in 5.66s` com o único aviso de chunk > 500 kB; `dist/assets/index-BLZoTx6l.js` **885.308 B** às 22:06:16, hash **novo** porque a tela mudou. `node scripts/confira-repo.ts` → `todas as 9 formas conferem`. `npm run tauri build` → `Finished 2 bundles at:` com código de saída 0, nsis **5.169.366 B** (4,93 MiB) às 22:11:57, msi **8.630.272 B** (8,23 MiB) às 22:12:02, `devops-desktop.exe` **19.576.320 B** às 22:12:08. Superfície lida depois: **148** comandos, **46.051** linhas em **78** arquivos. Diff desta rodada, lido de `git diff --numstat`: **6 arquivos, +183/−42** (`analysis.rs` 90/13, `commands.rs` 3/3, `api.ts` 2/2, `AnalysisView.tsx` 9/2, `README.md` 3/1, `ENTREGAS.md` 76/21) — **104** linhas novas de código contra **20** substituídas |
| **Aba Git presa em "Lendo o repositório…" — a causa está no ref, não no git** (print dele, 22:3x) | Lido do próprio print o par de estado que não tem como acontecer por leitura em andamento: o rótulo do cabeçalho **e** o placeholder da lista dizem `Lendo o repositório…`, que é `git.busy === "Lendo o repositório"` com `status === null` (`GitView.tsx:525`), e nenhum dos dois aparece sem o `Promise.all` das sete leituras ter **resolvido** — `reload()` só escreve estado depois dele, e o `finally` do `guard` limpa o rótulo sempre. Então o que estava preso não era o git: era a tela deitando a resposta fora. Confirmado por três leituras de fora: (1) `curl http://localhost:1420/` → **200**, ou seja, a janela dele é o **modo de desenvolvimento** (`tauri dev`, `devUrl` do `tauri.conf.json`), e `src/main.tsx:7` liga `React.StrictMode`, que nesse modo monta, desmonta e monta — com o `useEffect` de uma via só do `useGitRepo.ts:115` (`() => () => { alive.current = false }`), o ref fica `false` para sempre e os dois lados do `guard` (`setBusy(null)`, `setError`, o `notify` do toast e o `setStatus` do `reload`) passam a não executar; (2) as cinco operações de git do `reload()`, cronometradas na linha de comando **sobre o mesmo repositório e o mesmo estado de disco dele** (`Python/` de 136 MB sem rastrear na raiz, cinco commits locais à frente): `status --porcelain=v2 -z` **77 ms**, `stash list` **69 ms**, `tag -l` **76 ms**, `remote -v` **66 ms**, `log --graph -n 200` **79 ms** — o git respondeu, e respondeu rápido; (3) `grep` do `alive` em `src/` dá **uma** dona, então nenhuma outra tela engolia o próprio fim por esse caminho. Corrigidos os dois (o ref de duas vias e o **limpo** que a tela afirmava com `status === null`, `GitView.tsx:427`), loop relido: `tsc --noEmit` `CODIGO=0`, `npm run build` `✓ built in 6.03s`, `cargo test --lib` **323 passed; 0 failed** em 18.48s, clippy **zero** avisos, `todas as 9 formas conferem`. **O que esta linha NÃO mediu, e por isso a correção ainda não está conferida na janela:** eu não reproduzi o defeito numa janela minha — abrir uma segunda instância não dá porta de debug (o processo de navegador do WebView2 é da instância dele; `Get-Process` mostra **um** `devops-desktop.exe`, PID **39732** das **22:31:05**, e `curl 127.0.0.1:9223` recusa conexão), e fechar a janela dele para medir destruiria o espécime; a cópia do `.db` para ler o Histórico dele foi **recusada** pelo classificador de permissões, e não foi contornada. Fica assim: causa lida no código, estado lido no print, git descartado por medida, e a confirmação do conserto é o clique dele em **Atualizar** na mesma janela aberta (o vite serve o módulo corrigido), que é justamente o tipo de leitura que a §6 linha 4 diz que não tenho como fazer sozinho |
| **Loop da revisão da validação** (relido 23:5x → 00:0x) | `cargo test` → `test result: ok. **336 passed; 0 failed; 0 ignored; 0 measured**; finished in 18.45s` no binário da lib, `0 passed` no do `main.rs` e **0** doc-tests (`src-tauri/target/final-tests.log`) — os 323 das rodadas anteriores mais os **13** desta. `cargo clippy --all-targets` **depois de `touch src/*.rs`** (com cache o "Finished" não reemite aviso nenhum, e isso não seria medição) → `Finished dev profile in 13.71s`, **0** linhas de `warning`/`error` (`final-clippy.log`). `npx tsc --noEmit` → saída **0**, nenhuma linha. `npm run build` → `✓ 86 modules transformed`, `✓ built in 5.66s`. **A sonda que reproduz a máscara** (`node scripts/sonda-mascara.mjs`, que lê as funções reais de `security.rs`/`analysis.rs`, compila com `rustc` e roda) → relida agora, sobre o código deste commit: **24 PASS** (21 linhas × escondido/à-vista/idempotente + 3 da cauda), `falhas_reproduzidas=0`. O estado **antes** da correção, na mesma sonda às 22:5x: `falhas_reproduzidas=10` com `passwd`, `secret`, `api_key`, `apikey`, `access_key`, `auth_token`, `password_com_espaco`, `duas_atribuicoes` e `unicode_bearer` em FAIL, e o panic `start byte index 1 is not a char boundary; it is inside 'é' (bytes 0..2 of string)`. **O instrumento real, duas medições** (`%TEMP%\semgrep-probe\Scripts\semgrep.exe`, 1.181.0): fora de qualquer repositório Git, sobre um `a.py` que as regras flagram → `{"results":[],"errors":[],"paths":{"scanned":[]}}`; depois de `git init` + `git add a.py` + commit, o mesmo comando → `codigo=0`, `results 2`, `errors 0`, `scanned ["a.py"]`, **sem** a chave `skipped`, `check_id` pontuado (`E.ssh-master.src-tauri.regras.dd-101-shell-true`) e `extra.lines` = `"requires login"` — por isso o trecho é lido por nós. No diretório do projeto a terceira medição (`review-validation-semgrep-real.log`) devolveu `status 0`, versão 1.181.0 e arquivos **sob `Python/`** na lista de lidos, com `git ls-files Python/` em **0** (154 rastreados no total): a régua que estava no envelope, o `check_id`, o `cwe` como texto e o `skipped` ausente, é a que o fixture guarda. **Banco lido numa cópia** (`%TEMP%\devops-rodada-leitura\rd.db`, o original intocado): corrida mais nova **#22** — `concluida`, `embutido`, 2.306 lidos / 167 pulados, counts `{"error":676,"warning":42,"info":640}`, `blocked 1`, corte `ERROR`, aberta `2026-10-22T03:22:08Z` — **1.358** linhas de achado nela e na #21, **0** snippets contendo `«valor mascarado»` e **0** contendo `password = FICT…`, `ghp_` ou `sk_live_` nas duas; `deploy_runs` parado na **#54**. **O que NÃO foi medido, e a linha diz para não presumir:** nada acima passou pela janela. O `target/release/devops-desktop.exe` em disco é das **22:12:08**, anterior às edições que fecharam em `c2c1877` (23:57:53) — não posso afirmar que ele contém a máscara nova, e por nenhuma linha desta rodada depende de janela ela continua por conferir. Primeiro passo quando ele quiser: `npm run tauri build`, varredura no **Piloto Etapa 5** (o do token plantado, corrida **#6**) e ler `analysis_findings.snippet` da corrida nova. **E uma correção de registro:** a mensagem do `c2c1877` escreveu "só varre o que o Git rastrea" — a medição do diretório do projeto acima **não** sustenta essa frase (ela sustenta `scanned: []` **fora** de um repositório); o commit está local e não foi empurrado, e a linha fica aqui em vez de reescrever a mensagem |
| **Git desta rodada (lido antes do commit)** |`git log --oneline -1` → `e0c2753 fix(caminho): as seis leituras que ainda esperavam o disco, e a guarda que nunca disparava`; `git status --porcelain` → só `?? Python/` (a distribuição CPython portável de 136 MB na raiz, **não rastreada**, `git check-ignore` devolve código 1 — ela não é ignorada, só nunca foi adicionada); `git ls-remote origin refs/heads/main` → `1985c391dd9b71a11b5f3a570abf0c0dfc7c3172`, com **quatro** commits locais à frente (`1bb0590`, `6b7459e`, `2738e41`, `e0c2753`). **Nada foi empurrado**; o commit que grava estas linhas fica local até a autorização ser pedida na frase, como em toda rodada |
| **Push dos cinco nomeados** (lido 22:5x) | a frase dele comitou a correção da tela e nomeou **cinco** hashes. O commit da tela saiu como `26e6ea1` às **22:50:50** — `3 files changed, 54 insertions(+), 10 deletions(-)` (`docs/ENTREGAS.md`, `src/lib/useGitRepo.ts`, `src/views/GitView.tsx`), com `?? Python/` ficando de fora como sempre. O empurrado foi **exatamente** a lista: `git push origin 32008f5:refs/heads/main` → `To https://github.com/DouglasGDev/devops-desktop.git` e `1985c39..32008f5  32008f5 -> main`, com o **refspec explícito** em vez de `origin main` justamente porque `git push origin main` levaria junto o sexto commit, que ele não nomeou. Relido depois: `git ls-remote origin refs/heads/main` → `32008f5bd85c92025e0c26bb0a48016cc16399fc` (o remoto bate com o nomeado, não presumido), `git rev-parse HEAD` → `26e6ea181979f2bbc6457ccbb37c1cce217c1cdb`, ou seja, **um** commit local ainda não empurrado, à espera da própria autorização na frase. Os cinco que chegaram ao remoto: `1bb0590` (saída de etapa no banco), `6b7459e` (leituras de Git com prazo), `2738e41` (docs das medições), `e0c2753` (as seis leituras de caminho com prazo) e `32008f5` (a exportação perguntando onde gravar) |
| **Aba Git relida, de regresso** (CDP no mesmo exe das 21:16) | as leituras da rodada anterior, repetidas no projeto 2 para conferir que as portas novas não mudaram nada: `git_status(2)` **1.024 ms** com `branch main`, `upstream origin/main` e **ahead 3 / behind 0** (os três commits locais), `git_graph(2, 5, true)` **412 ms**, `git_branches` **614 ms** (2), `git_remotes` **820 ms** (`origin → https://github.com/DouglasGDev/devops-desktop.git`), `git_identity` **614 ms**, `git_stash_list` **410 ms** (0), `git_tree` **411 ms** (154 entradas), `git_commit_detail(2, 2738e41ddda49aa…)` **1.030 ms**, `git_diff(from: "HEAD")` **414 ms** devolvendo exatamente os **4** arquivos `M` desta rodada (`analysis.rs`, `deploy.rs`, `git_probe.rs`, `ssh_config.rs`) — a tela vendo o mesmo que `git status --porcelain`. `git_secret_check(2, "tracked")` **413 ms** devolve **0** achados, e isso **não** contradiz os 7 da linha anterior deste §2: aquela medição foi no escopo `index`, esta no escopo `tracked` — são checagens diferentes, com números diferentes, e a igualdade delas não foi presumida |

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
- **Log de etapa persistido e pesquisável** (etapa 6 do plano CI/CD — `src-tauri/src/db.rs`, `src-tauri/src/deploy.rs`,
  `src-tauri/src/pipeline.rs`, `src-tauri/src/audit.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/models.rs`,
  `src/components/deploy/StepLogs.tsx`, `src/components/deploy/StepPipeline.tsx`, `src/views/DeployView.tsx`): a saída de
  uma etapa deixa de viver só na janela. A migração **11** cria `deploy_step_logs` `(run_id, position, attempt, seq,
  stream, ts, text)` com `CHECK (stream IN ('stdout','stderr','system'))`, índice **único** em
  `(run_id, position, attempt, seq)` — que é a forma da promessa: *uma linha, num lugar, uma vez* — e índice de leitura
  `(run_id, position, seq)`, com `ON DELETE CASCADE` para `deploy_runs`. No motor, `Ctx::log` faz uma coisa só a mais:
  mascara, **grava o lote** em `db::append_step_logs` e então emite no `deploy-log`; `deploy::bloco_de_log` é quem decide
  o nome do fluxo (`out`→`stdout`, `err`→`stderr`, o resto →`system`), descarta linha em branco e aplica o teto de
  **2.000 linhas por tentativa** com a última linha `system` declarando o corte. A tentativa vem do contador que o
  escalonador **já tinha** (`ctx.attempt`), então o bloco da tentativa 1 fica intacto quando o nó repete. A leitura é
  **uma porta só**: `pipeline::step_logs(db, run_id, position, query, limit)` com `LIKE` ligado por parâmetro (nada de
  concatenar a busca no SQL) e tope **por bloco**, reexportada como o comando `pipeline_step_logs` (148) e usada pelo
  painel **Saída da etapa** no cartão da execução — a mesma tela que mostra as quinze etapas, agora com o que cada uma
  imprimiu, relido do banco. O que isto **não** toca, de caso pensado: `retry_limit`, aprovação e ambiente continuam
  como estavam; o evento `deploy-log` continua o canal da execução ao vivo (persistir não é substituir); e o log de uma
  corrida feita **antes** da migração não é reconstruído — a tela diz `nenhuma linha gravada nesta etapa`.

  Medido na janela real (§2): o `%CHAVE_PILOTO%` cru na **#49** e o `***` na **#50** com o valor ausente do `.db`, do
  `-wal` e do `-shm`; os **dois blocos** da **#51** com `seq` recomeçando e o aviso de repetição no bloco que falhou; as
  **2.001** linhas da **#52** terminando no aviso de corte, lidas inteiras com tope alto e em 500 no padrão, e o
  `truncado` encontrado pela busca da tela; e o fechar-e-reabrir que é o ponto da etapa — janela das 14:52 encerrada,
  binário das 15:05 aberto, `pipeline_step_logs` devolvendo as linhas de **#49** a **#52** sem sessão nenhuma no meio.

Total da superfície: **148 comandos Tauri registrados** (contados por `#[tauri::command]`, que vivem em `src-tauri/src/commands.rs`), 24 arquivos Rust, 11 views, **45.245** linhas de TS/TSX/Rust em **78** arquivos — contadas
por `git ls-files '*.ts' '*.tsx' '*.rs' | xargs wc -l` com o componente novo já rastreado, e relidas nesta rodada:
`node_modules`, `target`, `dist` e a pasta `Python/` (venv **não rastreado**, nunca `git add` nele) ficam fora da conta.
O que a etapa 6 escreve, lido de `git diff --numstat` e não estimado: **+317** em `db.rs`, **+161** em `deploy.rs`,
**+125** em `pipeline.rs`, **+35** em `audit.rs`, **+16** em `commands.rs`, **+13** em `models.rs`, **+23** em
`StepPipeline.tsx`, **+106** em `StepLogs.tsx` (arquivo novo), **+16** entre `api.ts`/`types.ts`/`DeployView.tsx` —
**812** linhas novas de código contra **25** substituídas. As verificações gravadas no Histórico (§5, item 28) continuam
as da rodada da análise: **171** linhas novas em `analysis.rs`, **30** em `commands.rs` (4 velhas substituídas) e **1** em
`ui.tsx`, ou **198** líquidas.

Persistência (SQLite local, migrações em `src-tauri/src/db.rs`) — as **quatorze** tabelas que existem hoje, lidas de
`sqlite_master` no banco real desta máquina (§1.1, `user_version 11`):
`settings`, `servers`, `projects`, `audit_log`, `routines`, `terminal_history`, `deploy_profiles`, `deploy_runs`,
`deploy_run_steps`, `deploy_step_logs`, `pipeline_revisions`, `pipeline_nodes`, `analysis_runs`, `analysis_findings`. Não há tabela de série
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
30. **Tentativa é bloco, não sobrescrita — e quem grava é o motor, no instante em que emite.** `deploy_step_logs` é
    chaveada por `(run_id, position, attempt, seq)` com índice **único**: a repetição de um nó abre `attempt + 1` e o
    `seq` recomeça do zero, então a falha que motivou a repetição continua lá para ser comparada com o que a tentativa
    nova imprimiu. O que não é gosto aqui: `ctx.attempt` sobe na **primeira instrução** do laço de retry, para
    o aviso `… repetindo o nó (tentativa 1/1)` ficar no bloco da tentativa que falhou (medido na **#51**, linha 3 do
    bloco 1), e a gravação acontece **antes** do `emit` no evento `deploy-log`, porque a tela pode fechar e o banco não.
    Uma mecânica, duas portas: `pipeline::step_logs` é a única leitura, usada pelo painel **Saída da etapa** e por
    qualquer consulta posterior — não existe um segundo caminho que "leia o log" montando texto de outra fonte. A
    gravação é **em lote**: uma chamada de `log` escreve de uma vez as linhas que aquele trecho de saída trouxe, sob o
    mesmo lock — foi assim que as 2.001 linhas da **#52** entraram numa só aquisição. E o `std::sync::Mutex` do
    `DbHandle` **não é reentrante**: `Ctx::registra` chamava `self.log()` enquanto segurava o `db.lock()`, o que trava
    a execução inteira no segundo `lock`; hoje o erro do `upsert_step` é colhido dentro do escopo e a linha é gravada
    **fora** dele.
31. **Máscara antes de gravar, e o contexto precisa carregar os valores.** A ordem não é negociável: `Ctx::log`
    produz `Secrets::mask(text)` → `audit::scrub_secrets` e **só então** entrega ao banco; a tela recebe o mesmo texto
    já mascarado. O que provou a necessidade da segunda metade foi a medição: a corrida **#49** gravou
    `envio %CHAVE_PILOTO%` **literal**, porque o `Secrets` do `Ctx` era montado só com as chaves declaradas e os
    valores fornecidos entravam por outro caminho — que a fila de quinze etapas refaz na etapa 1, mas um **nó rodado
    sozinho** nunca passa por ela. Corrigido na causa (`Secrets::new(&profile.secret_keys, &secret_values)` na
    construção do `Ctx`), a **#50** gravou `envio ***`. A prova de ausência foi feita **no arquivo** do banco, varrido
    byte a byte com `.db`, `-wal` e `-shm`, e não pela tela: o valor `valor-de-teste-7h4d` não está em nenhum dos três.
    Testes novos travam os dois lados: o que some (`o_valor_da_variavel_nao_chega_ao_log_gravado`) e o que fica
    (`enviando a.html`, `cabeçalho TOKEN=***`, `Authorization: Bearer ghp_***`, `fim`).
32. **O corte é declarado, nunca escondido.** Uma etapa pode imprimir mais do que o motor deve guardar; a regra é
    **2.000 linhas por tentativa** e uma última linha `system` dizendo `… truncado em 2000 linha(s) por tentativa — N
    linha(s) além do teto não foram gravadas`, escrita pelo `deploy::bloco_de_log` — quem lê a tela sabe que aquilo foi
    cortado, em vez de ver um log que parece completo. O aviso é escrito **uma vez** por bloco: `gravadas >= teto`
    devolve vazio, para o corte não fingir tamanho repetindo-se linha a linha (`o_teto_de_linhas_declara_o_corte_uma_só_vez`).
    Na leitura, o teto é `TETO + 1`: uma porta limitada a 2.000 **não conseguiria devolver a linha 2.001**, que é
    justamente o aviso — e o tope do pedido é **por bloco de tentativa** (`ROW_NUMBER() OVER (PARTITION BY attempt …)`),
    porque um bloco cheio não pode engolir o bloco da tentativa seguinte. Medido na **#52**: com tope 9.999 a porta
    devolve as 2.001 e termina no aviso; com o padrão devolve 500, que é um tope, não uma promessa; e na tela, buscar
    `truncado` faz o aviso aparecer.
33. **Mascarar não pode reescrever a saída.** O `audit::scrub_secrets` antigo tratava o texto inteiro como uma
    palavra só: `split_whitespace` + `join(" ")` **dizia** "não achata", mas achatava — e como `Ctx::log` chama a
    máscara em cada linha emitida pela porta, um valor que aparecesse em linha isolada perdia as quebras e os espaços
    que o comando produziu. Isso não era cosmético: uma saída de `ssh -v`, um `docker compose up` ou uma diff de arquivo
    perdem o sentido sem estrutura de linha, e o log gravado é justamente a coisa que se consulta depois. Corrigido na
    **causa**, em `scrub_key_values`: a linha é a unidade (`text.lines()`), e dentro dela só a palavra sensível é
    trocada, com `split_inclusive(char::is_whitespace)` e o resto do token preservado, para as quebras e os recuos
    voltarem como saíram. Travado por `a_mascara_nao_achata_as_linhas_da_saida`, que compara a string mascarada com o
    texto que `ssh -vvv` produziria — inclusive a linha em branco e o recuo de quatro espaços. Foi um teste desta etapa
    que o encontrou: `o_valor_da_variavel_nao_chega_ao_log_gravado` esperava quatro linhas e recebeu uma.
34. **Nenhuma leitura de Git ou de caminho roda sem prazo — e "não respondeu" nunca é lido como "não existe".** A
    regra já existia no texto (`git -C` sempre teve 30 s/10 min); o que esta rodada corrigiu é que **três leituras
    passavam por fora dela** e giravam para sempre: `git show <rev>:<caminho>` nos blobs (`read_blob`, chamado **uma vez
    por arquivo staged** na checagem de segredos), a sondagem de pasta (`git_probe::probe`, que ainda perguntava
    `.git` no disco e rodava `git` com `.output()` sem prazo) e a resolução do caminho (`canonical_dir`, que fazia
    `metadata()` **e** `canonicalize()` — as duas chamadas que esperam o volume dentro do kernel) **na thread do runtime
    assíncrono**. Contra um disco externo adormecido nenhuma delas devolvia nada, e a tela, que limpa o próprio spinner
    no `finally`, ficava girando sem erro. Hoje: todo processo de git sai de **uma única porta** (`git::abrir`) e é
    colhido por `sshexec::collect_bytes` com prazo; `sonde`, `blob_at`, `index_blob` e `is_repo` têm **três estados** —
    `Some`/`Ok(...)` = respondeu, `Ok(None)`/`false` = respondeu que não tem, `Err` = **não respondeu** — e o `Err` é
    propagado, nunca convertido em "sem branch", "sem remoto", "não é repositório" ou, na checagem de segredos, em
    varredura limpa (`head_findings`/`index_findings` agora devolvem `Result`). `canonical_dir` espera o disco numa thread
    auxiliar e abandona a espera com o motivo (`git_probe::com_prazo`, 20 s), e normaliza com `std::path::absolute`, que
    é **léxica** e não pergunta nada ao volume, em vez de `canonicalize()`. O trabalho de disco saiu da thread
    assíncrona: `git_read`/`git_write` resolvem o projeto **dentro** do `spawn_blocking`, e o mesmo vale para
    `scan_project_secrets`, `git_clone`, `analysis_executar` e o `open_in_editor` (que ainda resolve o caminho real para
    conferir o contenimento, só que pela sondagem com prazo). A colheita dorme **só o tempo que falta** para o prazo, não
    o passo fixo de 200 ms — com o passo fixo, um prazo de 1 ms acordava depois dele e colhia o processo como se tivesse
    respondido a tempo. Isso está travado por `a_leitura_de_blob_devolve_conteudo_ausencia_e_sem_resposta`,
    `a_sondagem_devolve_ausencia_e_nunca_confunde_com_sem_resposta`, `uma_sondagem_lenta_e_abandonada_com_o_motivo`,
    `sonda_distinge_pasta_sem_git_de_pasta_que_nao_respondeu`, `uma_leitura_impossivel_para_a_varredura_em_vez_de_devolver_limpo`
    e `uma_espera_e_dita_na_unidade_que_faz_sentido`.
35. **Quem compara caminhos compara as duas pontas na forma que o disco respondeu — e um prazo que se escolhe, se escolhe
    igual aos vizinhos.** A regra de não deixar leitura de caminho sem fim não valia fora da aba Git: seis chamadas de
    `canonicalize()` em três funções (`analysis::executar`, `deploy::dir_do_no`/`dir_local`, `deploy::checar_definicao`)
    esperavam o volume dentro do kernel, e um `Err` delas virava texto de erro genérico ou, no agente, "não está no PATH".
    Hoje as seis passam por `git_probe::canon` / `canon_ou_ancestral`, que devolvem `Resolveu` (já sem `\\?\`), `Ausente`
    (com o motivo do sistema operacional) ou `SemResposta` (com a duração lida), e nenhum dos três é convertido em outro.
    O ponto que não era óbvio e virou regra: **resolver um lado e comparar com o outro digitado é não comparar**. A
    contenção do destino em `checar_definicao` fazia exatamente isso e, como o destino da primeira publicação ainda não
    existe, a guarda `alvo.starts_with(&origem)` nunca disparava — ou seja, a proteção contra "publicar dentro do próprio
    projeto" estava ativa só quando não era preciso. `canon_ou_ancestral` resolve a pasta até onde o disco responde e
    acrescenta o resto do caminho, pondo os dois lados na mesma forma; a leitura da caixa confirma que a parte resolvida
    volta na grafia do disco, então maiúscula diferente não dobra a guarda (medido na janela, §2). Segundo ponto: um prazo
    inventado para uma sonda nova tem de ser o prazo das sondas vizinhas — a linha de pendência pedia 2 s para o
    `ssh-add`, e os 5 s escolhidos são o `PRAZO` que `tools.rs` já usa, para a descoberta de chaves não virar a exceção da
    casa. Travado por `canon_separa_nao_existe_de_nao_respondeu`,
    `a_regua_do_destino_resolve_o_lado_ausente_pelo_ancestral`, `a_varredura_para_na_porta_quando_a_pasta_nao_acorda`,
    `a_pasta_de_trabalho_de_um_no_e_lida_com_prazo_e_a_falta_e_dita`,
    `a_regua_do_destino_compara_os_dois_lados_na_mesma_forma` e `os_quatro_vereditos_do_agente_sao_coisas_diferentes`.

36. **Exportar é escrever no arquivo que a pessoa escolheu — o aplicativo não escolhe a pasta por ela.** O defeito chegou
    como queixa ("a análise de código não salva nada na pasta") e a leitura discordou da frase: os dois arquivos
    **existiam** em `%APPDATA%\com.douglasgdev.devopsdesktop\analises` (123.822 B e 789.728 B, das 20:48/20:53). O que
    não existia era qualquer superfície na aplicação capaz de dizer onde gravar — `exportar` fazia
    `dados.join("analises")` + `create_dir_all` + nome fixo `corrida-{id}`, e `save` do `@tauri-apps/plugin-dialog` não
    era importado em **nenhum** arquivo do `src/`. Ou seja: a exportação gravava sozinha num lugar que a tela nunca
    mostrava, e gravar onde o aplicativo decidiu sem perguntar não é exportar. Hoje `exportar`
    (`analysis.rs:1802`) recebe o destino, a porta `analysis_exportar` (`commands.rs:3347`) repassa o caminho escolhido
    pela janela e a tela pergunta no diálogo nativo de salvar; as duas funções puras de conteúdo (`pacote`
    `:1556`, `sarif` `:1728`) não mudaram uma linha. A regra que fica: **o nome e a pasta são de quem exporta, e um
    caminho que não serve não deixa arquivo nenhum** — relativo ou vazio (`“x.json” não é um arquivo com caminho
    completo — escolha a pasta onde a exportação vai ficar`), extensão de outro formato (`um arquivo de pacote termina em
    .json; …\analise.html não termina`; `sarif` aceita `.sarif` **e** `.json`, porque o conteúdo é JSON e um revisor de
    código espera esse nome), e a recusa carrega o motivo do sistema operacional junto quando o problema é o disco
    (`não foi possível gravar …\nao-existe-dd-9z4k\x.sarif: O sistema não pode encontrar o caminho especificado. (os error 3)`),
    que é a mesma régua dos três estados das linhas 34 e 35: um `Err` não vira sucesso nem ausência. O caminho devolvido
    passa por `git_probe::simple`, então sai sem o prefixo `\\?\` — a linha 27 vale para a exportação como valia para o
    gate. Travado por `exportar_escreve_o_mesmo_texto_que_a_tela_leu` (afirma o nome escolhido **e** que `dados/analises`
    não existe mais), `o_sarif_exportado_sai_das_linhas_gravadas` e `um_caminho_que_nao_serve_nao_escreve_arquivo_nenhum`
    (`analysis.rs:2709`), que pede relativo, extensão errada e pasta inexistente e confere `exists()` de cada um. O que
    ainda **não** está travado: o conteúdo do diálogo. A janela provou o de fora — depois do clique em **Exportar pacote**
    o `EnumWindows` do PID lista a janela `#32770` "Exportar pacote" ao lado da `Tauri Window`, e a tela não mostra caminho
    nem toast enquanto a escolha não vem, que é o estado certo — mas nome sugerido, filtro e cancelamento ficam para o
    olho dele, e o cancelamento hoje só tem a leitura do `if (!escolha) return;` da tela, não uma medição.

37. **Um ref de "ainda está montado" tem de ser reafirmado ao montar — os dois lados, não só a limpeza.** A queixa veio
    em print: aba Git, repositório `ssh-master`, `Lendo o repositório…` eterno no cabeçalho e na lista de alterações, sem
    erro e sem toast. Lido do estado da tela, o par `{status === null, busy === "Lendo o repositório"}` só existe se
    `reload()` **resolveu** e mesmo assim não escreveu nada — e é exatamente o que `src/lib/useGitRepo.ts:115` fazia:

    ```ts
    const alive = useRef(true);
    useEffect(() => () => { alive.current = false; }, []);   // como estava
    ```

    O `useEffect` só tem limpeza: depois de desmontar-e-remontar, o ref fica `false` **para sempre**, e como todo
    `guard` do hook termina em `if (alive.current) setBusy(null)` (e `reload` em `if (!snapshot || !alive.current)
    return`), o resultado é o print: as sete leituras rodam, o estado volta, e a tela deita tudo fora — spinner colado,
    nenhum erro, nenhum toast. O que dispara o desmonte-e-remonte é o `React.StrictMode` do `main.tsx:7`, que no
    **modo de desenvolvimento** monta, desmonta e monta de novo; por isso o binário de release, que é onde as nove
    leituras foram medidas na rodada anterior, estava limpo e a janela dele não. A correção é o padrão de duas vias:

    ```ts
    useEffect(() => {
      alive.current = true;
      return () => { alive.current = false; };
    }, []);
    ```

    Duas coisas que esta linha obriga a conferir antes de culpar o git: **o `git` respondeu**, e isso foi medido na
    linha de comando sobre o mesmo repositório e o mesmo estado de disco (`Python/` de 136 MB sem rastrear na raiz) —
    `status --porcelain=v2 -z` **77 ms**, `stash list` **69 ms**, `tag -l` **76 ms**, `remote -v` **66 ms**,
    `log --graph -n 200` **79 ms**; e **o padrão não se repete**: `grep` do `alive` em `src/` dá uma só dona
    (`useGitRepo.ts`), então nenhuma outra tela engolia o próprio fim por esse caminho. De quebra, o mesmo print tinha
    uma segunda mentira de tela, corrigida na causa: o rótulo **limpo** era `dirty > 0 ? … : <Badge>limpo</Badge>`, e
    com `status === null` o `dirty` derivado é zero — a tela afirmava "limpo" de um repositório que não tinha lido
    nada (`GitView.tsx:427` agora só renderiza a etiqueta quando existe status). É a regra 1 da casa, "lista vazia
    aparece como vazia", aplicada ao estado desconhecido: **ausência de leitura não é veredito**.

38. **A máscara corta o valor no texto que chegou — uma cópia em minúsculas não é o texto.** A rodada foi aberta pelo
    pedido de **revisar e validar a própria validação**: em vez de confiar no relatório, olhar as funções que o produzem.
    Lidas, `security::mascarar` tinha três causas, todas compartilhadas com o detetector que barra o commit, e
    `analysis::cauda` tinha a sua: um panic no meio de um acento. Reproduzi as quatro **antes**
    de tocar em qualquer arquivo, com uma sonda que extrai as funções reais de `security.rs`/`analysis.rs`, compila com
    `rustc` e roda (`scripts/sonda-mascara.mjs`, que foi para o `scripts/` justamente para sobreviver a um `cargo clean`); a primeira
leitura dela devolveu

    ```
    mascara passwd: FAIL … mascara unicode_bearer: FAIL      (9 nomes em FAIL, 1 em PASS)
    falhas_reproduzidas=10
    thread 'main' panicked at <anon>:138:26:
    start byte index 1 is not a char boundary; it is inside 'é' (bytes 0..2 of string)
    ```

    As três causas, todas na mesma decisão de conveniência: **(a)** a linha era montada em minúsculas e cortada pelo
    deslocamento **dessa cópia** — em Unicode a caixa muda o número de bytes, então um `İ` derrubava a máscara e a linha
    inteira escapava ilesa; **(b)** `atribuicoes` devolvia **no máximo uma** chave e parava na primeira que não existia na
    linha, então `password = …; secret = …` escondia uma e deixava a outra à vista — para a máscara **e** para o detetector
    que barra o commit, que lê as mesmas funções; **(c)** o valor entre aspas parava no primeiro espaço, mostrando o resto
    de uma frase-senha. E a `cauda` cortava por **byte**, o que matava a varredura antes de qualquer veredito. O que ficou:

    ```rust
    struct Atribuicao { chave: &'static str, inicio: usize, fim: usize }   // bytes do texto original
    ```

    `ocorrencia` compara com `eq_ignore_ascii_case` sem reescrever nada (`security.rs:179`), `atribuicoes` percorre a linha
    sempre pela chave mais à esquerda e continua depois dela (`:296`), o valor entre aspas vai até a aspa que fecha — ou
    até o fim da linha se ela não fecha, porque um valor cortado no meio do arquivo não é motivo para mostrá-lo (`:223`),
    e `intervalos_a_esconder` (`:327`) funde atribuição + `Bearer` + prefixo de token numa passada só sobre o texto
    original, deixando o **prefixo** de fora porque ele diz a espécie e não é segredo (`sk_live_«valor mascarado»`).
    `esta_mascarado` (`:238`) é o que faz mascarar duas vezes escrever **uma** marca. `cauda` (`analysis.rs:1278`) passou a
    cortar e mascarar por caractere. O que **não** é valor continua à vista, e isso é medido, não presumido: comparação
    (`==`), referência de ambiente (`${DB_PASSWORD}`), nome sem valor e valor curto de seis caracteres ou menos.
    Relida agora, sobre o código deste commit: **24 PASS** (21 linhas × escondido/à-vista/idempotente + 3 da cauda),
    `falhas_reproduzidas=0`. As defesas que ficaram no arquivo são sete testes novos
    (`toda_chave_de_segredo_tem_o_valor_escondido`, `uma_linha_pode_ter_mais_de_um_valor`,
    `o_valor_entre_aspas_some_inteiro`, `a_mascara_nao_desiste_diante_de_unicode`,
    `o_prefixo_de_token_continua_dizendo_a_especie`, `o_que_nao_e_valor_fica_a_vista`, `a_mascara_e_idempotente`), e a
    sonda é uma reprodução **fora do app**: ela prova a mecânica, nunca a tela.

39. **Campo ausente não é campo vazio — um veredito não pode inferir leitura.** A mesma revisão olhou o outro lado do
    relatório: o que acontece quando o instrumento responde *qualquer coisa*. Estava assim: `results` que não vinha no
    JSON era lido como "nenhum achado" — exatamente a conclusão que **libera** o pipeline. Hoje as três listas que
    sustentam a prova são obrigatórias (`analysis.rs:1129`–`:1141`): sem `results`, sem `errors` ou sem `paths.scanned` a
    resposta é dita ilegível; um achado sem regra, sem caminho ou sem linha conta como **leitura que não se leu**
    (`sem_forma`, que marca `invalida`) e não como achado que não existia; e o fechamento passou a ter três motivos para
    dizer `falhou` em vez de `concluida` — o instrumento pedido que não rodou (`externo_sem_prova`, `:1472`), a resposta
    ilegível e a varredura que abriu zero arquivo (`sem_leitura`, `:1508`); neste último caso a nota é dita com todas as
    letras: "a varredura não leu arquivo nenhum — não há o que afirmar sobre o código".
    É a regra 1 da casa aplicada ao veredito de segurança, a mesma do item 37.

    Dois números que antes se confundiam foram separados: **o que barra é a contagem do corte, não a total**
    (`barra_o_corte`/`achados_no_corte`, `:1566`–`:1579`) — com corte `ERROR` e os counts lidos da corrida **#22**
    (676/42/640) o número dito é **676**, e "barram 1.358" seria inventar o resto; e **cobertura parcial não é ausência de
    prova**, é prova de menos: com corte declarado ela barra, sem corte o nó informa e a nota diz o que ficou de fora
    (`:1520`). O cancelamento, por fim, voltou a ser cancelamento: `run.status == "interrompida"` devolve
    `Fail::cancelled()` (`deploy.rs:1694`) em vez de um sucesso com metade da árvore lida, que era o que deixava os
    dependentes publicarem; travado por `uma_varredura_interrompida_e_cancelamento_do_no`.

    O formato do envelope não é presumido — é o que a ferramenta desta máquina (semgrep **1.181.0**) escreveu, medido duas
    vezes: **fora** de qualquer repositório Git, sobre um `a.py` que as regras flagram, o comando devolve
    `{"results":[],"errors":[],"paths":{"scanned":[]}}` (zero lidos — o caso que hoje fecha em `falhou`); **depois** de
    `git init` + `git add` + commit, o mesmo comando devolve `codigo=0`, `results 2`, `errors 0`, `scanned ["a.py"]` e
    **nenhuma** chave `skipped`, com `check_id` pontuado e `extra.lines` escrito `"requires login"` — por isso a linha é
    relida por nós, e o teste `o_envelope_real_medido_e_lido_com_prova` guarda essa forma. Uma frase da mensagem do
    `c2c1877` ("só varre o que o Git rastrea") **não** é sustentada pela terceira medição, feita no diretório do projeto:
    lá a lista de lidos veio com arquivos sob `Python/`, que tem **0** arquivos rastreados (`git ls-files Python/`) contra
    154 no total; o que as duas medições sustentam é que `scanned: []` é um estado real do instrumento fora de um
    repositório, e o que ele varre dentro de um continua sendo a pendência 14 do §6. Commit local, não empurrado — a
    correção fica registrada aqui em vez de reescrever a mensagem.

## 6. Pendências assumidas (com o primeiro passo de cada)

| # | Pendência | Primeiro passo concreto |
| --- | --- | --- |
| 0 | **Dados do piloto das etapas 2, 3, 4, 5 e 6, ainda cadastrados de propósito** | O projeto **Piloto Etapa 2** (`%TEMP%\piloto-etapa2`) com a definição *Piloto sem servidor* **v22** e as execuções **#23–#54** (32 no total, lidas em `deploy_runs` nesta rodada) continuam no cadastro para ele abrir a janela e ver as telas conferidas — a mais nova, **#54**, é o perfil 22 em `failed` com `o grafo parou: existe nó com dependência que não se completa`, `definition_version 2`, `commit_sha` nulo e **duas** linhas gravadas no bloco `(run 54, posição 1, tentativa 1)`: `+ análise de código é embutido — corte ERROR` e `3 arquivo(s) lido(s) — 14 achado(s): 9 erro, 4 aviso, 1 info` — a prova de que o grafo para **e** o que a etapa imprimiu fica no banco; a v22 é o grafo de oito nós lido hoje por `pipeline_nodes` e impresso por `pipeline_validate` (`validar, git, coleta, preparo, publicar, gate, copia, historico` · `motor 6 · comando 1 · aprovação 1 · git 0 · análise 0`, com `preparo` de volta a `echo preparo` e `secret_keys` **[]**), a #34 é a corrida em que `preparo` falha e trava o resto, a #38/#39 são as duas execuções de nó sozinho lidas no binário final (`preparo` → "1 comando(s) nesta máquina", `gate` → "aprovado na janela"), a **#42/#43** são as duas execuções do nó git (uma `tag` que criou `v0.0.1-janela` de verdade no piloto e uma `checkout main` que respondeu "em main"), a **#44/#45/#46** são as três execuções do nó `analise` pelo mesmo motor (falha por ferramenta ausente, sucesso com o semgrep 1.181.0, sucesso com o embutido) e a **#49 a #53** são as cinco da etapa 6: **#49** guarda o `%CHAVE_PILOTO%` **literal** (a evidência crua do defeito que a corrigiu), **#50** guarda `envio ***`, **#51** é o nó com `retry_limit` 1 que falhou duas vezes e deixou **dois blocos**, **#52** é o laço de 2.500 ecos com o corte declarado na última linha, e **#53** é o `preparo` rodado sozinho depois do reopen — as cinco continuam no banco porque é nelas que a tela **Saída da etapa** se confere. O projeto **Piloto Etapa 5** (`%TEMP%\piloto-etapa5`, com `app.py`, `src/legado.php` e `src/servidor.js` — três arquivos,
que é o `filesScanned` lido das corridas dele) é o que tem código varrerável, e desde a medição do gate ele **tem**
modelo de deploy: perfil 22 *Gate de análise*, destino `%TEMP%\piloto-etapa5-publicado` (que ficou inexistente de
propósito — a publicação foi barrada antes de copiar), definição **v2** com o grafo de dois nós `varredura` →
`depois-do-gate`, e as execuções **#47** e **#48**, as duas `failed` pelo corte `ERROR`. As corridas de análise do
cadastro, relidas às 00:0x numa **cópia** do banco (o original intocado), vão até a **#22**: no projeto 4 existem
**#2, #3, #4, #6, #11, #12, #13, #16 e #19** (9, a mais nova a #19), e a **#6** é a das 12 linhas
achadas com o semgrep real e o token mascarado, enquanto **#11/#12** são as duas do nó `analise` que barrou a
publicação (9 erros, 4 avisos, 1 info). No projeto 2 (**`E:\ssh-master`**) há **8** corridas, e **#21** e **#22** — as
duas últimas, `concluida` pelo embutido, 2.306 lidos / 167 pulados, counts 676/42/640, `blocked 1` com corte `ERROR` e
**1.358** linhas de achado cada uma — são as que sustentam o número citado no item 39 do §5. O que a etapa 4 criou para medir **foi desfeito e conferido**: o servidor temporário foi apagado (`list_servers` → 0), o modelo *medição docker* 21 foi removido pela confirmação da tela, a etiqueta `v0.0.1-janela` saiu do repositório do piloto (`git tag -l` → 0) e o grafo voltou aos oito nós — **e o mesmo
foi feito na etapa 6**: os três nós que serviram de andaime para medir (`repete`, com `retry_limit` 1; `segredo`, com
`echo envio %CHAVE_PILOTO%`; `muito`, com o laço de 2.500 ecos) foram removidos por `pipeline_save_nodes` e a leitura de
volta dá as **oito** chaves de sempre, com `secret_keys` voltando a `[]` por `save_deploy_profile`. As corridas **#49 a
#53 ficam** de propósito: são a única prova fora de teste unitário de que a linha mascarada sobrevive no banco. Quando ele decidir, o primeiro passo é **Projetos → Piloto Etapa 2 → Remover do cadastro** (e o mesmo para o Piloto Etapa 5; a confirmação avisa quantos modelos saem junto; nenhum arquivo local é apagado) e depois apagar as duas pastas em `%TEMP%`. O `analise.semgrep.caminho` salvo hoje aponta para `%TEMP%\semgrep-probe\Scripts\semgrep.exe` — se a pasta cair, a sonda passa a dizer `ausente` e o instrumento embutido continua funcionando. O arquivo de teste de 1,5 GB já foi removido do projeto e do destino |
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
confere. **Acrescentado na etapa 6:** falta o olho dele em **Deploy → Deploys registrados → uma corrida do Piloto →
Saída da etapa** — abrir a **#51** para ver os **dois blocos** (`tentativa 1 · 3 linha(s)` em cima, `tentativa 2 · 2
linha(s)` embaixo, com o aviso de repetição no bloco que falhou), a **#52** para ver o painel cheio e buscar `truncado`
até a linha do corte aparecer, e a **#48** para ver o que uma corrida anterior à migração diz: `nenhuma linha gravada
nesta etapa`. Eu li os três estados na janela real (§2), mas "os blocos por tentativa são legíveis como duas passadas
distintas" é julgamento dele, não meu |
| 7 | **Ver o motor *bloquear* uma publicação pelo corte da análise, na janela — FECHADA em 2026-10-15** | Estava listada porque a regra só tinha prova de teste unitário. Foi medida na janela real: perfil 22 *Gate de análise* no `piloto-etapa5`, grafo `varredura` (nó `analise`, corte `ERROR`) → `depois-do-gate`, execuções **#47** e **#48** `failed` com a contagem 9/4/1 citada na linha do nó, o nó seguinte `skipped` e a pasta de destino **nunca criada** — tudo lido em `deploy_run`/`analysis_corridas`/`analysis_pacote`, linha por linha no §2. O que ela revelou de quebra (o prefixo `\\?\` na mensagem e no pacote) virou o §5, item 27, com teste novo |
| 6 | **Revalidar o deploy por SSH num host real — e pela primeira vez com um executor de runtime** | Além de `build_step`/`identify_step`, a etapa 2 mexeu no caminho com servidor: `validate_remoto` agora chama `deploy::checar_definicao(..., Some(&servidor))` e `insert_run` passou a receber a procedência. As checagens remotas foram movidas verbatim e `server_ref()` continua sendo a primeira delas, então por construção o remoto não muda — mas "por construção" sem medir num host é presunção. **A etapa 4 amplia essa pendência em vez de criar outra:** Docker, Compose e PM2 só rodo no servidor, e o que foi conferido aqui é o roteiro gerado e a recusa da tela, nunca um container subindo de verdade. Primeiro passo: cadastrar um servidor, repetir C/D com `server_id` preenchido conferindo que a etapa 1 recusa o que recusava antes, e então publicar **uma** aplicação real com `docker` (o caminho completo: build da imagem no servidor, container novo no ar, `previous_image` gravada, reversão voltando a imagem anterior) e, se houver, uma com `pm2`. Nesta máquina isso não tem como: o `docker` 29.3.1 existe mas a daemon do Docker Desktop está parada, e `pm2` não está instalado |
| 8 | **Conferir o prazo contra o disco externo de verdade — na máquina dele** | A correção está medida por teste (a espera abandonada, os três estados de cada leitura) e por janela nesta máquina, mas **sem volume externo para consultar**: `Get-PhysicalDisk` devolve três SSDs internos (`KINGSTON SNV3S1000G` 1.000.204.886.016 B, `SA400S37240G` 240.057.409.536 B, `SA400S37120G` 120.033.041.920 B) e o campo `Bus` volta vazio, então o que a janela provou aqui foi "toda leitura responde, e responde com motivo" — inclusive as duas pastas mortas (`Z:\repo-externo` em 3 ms e `\\nao-existe-devops\repo` em 1.273 ms), que o Windows recusa antes de qualquer prazo. Falta o caso real: abrir a pasta num externo **adormecido** e ler a frase que volta — `a pasta F:\… não respondeu em 20 s — o disco pode estar adormecido, desconectado ou com a letra trocada` para o caminho, ou `o git não respondeu em 30 s e foi interrompido — … ou um disco externo que ainda não acordou` para a leitura do git. Primeiro passo: cadastrar o projeto no externo, abrir **Git**, e dizer qual das duas frases apareceu (isso diz se o que estava preso era o `stat` do caminho ou o `git show` do blob) |
| 9 | **Os `canonicalize()` nus fora da aba Git — FECHADA em 2026-10-21** | A linha como estava escrita tinha dois erros, e fica registrada assim para a próxima pessoa não procurar por eles: não eram **três** chamadas em **dois** lugares, eram **seis** em **três** funções — `analysis::executar` (a raiz **e** o alvo, hoje atrás de `alvo_da_varredura`, `analysis.rs:1293`, chamada em `:1338` com `git_probe::PRAZO_SONDA`), `deploy::dir_do_no` → `dir_local` (`deploy.rs:1414`/`:1422`; a linha citava `publicar_local`, que é outra função e não tem essas linhas) e `deploy::checar_definicao` (a origem **e** o destino da contenção, `deploy.rs:2234` e o ponto de chamada `:2310` — estas duas a linha nem contava). As seis passam por `git_probe::canon`/`canon_ou_ancestral` com os três estados, e o que se leu na janela está no §2: varredura de subpasta em 12 ms com a raiz devolvida sem `\\?\`, subpasta inexistente recusada em 2 ms como **ausência**, as duas definições do piloto passando em 3 ms e 2 ms, e a contenção disparando pelos dois motivos certos nos três casos (dentro ainda não criada, raiz, fora). O efeito colateral desta linha é a maior dela: a guarda `alvo.starts_with(&origem)` comparava origem canônica contra destino **digitado** e nunca disparava para o destino da primeira publicação — corrigido e §5, item 35 |
| 10 | **`ssh-add -l` sem prazo — FECHADA em 2026-10-21, com prazo diferente do que a linha propunha** | `ssh_config::read_agent` (`ssh_config.rs:398`) não faz mais `.output()`: faz `spawn()` por `crate::proc::command` e colhe por `sshexec::collect` com `PRAZO_AGENTE` (`ssh_config.rs:367`), e `veredito_agente` (`:404`) separa os quatro estados — ausente do PATH, **não respondeu em 5 s**, vivo com chaves, vivo sem chaves. O `Err(_) =>` que escrevia "ssh-add não encontrado no PATH" para qualquer falha saiu, e é isso que a linha 34 do §5 proíbe. A proposta pedia **2 s**; ficou **5 s**, o mesmo `PRAZO` de `tools.rs`, para a casa ter um número só de sonda de processo. Lido na janela: `discover_ssh` **211 ms** com o veredito do código 2 (`"SSH Agent não está em execução (ou não há socket disponível)."`); lido na linha de comando: serviço `ssh-agent` **STOPPED**, `ssh-add -l` do Git devolve **2 em 49 ms** e o de `System32` **2 em 39 ms**. O agent preso de verdade continua sem prova nesta máquina (não há um aqui para prender), e quem prova o abandono é `uma_sonda_de_agente_que_nao_termina_e_abandonada_dentro_do_prazo` com um `ping -n 40` real |
| 11 | **34 chamadas `.is_dir()`/`.exists()` ainda sem prazo, em 8 arquivos** | Contagem lida agora, com o comando na mesa: `grep -c "\.is_dir()\|\.exists()" src-tauri/src/*.rs` → `deploy.rs` 12, `git.rs` 11, `ssh_config.rs` 3, `analysis.rs` 2, `auth.rs` 2, `security.rs` 2, `git_probe.rs` 1, `pipeline.rs` 1, **34** no total — número que **inclui** as linhas dentro de `#[cfg(test)]`, então é teto do trabalho, não piso. As duas que pesam são de produção: `pipeline.rs:445`, o `is_dir()` da raiz dentro de `pipeline_validate`, que roda **antes** das duas leituras com prazo da mesma função (um externo adormecido trava ali, sem prazo nenhum), e `analysis.rs:944`, o `read_dir` da varredura embutida, que é a linha 13 abaixo. Primeiro passo: essas duas irem pela `git_probe::com_prazo`/`canon` que já existem e então recomeçar a contagem separando teste de produção, arquivo por arquivo — sem chamar a lista inteira de verde antes de ler cada linha |
| 12 | **`pipeline_validate` roda na thread assíncrona, sem `spawn_blocking`** | `commands.rs:768` é `async fn` e chama `pipeline::validate` direto. A validação agora abre dois caminhos por `com_prazo`, e a espera dela é um `recv_timeout` que **para um worker do runtime assíncrono** — 20 s presos ali não são 20 s só daquele comando. As leituras de Git saíram dessa armadilha na rodada anterior (`git_read`/`git_write` resolvem dentro do `spawn_blocking`); esta ficou. Primeiro passo: envolver o corpo de `pipeline_validate` em `spawn_blocking` como os vizinhos e conferir na janela que a chamada continua devolvendo os **3 ms / 2 ms** medidos no §2, desta vez com o `is_dir()` da linha 11 também contido |
| 13 | **`percorrer`, a varredura do analisador embutido, não tem prazo nem cancelamento dentro do laço** | `analysis.rs:936`–`:960`: `std::fs::read_dir(pasta)` sem fim, a cada pasta, e o `cancel: &AtomicBool` que `analysis_interromper` levanta **não é consultado dentro do laço** (a assinatura de `percorrer` não o recebe); o único corte é `LIMITE_ARQUIVOS`. Um diretório que o volume não devolve segura a corrida em "em andamento" para sempre, porque o cancelamento cooperativo não alcança uma chamada presa no kernel — exatamente o motivo da linha 8. Primeiro passo: receber o `cancel` e conferi-lo a cada pasta, e envolver o `read_dir` pela espera com prazo, devolvendo veredito próprio ("a varredura parou: `…` não respondeu em N s") que **não** é lido como varredura limpa nem como cobertura zero |
| 14 | **O escopo da varredura: nada na tela diz o que ela deve ignorar, e numa pasta com distribuição Python dentro ela varre a distribuição** | Achado lido nesta rodada, a partir do pacote que ele exportou da corrida **#18** e do disco: `filesScanned` **2.306** (e **2.305** na #17, que rodou com instrumento `ambos`) contra **154** entradas de `git_tree` no projeto 2 e **0** arquivos `.py` rastreados — a `Python\` da raiz, a distribuição CPython portável de **136 MB**, é o grosso da varredura, e é ela que empurra a contagem para **676 erros / 42 avisos / 640 infos**, igual nas cinco corridas lidas (`#9`, `#10`, `#15`, `#17`, `#18`), todas com veredito `barra`. Não é um defeito do motor: `security::pular_dir` (`security.rs:86`) é a dona única da regra de "não é código do projeto" e lista `.git` + 12 nomes (`node_modules`, `target`, `dist`, `build`, `vendor`, `__pycache__`, `.venv`, `venv`, `.next`, `.cache`, `coverage`, `.terraform`) — `Python` não está entre eles, e `git check-ignore Python/` devolve código **1**, então ela não é ignorada, só nunca foi rastreada. O que falta é **porta**, não regra: não há nenhum ajuste que chegue a `analysis::percorrer` para dizer o que ficar de fora, e a pessoa hoje só tem o campo **Pasta no projeto** (subpasta), que estreita por dentro em vez de excluir. Primeiro passo: um `analise.ignorar` por projeto (lista de pastas/arquivos) **alimentando a mesma `pular_dir`**, não uma segunda lista dentro da análise — a casa tem uma mecânica por responsabilidade, e as duas portas (tela + nó do grafo) têm de concordar com o que está fora. Duas coisas não foram presumidas e continuam em aberto: **não medi na porta do semgrep**, e a lista fixa **não** é compartilhada com ela como eu escrevi nesta linha na primeira versão — lida de novo, `security::pular_dir` é consultada pelo laço embutido (`analysis.rs:953`) e pela checagem de segredo (`security.rs:425`), enquanto `varredura_semgrep` (`analysis.rs:1072`–`:1080`) monta `scan --metrics=off --json-output … --config … .` e **não passa `--exclude` nenhum**, ficando o que for ignorado por conta dos padrões da própria ferramenta; se um `analise.ignorar` entrar, ele tem de virar as duas coisas (a lista da `pular_dir` **e** o `--exclude` do semgrep) porque é uma mecânica com duas portas, e o primeiro passo é conferir o que o semgrep 1.181 já pula sozinho antes de somar qualquer coisa. E o número que faltava saiu do pacote que ele exportou da corrida **#21** (`dist\analise-corrida-21.json`, lido campo a campo): cobertura **2.306 arquivos lidos**, **167 pulados**, **1.358 achados guardados**, veredito `barra` com 676/42/640 — e dos **300** achados que o pacote carrega (`truncado true`, o teto do pacote), **300** estão sob `Python/`; as **4** regras que aparecem somam 300 e **nenhum** exemplo de arquivo é do código do projeto. O que isso **não** diz: os outros **1.058** achados não vêm no pacote e só estão no banco, então "quanto do projeto próprio acha" continua sem número — e o caminho barato para obtê-lo hoje é o que já existe: rodar a varredura com **Pasta no projeto** = `src` (ou `src-tauri`), que estreita por dentro em vez de excluir. Alternativa descartada de propósito: acrescentar `python` em `NOISE_DIRS`, porque ignoraria a pasta de quem tem Python **como** projeto |

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
