# PROJETO: DevOps Desktop — Central de SSH, Git e Deploy

## 1. Objetivo do projeto

Desenvolva uma aplicação desktop profissional, moderna, rápida e segura para administrar servidores remotos via SSH, transferir arquivos, executar comandos, gerenciar repositórios Git e automatizar deploys completos, tudo por uma interface gráfica.

A aplicação deve eliminar a necessidade de acessar manualmente aaPanel, Coolify, terminais separados ou interfaces web de administração para tarefas rotineiras de publicação e manutenção.

O objetivo é transformar operações repetitivas em fluxos automatizados, reutilizáveis e executáveis com poucos cliques.

Não quero apenas um terminal com interface gráfica. Quero uma plataforma desktop de administração de infraestrutura e publicação de software, com recursos de DevOps, gerenciamento de arquivos, automação e Git.

## 2. Tecnologias obrigatórias

Utilize uma arquitetura moderna, adequada para uma aplicação desktop profissional.

- **Rust:** núcleo da aplicação, operações de sistema, processos, SSH, transferência, automações e segurança.
- **Tauri 2:** estrutura desktop multiplataforma, com comunicação segura entre interface e backend.
- **React + TypeScript:** interface gráfica moderna, componentes reutilizáveis e tipagem rigorosa.
- **Vite:** ambiente de desenvolvimento e compilação da interface.
- **Git nativo:** integração com o Git instalado no computador, preservando compatibilidade com os repositórios existentes.
- **OpenSSH:** suporte às configurações, chaves, agentes e mecanismos de autenticação já existentes no sistema.
- **SQLite:** armazenamento local de projetos, servidores cadastrados, preferências, históricos e configurações não secretas.

Avalie bibliotecas Rust confiáveis para SSH e SFTP. Utilize OpenSSH como alternativa quando isso proporcionar maior compatibilidade com as configurações locais.

O projeto deve ter separação clara entre interface, serviços, domínio, infraestrutura e persistência.

Não implemente tudo em um arquivo único. Não utilize Electron se Tauri atender aos requisitos. Não simule funcionalidades com dados falsos.

## 3. Descoberta e gerenciamento de acessos SSH

A aplicação deve aproveitar os acessos SSH que já existem no computador.

Detecte automaticamente, com autorização do usuário, as configurações convencionais do OpenSSH, incluindo:

- `~/.ssh/config`
- `~/.ssh/known_hosts`
- Chaves privadas e públicas presentes nos locais configurados.
- SSH Agent e chaves já carregadas.
- Aliases, usuários, portas, hosts e opções de conexão definidos na configuração SSH.
- Arquivos de configuração adicionais explicitamente indicados pelo usuário.

Permita selecionar uma pasta personalizada que contenha configurações e arquivos de acesso SSH.

A aplicação deve ler e interpretar as configurações existentes sem mover, modificar ou copiar as chaves originais.

Apresente os servidores encontrados em uma lista visual, mostrando nome, endereço, porta, usuário, método de autenticação e estado da conexão.

Ofereça as seguintes funcionalidades:

- Importar e cadastrar conexões SSH.
- Detectar configurações existentes automaticamente.
- Testar conexão com um clique.
- Identificar falhas de autenticação, DNS, porta, permissões e timeout.
- Abrir sessões SSH interativas.
- Manter conexões persistentes quando apropriado.
- Suportar múltiplos servidores simultaneamente.
- Editar aliases, portas e configurações sem danificar o arquivo original.
- Exportar configurações de servidores sem exportar segredos.
- Agrupar servidores por ambiente, empresa, projeto ou finalidade.

Não exija recadastrar manualmente todos os servidores já configurados no computador.

## 4. Segurança dos acessos e segredos

Segurança é um requisito fundamental.

Nunca grave senhas, tokens, chaves privadas ou outros segredos em arquivos versionados, no SQLite sem proteção, nos logs ou na configuração pública do aplicativo.

Requisitos:

- Não copiar chaves privadas para o projeto.
- Não modificar ou substituir chaves SSH existentes.
- Preferir SSH Agent, mecanismos de credenciais do sistema operacional e gerenciadores de segredos nativos.
- Se for necessário armazenar uma senha, utilizar o armazenamento seguro do sistema operacional.
- Nunca imprimir senhas, tokens ou chaves nos logs.
- Mascarar dados sensíveis na interface.
- Nunca desabilitar automaticamente a verificação de identidade do servidor.
- Validar a chave de host com `known_hosts` ou solicitar confirmação explícita quando o host for desconhecido.
- Tratar alterações inesperadas de identidade do servidor como um possível risco de segurança.
- Solicitar confirmação antes de executar operações potencialmente destrutivas.

