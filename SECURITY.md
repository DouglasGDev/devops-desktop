# Relatório de vulnerabilidade

O DevOps Desktop fala com servidores SSH, lê `~/.ssh/config`, executa comandos na sua máquina e no servidor, e mexe em
repositórios Git. Por isso, problema de segurança é tratado antes de qualquer feature.

## Como reportar

- **Abra uma [security advisory](https://github.com/DouglasGDev/devops-desktop/security/advisories/new)** no repositório
  (GitHub → Security → Report a vulnerability). Ela é privada: só você e quem mantém o projeto veem.
- Não use issue pública, PR pública, nem comentário em issue existente para descrever a falha — nem parcialmente.
- Se você não conseguir pela interface do GitHub, descreva o problema em uma issue **sem** o vetor de ataque e sem
  prova de conceito, pedindo contato em particular.

Inclua, quando possível: versão do aplicativo (o instalador é `0.1.0`), sistema operacional, o que a falha permite
fazer, e o passo a passo para reproduzir.

O que acontece depois: quem mantém responde em até 7 dias dizendo se confirma, pede o que faltar, e combina com você
como e quando o conserto será publicado. Se a falha estiver em dependência, isso fica dito com o nome dela.

## O que o projeto já considera linha vermelha

Mudanças que violem qualquer item abaixo são tratadas como falha de segurança, não como detalhe de implementação — o
detalhado está em [README.md](README.md), §10 (Modelo de segurança):

- gravar senha, token, chave privada ou valor de variável em banco, log, auditoria, configuração ou commit;
- aceitar automaticamente a host key de um servidor desconhecido ou alterado;
- montar comando por concatenação de texto, em vez de argumentos separados, ou executar comando remoto que não passou
  pelas validações existentes;
- apagar, sobrescrever ou mover arquivo no destino sem a confirmação e o ponto de recuperação que a etapa correspondente
  registra;
- commitar ou publicar sem ação explícita de quem usa.

Se a sua contribuição corrige um desses pontos, diga isso no texto da PR — ela vai para o topo da fila.
