# Registro de entregas — onde está cada coisa, o que falta e como retomar

Documento de retomada. Ele não substitui o [README](../README.md) (uso do aplicativo) nem o
[SPEC.md](SPEC.md) (o que foi pedido); ele registra **o que já está pronto, onde está o código, como se prova
cada parte e o que ainda não existe**. Escrito em 2026-10-10 sobre o commit `a510b5e`.

## 1. Retomar em três passos

```bash
node ./node_modules/typescript/bin/tsc --noEmit     # tipos do frontend
npm run build                                        # tsc + bundle
(cd src-tauri && cargo test)                         # 184 testes de backend
(cd src-tauri && cargo clippy --all-targets -- -D warnings)
node scripts/confira-repo.ts                         # o comparador remoto↔pasta, 9 formas reais de URL
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
| `npm run build` | `✓ built in 3.32s`; 81 módulos; único aviso é o de chunk > 500 kB (bundle único de ~833 kB) |
| `cargo test` (todos os alvos) | `184 passed; 0 failed; 0 ignored` em `devops_desktop_lib`; `main.rs` e doc-tests com 0 testes |
| `cargo clippy --all-targets -- -D warnings` | `Finished` sem aviso (com `-D warnings`, qualquer aviso derrubaria o comando) |
| `node scripts/confira-repo.ts` | `todas as 9 formas conferem`, código 0 |
| Executando | `devops-desktop.exe` (PID 3840) com `MainWindowTitle = "DevOps Desktop - Central de SSH, Git e Deploy"`, `Responding = True` |
| Git | `main` em sincronia com `origin/main`; público em <https://github.com/DouglasGDev/devops-desktop> |

O que isso **não** cobre: não há runner de teste de interface. As telas são conferidas por quem usa — a suíte automática
é de backend (git/ssh **reais** do sistema, sem mock) e os tipos do frontend.

## 3. Etapas do SPEC §16 — entregues, com o código e a prova

| Etapa | O que funciona | Onde | Como ver na tela |
| --- | --- | --- | --- |
| 1. Estrutura desktop, UI, persistência | shell com 10 telas, tema claro/escuro, SQLite local com migrações, auditoria, busca global | `src/App.tsx`, `src/store/AppContext.tsx`, `src-tauri/src/{db,models,audit}.rs` | **Topbar** (busca) e **Histórico** (o que foi gravado) |
| 2. Descoberta dos acessos SSH | lê `~/.ssh/config` e `known_hosts` sem mover nem copiar chave | `src-tauri/src/ssh_config.rs` | **Servidores → Descobrir** |
| 3. Teste de conexão e gerenciamento | diagnóstico por camada (DNS, porta, autenticação, host key), cadastro/edição/exclusão, ambiente por servidor | `src-tauri/src/ssh_test.rs`, `auth.rs` | **Servidores → Testar** |
| 4. Terminal SSH | PTY múltiplo, redimensionamento, histórico de comandos local, rotinas salvas | `src-tauri/src/terminal.rs`, comandos `*_terminal_session`, `*_routine` | **Terminal** |
| 5. Gerenciador de arquivos remoto | listar, criar pasta, renomear, chmod, ler/editar texto, subir/descarregar arquivo e pasta com progresso e cancelamento | `src-tauri/src/files.rs`, `files_transfer.rs` | **Arquivos** |
| 6. Git completo | status, stage por arquivo e por hunk, diff linha a linha, commit/amend, gráfico, árvore rastreada + conteúdo do HEAD, ramos, tags, remotos, merge/rebase com conflito por lado, revert/reset, fetch/pull/push, clone, abrir no editor | `src-tauri/src/git.rs`, `src/views/GitView.tsx`, `src/components/git/*` | **Git** (abas Alterações, Árvore, Histórico, Ramos, Tags, Remotos, Segredos) |
| 7. Projetos e perfis de deploy | cadastro de pasta com sondagem real de git, perfil por projeto/estratégia | `src-tauri/src/git_probe.rs`, `deploy.rs`, `commands.rs` (`*_deploy_profile`) | **Projetos**, **Deploy** |
| 8. Motor de deploy com pipeline visual | estratégias A–E, preflight de ferramentas no servidor, etapas com comando/duração/código de saída, cancelamento cooperativo, rollback | `src-tauri/src/deploy.rs`, `sshexec.rs` | **Deploy** |
| 9. Segredos e proteção de commit | varredura do conteúdo **que o Git publicaria** (`git show :path` / `HEAD:path`), bloqueio antes de commit e push, `git rm --cached` assistido, regras extras | `src-tauri/src/security.rs` | **Git → Segredos**; o bloqueio aparece ao commitar/pushar |
| 10. Dashboard, histórico, refinamentos | métricas reais lidas do servidor quando você pedir, deploys recentes, falhas, transferências, alertas de configuração | `src-tauri/src/infra.rs`, `src/components/InfraPanel.tsx` | **Painel** |

## 4. Frentes que foram além do SPEC (o "canivete")

- **GitHub real** (`src-tauri/src/github.rs`, `src/views/GitHubView.tsx`, `src/components/github/*`): fluxo de dispositivo
  OAuth com token **só na memória**, bandeja, PR com diff real pelo mesmo parser do diff local, revisão, mesclar com
  método escolhido, conversa de comentários, issues, execuções do Actions com reexecução, abrir PR de um ramo local,
  buscar o ramo de um PR (`refs/pull/<n>/head`), criar repositório.
- **Publicar sem servidor e sem conta** (`git_publish`): roda os comandos locais do projeto, `git push` do ramo e tag, com
  checagem de segredo antes; **Despublicar** uma tag; **Testar acesso** com `git ls-remote` e prazo de 25 s.
- **Aba Local no GitHub**: a seção Git inteira (`GitView`) embutida sobre a pasta mapeada do repositório — mesmos painéis,
  mesmos comandos, sem duplicar código nem botões enfeitados.
- **Mapear pasta local** (`src/lib/useMapearPasta.ts`, `src/lib/repo.ts`): aponta o clone que já existe no disco, compara
  `dono/repo` do que o git respondeu (host-agnóstico) e mostra o caminho exato + **Mostrar a pasta** no Explorador.
- **Máquina** (`src-tauri/src/tools.rs`, `src/components/MachineAccess.tsx`): detecta `ssh`/`git`/`gh`/`ssh-add`,
  identidade do git, chaves no agente, editores no disco, e diz se esta máquina já está liberada para publicar.
- **Senha por prompt, nunca salva** (`src-tauri/src/auth.rs`, `PasswordPrompt.tsx`): `SSH_ASKPASS` por operação.

Total da superfície: **118 comandos Tauri registrados**, 21 arquivos Rust em `src-tauri/src`, 10 views,
30.302 linhas de TS/TSX/Rust.

Persistência (SQLite local, migrações em `src-tauri/src/db.rs`) — as nove tabelas que existem hoje:
`settings`, `servers`, `projects`, `audit_log`, `routines`, `terminal_history`, `deploy_profiles`, `deploy_runs`,
`deploy_run_steps`. Não há tabela de série histórica de métrica: o painel lê do servidor na hora pedida.

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

## 6. Pendências assumidas (com o primeiro passo de cada)

| # | Pendência | Primeiro passo concreto |
| --- | --- | --- |
| 1 | **Canivete 2 — deploy com perfil sem servidor** (tarefa de plano nº 16) | `deploy_profiles.server_id` ainda é obrigatório. Tornar a coluna anulável em migração, `Option<i64>` em `models.rs`/`deploy.rs`, `LEFT JOIN servers`, e na tela permitir perfil puramente local (comandos + artefatos sem SSH) com rollback por perfil |
| 2 | **Client ID na máquina do Douglas** | Colar o **Client ID** da OAuth App (o campo está em **GitHub → Configuração**) e gerar um secret novo no GitHub — o valor hexadecimal que estava salvo no banco era o **secret**, não o ID. Sem isso a aba GitHub não conecta: é limite de conta, não do código |
| 3 | **Passada de visual (prioridade quando ele chamar)** | Só depois das funções; começar pelo print da tela que incomoda. Não refazer estilo por conta própria junto de feature |
| 4 | **Pacotes Linux** | Os alvos `deb`/`rpm`/`AppImage` nunca foram gerados aqui. Rodar `npm run tauri build` numa imagem Linux e ajustar `bundle.targets` + paths (o código já usa `dirs`/caminhos relativos, mas isso precisa ser provado, não suposto) |
| 5 | **Teste de interface automatizado** | Não há runner hoje. Se um dia entrar, o mínimo é o fluxo do SPEC §17; por ora, a cobertura real é `tsc` + suíte Rust + `scripts/confira-repo.ts` |
| 6 | **Aceite visual das últimas entregas** | `Git → Árvore` e `GitHub → repositório → Local` ainda não foram vistos por ele nesta máquina |

## 7. Lacunas honestas do "canivete suíço" (ainda NÃO existe)

Listadas para escolher a próxima frente — nada aqui está implementado, e nada aqui foi inventado como "quase pronto":

**Git** — `stash` (guardar trabalho em andamento), `cherry-pick`, `bisect`, `blame` por linha, busca no histórico por
autoria/texto, `submodule`, `worktree` múltiplo por projeto, assinatura de commit (`gpg.sign`), `git fsck`/manutenção.

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