A aplicação deve identificar possíveis segredos dentro dos diretórios selecionados e alertar o usuário antes de qualquer operação de Git que possa incluí-los.

## 5. Gerenciador de arquivos remoto

Implemente um explorador de arquivos remoto semelhante a um gerenciador de arquivos moderno.

A interface deve apresentar:

- Navegação por diretórios remotos.
- Caminho atual e histórico de navegação.
- Visualização de arquivos, pastas, tamanho, permissões e datas.
- Upload de arquivos e diretórios do computador para o servidor.
- Download de arquivos e diretórios do servidor.
- Arrastar e soltar arquivos.
- Upload de múltiplos arquivos.
- Barra de progresso em tempo real.
- Velocidade de transferência, tamanho total e tempo estimado.
- Cancelamento e tratamento de falhas.
- Retomada de transferências quando tecnicamente suportada.
- Renomeação, criação de pastas e exclusão de arquivos.
- Edição de arquivos de texto remotos, com salvamento controlado.
- Comparação entre arquivos locais e remotos.
- Permissões Unix e proprietário, quando houver autorização.
- Sincronização entre diretórios locais e remotos.

Permita configurar exclusões de arquivos e pastas, como `.git`, `node_modules`, `.env`, caches, builds desnecessários e arquivos temporários.

As exclusões devem ser configuráveis por projeto. Não exclua automaticamente arquivos necessários à aplicação.

Para grandes transferências, implemente processamento em lotes, controle de concorrência e tratamento de interrupções.

## 6. Terminal SSH integrado

Crie um terminal profissional integrado à aplicação.

Ele deve permitir:

- Executar comandos interativamente no servidor remoto.
- Manter sessões com suporte a PTY.
- Abrir múltiplas abas de terminal.
- Executar comandos longos sem bloquear a interface.
- Exibir saída em tempo real.
- Preservar códigos de saída dos processos.
- Suportar interrupção de processos.
- Copiar e colar texto.
- Redimensionar o terminal.
- Manter histórico de comandos local, com opção de desativação.
- Diferenciar claramente o servidor conectado e o diretório atual.
- Exibir o estado da conexão.
- Reconectar quando apropriado, sem repetir comandos automaticamente.

Implemente um executor de comandos que permita salvar rotinas reutilizáveis, com parâmetros configuráveis.

Exemplos de rotinas:

- Verificar uso de CPU, RAM e disco.
- Verificar processos e serviços.
- Consultar logs.
- Verificar portas em uso.
- Reiniciar serviços autorizados.
- Executar comandos Docker.
- Consultar containers e imagens.
- Verificar status de aplicações.
- Consultar espaço em disco e arquivos grandes.

A interface deve diferenciar comandos de leitura de comandos que alteram o estado do servidor. Comandos destrutivos devem exigir confirmação.

## 7. Gerenciador Git completo

Implemente uma interface gráfica para o Git, integrada aos repositórios locais.

A aplicação deve permitir:

- Detectar repositórios Git existentes.
- Adicionar projetos a partir de pastas locais.
- Clonar repositórios.
- Visualizar branches locais e remotas.
- Criar, renomear e alternar branches.
- Visualizar alterações em arquivos.
- Comparar diferenças linha por linha.
- Fazer stage e unstage por arquivo ou trecho.
- Criar commits com mensagens.
- Consultar histórico de commits.
- Comparar commits e branches.
- Executar fetch, pull e push.
- Resolver conflitos de merge.
- Fazer merge e rebase com visualização apropriada.
- Consultar tags e releases.
- Configurar remotes.
- Exibir erros de autenticação e conflitos de maneira compreensível.
- Abrir o repositório no editor configurado pelo usuário.

Nunca execute commits ou pushes automaticamente sem que a ação tenha sido autorizada por uma rotina explicitamente configurada.

O aplicativo deve funcionar com GitHub e outros servidores Git compatíveis, sem depender exclusivamente de um provedor.

## 8. Proteção automática contra commits de arquivos sensíveis

Implemente um sistema de segurança específico para o Git.

Antes de criar commits ou realizar push, verifique:

- Arquivos `.env` e suas variantes.
- Chaves privadas SSH.
- Certificados e arquivos de credenciais.
- Tokens de acesso.
- Senhas em arquivos de configuração.
- Arquivos de backup que contenham dados sensíveis.
- Configurações locais da própria aplicação.
- Pastas de cache, dependências e arquivos de build excluídos pelo projeto.

Inclua verificações por nome, extensão, padrões de conteúdo e regras configuráveis.

Use `.gitignore`, mas não confie exclusivamente nele.

Se um arquivo sensível já estiver sendo rastreado pelo Git, avise o usuário que adicioná-lo ao `.gitignore` não o remove automaticamente do histórico nem do índice.

Ofereça uma ação assistida para removê-lo do índice sem apagar o arquivo local, quando apropriado.

Não modifique o histórico remoto automaticamente. Explique os riscos e solicite confirmação explícita para operações de reescrita de histórico.

## 9. Central de deploy com um clique

Este é um dos recursos mais importantes do sistema.

Quero cadastrar um projeto uma única vez e depois executar sua publicação por meio de um botão chamado **Deploy**.

Cada projeto deve permitir configurar:

- Diretório local do código.
- Repositório Git associado.
- Branch de origem.
- Servidor SSH de destino.
- Diretório remoto da aplicação.
- Método de publicação.
- Arquivos e pastas que devem ser enviados.
- Arquivos e pastas que devem ser ignorados.
- Comandos de preparação.
- Comandos de build.
- Estratégia de publicação.
- Comandos de reinicialização.
- Testes de saúde.
- Estratégia de rollback.
- Variáveis e segredos fornecidos de forma segura.
- Tempo limite de cada etapa.

Implemente diferentes estratégias de deploy.

**Modo A — Sincronização por SSH/SFTP**

Enviar apenas arquivos novos ou alterados, com base em comparação confiável entre origem e destino.

Não sobrescrever arquivos desnecessários. Não apagar arquivos remotos sem uma regra explícita.

**Modo B — Publicação baseada em Git**

Atualizar o código no servidor usando o repositório e a branch configurados.

Antes de alterar arquivos, detectar mudanças locais não previstas e evitar sobrescrevê-las silenciosamente.

**Modo C — Build local e envio dos artefatos**

Compilar o projeto no computador local e enviar somente os artefatos necessários.

Suportar aplicações Node.js, React, Vite, PHP e projetos com binários compilados, além de permitir configurações personalizadas.

**Modo D — Deploy por script**

Executar uma sequência configurada de comandos SSH, com parâmetros, verificações e logs.

**Modo E — Docker e Docker Compose**

Permitir operações configuradas de build, publicação de imagens, atualização de containers e verificação de saúde.

Não presumir que todos os servidores possuem Docker, Node.js, PHP, Rust ou qualquer outra ferramenta instalada. Verificar os requisitos antes da execução.

## 10. Pipeline visual de publicação

Cada deploy deve ser apresentado como uma sequência de etapas.

Exemplo:

1. Validar configurações e credenciais.
2. Testar conexão SSH.
3. Verificar a branch e o estado do Git.
4. Identificar arquivos alterados.
5. Verificar segredos e arquivos proibidos.
6. Executar testes, quando configurados.
7. Compilar o projeto, quando necessário.
8. Preparar a publicação.
9. Criar backup ou ponto de recuperação, quando configurado.
10. Enviar arquivos ou artefatos.
11. Executar comandos de atualização.
12. Reiniciar serviços, quando necessário.
13. Realizar testes de saúde.
14. Confirmar o resultado.
15. Registrar o histórico do deploy.

Cada etapa deve apresentar estado visual: aguardando, executando, concluída, ignorada ou falhou.

Ao falhar, o sistema deve mostrar a etapa, o comando executado, o código de saída, a mensagem de erro e sugestões de diagnóstico.

Não exibir sucesso se alguma etapa obrigatória falhar.

Permitir interromper o pipeline quando possível, preservando informações para diagnóstico.

Para projetos críticos, oferecer publicação por releases em diretórios separados e troca controlada do diretório ativo, quando a infraestrutura permitir. Isso facilita rollback e reduz o risco de deixar a aplicação parcialmente atualizada.

## 11. Gerenciamento de múltiplos servidores e ambientes

Permita organizar servidores em grupos:

- Desenvolvimento.
- Homologação.
- Produção.
- Servidores de banco de dados.
- Servidores de aplicações.
- Servidores Docker.
- VPS e servidores dedicados.

Cada servidor deve possuir identificação visual clara do ambiente.

Operações em produção devem ter avisos próprios e regras de confirmação configuráveis.

Permita cadastrar diferentes projetos no mesmo servidor e o mesmo projeto em servidores diferentes.

Implemente modelos de deploy reutilizáveis.

Não presuma que um servidor tenha acesso a outro ou que todas as conexões compartilhem as mesmas credenciais.

## 12. Dashboard de infraestrutura

Crie uma tela inicial que mostre:

- Servidores cadastrados.
- Estado da conexão.
- Última conexão bem-sucedida.
- Projetos associados.
- Último deploy e seu resultado.
- Histórico de falhas.
- Transferências em andamento.
- Uso de CPU, memória e disco, quando consultáveis.
- Containers e serviços monitorados, quando disponíveis.
- Alertas de configuração.
- Atalhos para tarefas frequentes.

As informações de infraestrutura devem ser obtidas dos servidores reais. Não utilizar dados simulados na versão funcional.

A coleta de métricas deve ser opcional e executar comandos de leitura com baixo impacto.

## 13. Histórico e auditoria

Registre localmente:

- Data e horário das operações.
- Servidor e projeto envolvidos.
- Usuário local responsável.
- Comandos executados por rotinas.
- Arquivos transferidos.
- Etapas dos deploys.
- Resultados e códigos de saída.
- Falhas de conexão.
- Ações Git realizadas pela interface.

Os logs não podem conter senhas, chaves privadas, tokens ou valores de variáveis secretas.

Permita pesquisar e filtrar o histórico por servidor, projeto, data, tipo de operação e resultado.

Não execute operações novamente apenas porque o aplicativo foi reiniciado. A retomada deve ser explícita e segura.

## 14. Interface e experiência de uso

A interface precisa parecer um produto comercial profissional de engenharia de software.

Utilize um design moderno, limpo e funcional, com:

- Tema escuro e claro.
- Menu lateral para servidores, projetos, Git, transferências e histórico.
- Painel central com conteúdo contextual.
- Abas para terminal, arquivos, Git e deploy.
- Indicadores claros de conexão.
- Pesquisa global.
- Filtros e ordenação.
- Janelas de configuração organizadas.
- Notificações discretas.
- Atalhos de teclado.
- Layout adaptável a diferentes resoluções.
- Feedback visual durante operações demoradas.

Evite excesso de efeitos, animações pesadas e interfaces poluídas.

A interface nunca deve travar durante transferências, execução de comandos ou operações Git.

Todas as operações demoradas devem ocorrer de maneira assíncrona, com cancelamento e tratamento de erros adequados.

## 15. Arquitetura e qualidade do código

Estruture o projeto por módulos bem definidos.

Sugestão de módulos:

- `connection_manager`
- `ssh_service`
- `sftp_service`
- `terminal_service`
- `git_service`
- `deployment_engine`
- `pipeline_executor`
- `secret_manager`
- `security_scanner`
- `server_inventory`
- `project_manager`
- `transfer_manager`
- `audit_logger`
- `health_checker`
- `settings_repository`

Utilize contratos explícitos entre frontend e backend.

Valide entradas, caminhos, parâmetros de comandos e configurações.

Evite concatenação insegura de strings para montar comandos shell. Utilize argumentos separados sempre que possível.

Não construa um executor remoto que aceite comandos arbitrários de fontes não confiáveis.

Inclua tratamento de erros, timeouts, limites de concorrência e cancelamento cooperativo.

Crie testes unitários, testes de integração e cenários simulados de falha para as operações críticas.

Inclua logs técnicos para diagnóstico, sem revelar segredos.

## 16. Entrega do produto

Não quero somente um protótipo visual, um documento de arquitetura ou uma tela demonstrativa.

Quero uma aplicação funcional, com integração real ao SSH, SFTP, Git e deploy.

Implemente em etapas, mantendo o projeto compilável.

Ordem de prioridade:

1. Estrutura desktop, interface e persistência local.
2. Importação das configurações SSH existentes.
3. Teste de conexão e gerenciamento de servidores.
4. Terminal SSH funcional.
5. Gerenciador de arquivos com upload e download.
6. Integração Git completa.
7. Cadastro de projetos.
8. Deploy automatizado com pipeline visual.
9. Verificação de segredos e segurança.
10. Dashboard, histórico, rollback e refinamentos.

Após cada etapa, compile o projeto, execute os testes aplicáveis e corrija os problemas antes de avançar.

Crie documentação de instalação, compilação, execução, configuração e utilização.

Forneça uma forma de gerar um instalador desktop para Windows e mantenha a arquitetura preparada para Linux.

Não deixe botões sem implementação. Não use respostas fictícias para simular conexões, transferências, commits ou deploys.

Quando um recurso depender de permissões ou ferramentas ausentes no servidor, informe isso claramente e ofereça um diagnóstico.

## 17. Critério de sucesso

O resultado final deve permitir o seguinte fluxo:

1. Abrir o aplicativo.
2. Encontrar os acessos SSH existentes no computador.
3. Selecionar um servidor.
4. Selecionar um projeto local.
5. Conferir as alterações no Git.
6. Selecionar o destino e a estratégia de deploy.
7. Clicar em **Deploy**.
8. Acompanhar visualmente cada etapa.
9. Receber a confirmação real da publicação.
10. Consultar logs e realizar rollback, se necessário.

A ferramenta deve reduzir drasticamente o trabalho manual de publicação, mantendo segurança, rastreabilidade e controle.

**Prioridade máxima: produtividade real, automação confiável, interface profissional e proteção dos acessos SSH.**