//! Executores de runtime do deploy.
//!
//! O motor continua sendo um só: estas funções **não executam nada**, elas montam
//! o roteiro POSIX que uma etapa do motor já sabe mandar para o servidor
//! (`ctx.script` + `ctx.remote`) ou para esta máquina, e lêem de volta o que o
//! roteiro imprimiu. A razão de existir do módulo é a segunda parte: nenhuma ação
//! de runtime é dada como concluída pelo código de saída do comando. `docker run`
//! devolve 0 para um container que morre dois segundos depois, `pm2 start` devolve
//! 0 para um processo que nunca chegou a `online`. Todo roteiro termina imprimindo
//! o estado **lido da ferramenta** (`docker inspect`, `docker compose ps`,
//! `pm2 jlist`) e quem chama só declara sucesso sobre o que foi lido.

use crate::sshexec::shell_quote;
use serde::Deserialize;

/// Os executores que existem. Vazio é "nenhum executor": a fila das quinze etapas
/// continua exatamente como era antes desta camada.
pub const RUNTIMES: [&str; 4] = ["", "docker", "compose", "pm2"];

/// Como o `docker inspect` é pedido: nome, estado e, se houver HEALTHCHECK, a
/// saúde. Ficou fora de um `format!` porque as chaves do template colidiriam com
/// os placeholders da própria formatação.
const MODELO_CONTAINER: &str = "{{.Name}}|{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}-{{end}}";

/// Nome e id da imagem que o build acabou de produzir.
const MODELO_IMAGEM: &str = "{{.Id}}";

/// O que o docker procura quando o modelo não disse onde o Dockerfile está, e o que
/// o compose abre quando o arquivo não foi nomeado. Ficam aqui porque a tela, a
/// validação e o motor montam o alvo a partir das mesmas colunas vazias: a regra do
/// default tem de ter um dono só.
pub const DOCKERFILE_PADRAO: &str = "Dockerfile";
pub const COMPOSE_PADRAO: &str = "docker-compose.yml";

/// Quantas leituras o laço de espera faz antes de dizer "sem resposta dentro do
/// prazo". Cada volta dorme 2 s, então o laço precisa caber no prazo da etapa: com
/// um prazo curto e tentativas fixas seria o motor matando o roteiro no meio da
/// espera, e a tela mostraria o timeout em vez do estado real do container.
pub fn tentativas_de_espera(timeout_secs: i64) -> u8 {
    (timeout_secs / 4).clamp(2, 8) as u8
}

/// O que a sonda da etapa 8 precisa procurar para cada executor. O `docker
/// compose` v2 é um plugin do `docker`, não um binário à parte — por isso
/// `compose` sonda `docker` e não `docker-compose`, que é o caminho legado e ainda
/// existe em servidores antigos.
pub fn ferramentas(runtime: &str) -> &'static [&'static str] {
    match runtime {
        "docker" => &["docker"],
        "compose" => &["docker"],
        "pm2" => &["pm2", "node"],
        _ => &[],
    }
}

pub fn validate_runtime(value: &str) -> Result<(), String> {
    if RUNTIMES.contains(&value) {
        Ok(())
    } else {
        Err(format!("executor de runtime desconhecido: {value}"))
    }
}

/// O nome que aparece nas mensagens — o que a pessoa entende, não o código.
pub fn nome(runtime: &str) -> &'static str {
    match runtime {
        "docker" => "Docker",
        "compose" => "Docker Compose",
        "pm2" => "PM2",
        _ => "sem executor",
    }
}

/// O que o executor precisa saber: o que o modelo declarou e onde as coisas estão
/// naquela corrida. `diretorio` é o caminho do código no destino — a pasta
/// publicada no servidor, a raiz do projeto nesta máquina.
#[derive(Debug, Clone)]
pub struct Alvo {
    pub runtime: String,
    pub diretorio: String,
    pub imagem: String,
    pub dockerfile: String,
    pub servico: String,
    pub compose_file: String,
    pub portas: Vec<String>,
    /// A etiqueta da corrida: entra no nome da imagem, por isso é saneada aqui.
    pub tag: String,
    /// Quantas tentativas o roteiro espera por um container que ainda está
    /// `starting` antes de dizer que não respondeu. Curto porque o prazo da etapa
    /// é o dono do relógio; aqui só se evita o falso negativo da primeira leitura.
    pub espera_saude: u8,
}

/// As colunas de executor de uma definição, na forma que têm tanto no modelo salvo
/// quanto no que a tela envia: `runtime` preenchido, o resto opcional. É um parâmetro
/// só porque as duas fontes dizem a mesma coisa — o alvo é a tradução delas, não uma
/// lista de argumentos soltos.
#[derive(Debug, Clone, Copy)]
pub struct Definicao<'a> {
    pub runtime: &'a str,
    pub imagem: Option<&'a str>,
    pub dockerfile: Option<&'a str>,
    pub compose_file: Option<&'a str>,
    pub servico: Option<&'a str>,
    pub portas: &'a [String],
}

impl Alvo {
    /// Um alvo só com o executor — o caso de teste do que falta preencher. Vive
    /// dentro dos testes porque o motor sempre monta o alvo a partir de uma
    /// definição salva (`Alvo::da_definicao`).
    #[cfg(test)]
    pub fn novo(runtime: &str) -> Self {
        Self {
            runtime: runtime.to_string(),
            diretorio: String::new(),
            imagem: String::new(),
            dockerfile: DOCKERFILE_PADRAO.into(),
            servico: String::new(),
            compose_file: COMPOSE_PADRAO.into(),
            portas: Vec::new(),
            tag: "atual".into(),
            espera_saude: 8,
        }
    }

    /// O alvo de uma definição salva, com o caminho do código naquela corrida e o
    /// prazo da etapa. Existe uma única tradução das colunas do modelo para cá: a
    /// checagem da tela e o motor montam o mesmo alvo.
    pub fn da_definicao(def: &Definicao<'_>, diretorio: &str, timeout_secs: i64) -> Self {
        Self {
            runtime: def.runtime.trim().to_lowercase(),
            diretorio: diretorio.to_string(),
            imagem: def.imagem.unwrap_or("").trim().to_string(),
            dockerfile: preenchido(def.dockerfile, DOCKERFILE_PADRAO),
            servico: def.servico.unwrap_or("").trim().to_string(),
            compose_file: preenchido(def.compose_file, COMPOSE_PADRAO),
            portas: def
                .portas
                .iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
            tag: "atual".into(),
            espera_saude: tentativas_de_espera(timeout_secs),
        }
    }

    pub fn com_tag(mut self, tag: &str) -> Self {
        let limpa: String = tag
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '.' { c } else { '-' })
            .collect();
        self.tag = limpa.trim_matches('-').to_string();
        self
    }

    /// O container que o executor cria quando o modelo não deu um nome: o último
    /// pedaço da imagem. Derivar é melhor que obrigar, e a tela mostra o resultado.
    pub fn container(&self) -> String {
        let dado = self.servico.trim();
        if !dado.is_empty() {
            return dado.to_string();
        }
        self.imagem
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("app")
            .to_ascii_lowercase()
    }

    /// `imagem:tag` desta corrida.
    pub fn imagem_tag(&self) -> String {
        format!("{}:{}", self.imagem.trim(), self.tag)
    }

    /// O que tem de estar preenchido para o executor funcionar. É uma função só
    /// porque a tela (antes de salvar) e o motor (antes de falar com o servidor)
    /// chamam a mesma checagem: o que a tela aceita o motor tem de aceitar.
    pub fn conferir(&self) -> Result<(), String> {
        validate_runtime(&self.runtime)?;
        if self.runtime.is_empty() {
            return Ok(());
        }
        if self.diretorio.trim().is_empty() {
            return Err(format!("{}: falta o diretório do código no destino", nome(&self.runtime)));
        }
        match self.runtime.as_str() {
            "docker" => {
                let imagem = self.imagem.trim();
                if imagem.is_empty() {
                    return Err("Docker: diga o nome da imagem (ex.: meuapp ou registry.local/meuapp)".into());
                }
                if !imagem
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-_/:@".contains(c))
                {
                    return Err("Docker: o nome da imagem só aceita minúsculas, números e . - _ / : @".into());
                }
                if imagem.contains("//") || imagem.starts_with(':') || imagem.ends_with(':') {
                    return Err("Docker: o nome da imagem tem uma barra ou dois-pontos sobrando".into());
                }
                if self.dockerfile.trim().is_empty() || self.dockerfile.starts_with('/') {
                    return Err("Docker: o Dockerfile é um caminho dentro do projeto, não um caminho absoluto".into());
                }
                if self.tag.is_empty() {
                    return Err("Docker: a etiqueta da imagem desta corrida ficou vazia".into());
                }
                for porta in &self.portas {
                    conferir_porta(porta)?;
                }
            }
            "compose" => {
                if self.compose_file.trim().is_empty() || self.compose_file.starts_with('/') {
                    return Err("Docker Compose: o arquivo do compose é um caminho dentro do projeto".into());
                }
                if !self.portas.is_empty() {
                    return Err(
                        "Docker Compose: quem publica as portas é o arquivo do compose, não o modelo"
                            .into(),
                    );
                }
            }
            "pm2" => {
                if self.servico.trim().is_empty() {
                    return Err("PM2: diga o nome do app como ele aparece no `pm2 list`".into());
                }
                if !self.portas.is_empty() {
                    return Err("PM2: a porta em que o app escuta é do próprio app, não do modelo".into());
                }
            }
            _ => return Err(format!("executor de runtime desconhecido: {}", self.runtime)),
        }
        Ok(())
    }
}

/// O valor declarado pelo modelo, ou o default do executor quando a coluna está vazia.
fn preenchido(declarado: Option<&str>, default: &str) -> String {
    match declarado.map(str::trim).filter(|s| !s.is_empty()) {
        Some(valor) => valor.to_string(),
        None => default.to_string(),
    }
}

/// `[[IP:]HOST:]CONTAINER[/PROTO]`, a forma que o `docker run -p` aceita. Conferir
/// aqui evita um container que sobe sem publicar nada porque um número tinha letra.
fn conferir_porta(porta: &str) -> Result<(), String> {
    let bruto = porta.trim();
    if bruto.is_empty() {
        return Err("Docker: tem uma porta publicada vazia na lista".into());
    }
    let (corpo, proto) = match bruto.split_once('/') {
        Some((c, p)) if matches!(p, "tcp" | "udp" | "sctp") => (c, Some(p)),
        Some((_, p)) => return Err(format!("Docker: protocolo de porta desconhecido: {p}")),
        None => (bruto, None),
    };
    let partes: Vec<&str> = corpo.split(':').collect();
    if partes.len() > 3 || partes.is_empty() {
        return Err(format!("Docker: não dá para entender a porta “{bruto}”"));
    }
    if let Some(ip) = partes.first().filter(|_| partes.len() == 3) {
        if !ip
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ':')
        {
            return Err(format!("Docker: o endereço da porta “{bruto}” não é um IP"));
        }
    }
    let ultima = partes.last().unwrap_or(&"");
    let numero = ultima
        .parse::<u16>()
        .map_err(|_| format!("Docker: a porta de container em “{bruto}” não é um número 1-65535"))?;
    if numero == 0 {
        return Err(format!("Docker: a porta de container em “{bruto}” é zero"));
    }
    if partes.len() > 1 {
        let host = partes[partes.len() - 2];
        let numero = host
            .parse::<u16>()
            .map_err(|_| format!("Docker: a porta de origem em “{bruto}” não é um número 1-65535"))?;
        if numero == 0 {
            return Err(format!("Docker: a porta de origem em “{bruto}” é zero"));
        }
    }
    let _ = proto;
    Ok(())
}

// ---------------------------------------------------------------------------
// Roteiros. Cada um devolve o corpo do script; `ctx.script` acrescenta o
// `set -eu` e as variáveis, e é o motor que manda rodar.
// ---------------------------------------------------------------------------

impl Alvo {
    /// O serviço como argumento de linha de comando, com o espaço à frente — ou
    /// nada, quando o modelo não declarou um. `docker compose … [serviço]` aceita
    /// o arquivo inteiro sem o argumento, e passar `''` é um nome de serviço que o
    /// docker recusa: a linha vazia é o "todos", não um serviço vazio.
    fn servico_argumento(&self) -> String {
        let dado = self.servico.trim();
        if dado.is_empty() {
            String::new()
        } else {
            format!(" {}", shell_quote(dado))
        }
    }

    /// Etapa 11: produzir a imagem desta corrida. Para compose é `compose build`
    /// do serviço declarado — sem serviço, o arquivo inteiro.
    pub fn roteiro_build(&self) -> Option<String> {
        match self.runtime.as_str() {
            "docker" => Some(format!(
                "cd {}\ndocker build -f {} -t {} .\nprintf 'IMAGEM:%s\\n' \"$(docker image inspect --format '{}' {})\"\n",
                shell_quote(&self.diretorio),
                shell_quote(self.dockerfile.trim()),
                shell_quote(&self.imagem_tag()),
                MODELO_IMAGEM,
                shell_quote(&self.imagem_tag()),
            )),
            "compose" => Some(format!(
                "cd {}\ndocker compose -f {} build{}\n",
                shell_quote(&self.diretorio),
                shell_quote(self.compose_file.trim()),
                self.servico_argumento(),
            )),
            _ => None,
        }
    }

    /// Etapa 12: pôr o runtime no ar. Remove o container anterior do mesmo nome
    /// (é uma máquina só, não há blue/green aqui) e imprime o estado lido de volta
    /// — inclusive o id da imagem que estava no ar antes, que é o que a reversão
    /// da etapa 15 usa para não ter que adivinhar.
    pub fn roteiro_subir(&self) -> Option<String> {
        match self.runtime.as_str() {
            "docker" => {
                let nome = self.container();
                let portas: Vec<String> = self
                    .portas
                    .iter()
                    .map(|p| format!("-p {}", shell_quote(p.trim())))
                    .collect();
                Some(format!(
                    concat!(
                        "cd {dir}\n",
                        "anterior=$(docker ps -aq --filter {filtro} | head -n1)\n",
                        "if [ -n \"$anterior\" ]; then\n",
                        "  printf 'ANTERIOR:%s|%s\\n' \"$anterior\" \"$(docker image inspect --format '{mimg}' \"$anterior\" 2>/dev/null || echo -)\"\n",
                        "  docker rm -f \"$anterior\" >/dev/null\n",
                        "fi\n",
                        "novo=$(docker run -d --name {nq} --restart unless-stopped {portas} {img})\n",
                        "{espera}\n",
                    ),
                    dir = shell_quote(&self.diretorio),
                    // O filtro inteiro é um argumento: montar `name='^{nome}$'` no
                    // texto deixaria um nome com apóstrofo escapar das aspas e
                    // virar comando no servidor.
                    filtro = shell_quote(&format!("name=^{nome}$")),
                    nq = shell_quote(&nome),
                    mimg = MODELO_IMAGEM,
                    portas = portas.join(" "),
                    img = shell_quote(&self.imagem_tag()),
                    espera = leitura_container(&nome, self.espera_saude),
                ))
            }
            "compose" => Some(format!(
                concat!(
                    "cd {dir}\n",
                    "docker compose -f {arq} up -d{servico}\n",
                    "ids=$(docker compose -f {arq} ps -q{servico})\n",
                    "if [ -n \"$ids\" ]; then\n",
                    "  docker inspect --format '{modelo}' $ids\n",
                    "else\n",
                    "  printf 'RAZAO:o docker compose não devolveu nenhum container\\n'\n",
                    "fi\n",
                ),
                dir = shell_quote(&self.diretorio),
                arq = shell_quote(self.compose_file.trim()),
                servico = self.servico_argumento(),
                modelo = MODELO_CONTAINER,
            )),
            "pm2" => Some(format!(
                concat!(
                    "cd {dir}\n",
                    "arquivo=''\n",
                    "for f in ecosystem.config.js ecosystem.config.cjs ecosystem.config.json; do\n",
                    "  if [ -f \"$f\" ]; then arquivo=\"$f\"; break; fi\n",
                    "done\n",
                    "if [ -n \"$arquivo\" ]; then\n",
                    "  pm2 startOrReload \"$arquivo\" --update-env\n",
                    "elif pm2 describe {nq} >/dev/null 2>&1; then\n",
                    "  pm2 reload {nq}\n",
                    "else\n",
                    "  printf 'RAZAO:o app %s não está no pm2 desta máquina e não há ecosystem.config.* na pasta publicada\\n' {nq}\n",
                    "fi\n",
                    "pm2 jlist\n",
                ),
                dir = shell_quote(&self.diretorio),
                nq = shell_quote(self.servico.trim()),
            )),
            _ => None,
        }
    }

    /// Etapa 13: reler o estado, sem mudar nada. É o que dá a frase "3 container(s)
    /// running" em vez de "comando terminou bem".
    pub fn roteiro_saude(&self) -> Option<String> {
        match self.runtime.as_str() {
            "docker" => Some(format!(
                "docker inspect --format '{}' {}\n",
                MODELO_CONTAINER,
                shell_quote(&self.container())
            )),
            "compose" => Some(format!(
                concat!(
                    "cd {dir}\n",
                    "ids=$(docker compose -f {arq} ps -q)\n",
                    "if [ -n \"$ids\" ]; then docker inspect --format '{modelo}' $ids; fi\n",
                ),
                dir = shell_quote(&self.diretorio),
                arq = shell_quote(self.compose_file.trim()),
                modelo = MODELO_CONTAINER,
            )),
            "pm2" => Some("pm2 jlist\n".to_string()),
            _ => None,
        }
    }

    /// Reversão: voltar o container para a imagem que estava no ar antes desta
    /// corrida, se ela ainda existir no depósito de imagens da máquina. Sem a
    /// imagem anterior o roteiro devolve o motivo em texto — a reversão diz isso e
    /// não finge que repos.
    pub fn roteiro_reverter(&self, imagem_anterior: &str) -> String {
        let nome = self.container();
        format!(
            concat!(
                "if ! docker image inspect {ant} >/dev/null 2>&1; then\n",
                "  printf 'RAZAO:a imagem %s não existe mais nesta máquina\\n' {ant}\n",
                "  exit 0\n",
                "fi\n",
                "antigo=$(docker ps -aq --filter {filtro} | head -n1)\n",
                "if [ -n \"$antigo\" ]; then\n",
                "  printf 'ANTERIOR:%s|%s\\n' \"$antigo\" \"$(docker image inspect --format '{mimg}' \"$antigo\" 2>/dev/null || echo -)\"\n",
                "  docker rm -f \"$antigo\" >/dev/null\n",
                "fi\n",
                "docker run -d --name {nq} --restart unless-stopped {portas} {ant}\n",
                "{espera}\n",
            ),
            ant = shell_quote(imagem_anterior),
            filtro = shell_quote(&format!("name=^{nome}$")),
            nq = shell_quote(&nome),
            mimg = MODELO_IMAGEM,
            portas = self
                .portas
                .iter()
                .map(|p| format!("-p {}", shell_quote(p.trim())))
                .collect::<Vec<_>>()
                .join(" "),
            espera = leitura_container(&nome, self.espera_saude),
        )
    }
}

/// O trecho que lê o container e repete enquanto ele estiver `starting`. O laço
/// tem teto de tentativas e dorme 2 s — é o bounded wait da regra da casa: a
/// ferramenta que não responde tem de dar nome a isso, não travar a corrida.
fn leitura_container(nome: &str, tentativas: u8) -> String {
    format!(
        concat!(
            "nq={nq}\n",
            "para=0\n",
            "while [ $para -lt {t} ]; do\n",
            "  linha=$(docker inspect --format '{modelo}' \"$nq\" 2>/dev/null || echo '-|ausente|-')\n",
            "  caso=$(printf '%s' \"$linha\" | cut -d'|' -f2)\n",
            "  saude=$(printf '%s' \"$linha\" | cut -d'|' -f3)\n",
            "  if [ \"$caso\" != \"created\" ] && [ \"$caso\" != \"starting\" ] && [ \"$saude\" != \"starting\" ]; then\n",
            "    printf 'CONTAINER:%s\\n' \"$linha\"\n",
            "    break\n",
            "  fi\n",
            "  para=$((para+1))\n",
            "  sleep 2\n",
            "done\n",
            "if [ $para -ge {t} ]; then printf 'CONTAINER:-|sem resposta dentro do prazo\\n'; fi\n",
        ),
        nq = shell_quote(nome),
        t = tentativas.max(1),
        modelo = MODELO_CONTAINER,
    )
}

// ---------------------------------------------------------------------------
// Leitura do que a ferramenta devolveu.
// ---------------------------------------------------------------------------

/// Um container como `docker inspect` devolve. `saude` é `-` quando a imagem não
/// declara HEALTHCHECK — ausência não é falha, e o motor diz qual dos dois foi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub nome: String,
    pub estado: String,
    pub saude: String,
}

impl Container {
    pub fn no_ar(&self) -> bool {
        self.estado == "running"
            && matches!(self.saude.as_str(), "-" | "" | "healthy")
    }

    /// A palavra que vai para a tela. Curta porque é o estado, não um parágrafo.
    pub fn resumo(&self) -> String {
        match self.saude.as_str() {
            "-" | "" => format!("{} · {}", self.nome, self.estado),
            s => format!("{} · {} · saúde {}", self.nome, self.estado, s),
        }
    }
}

pub fn parse_containers(raw: &str) -> Vec<Container> {
    let mut out = Vec::new();
    for bruto in raw.lines() {
        let linha = bruto.trim();
        let corpo = linha.strip_prefix("CONTAINER:").unwrap_or(linha);
        let partes: Vec<&str> = corpo.split('|').map(|p| p.trim().trim_start_matches('/')).collect();
        if partes.len() < 2 || partes[1].is_empty() {
            continue;
        }
        // `-` no lugar do nome é o roteiro dizendo que não identificou container
        // nenhum (sem resposta dentro do prazo, container sumiu). O nome fica
        // vazio para o resto do motor não tratar `-` como nome de app.
        let nome = if partes[0].is_empty() || partes[0] == "-" {
            String::new()
        } else {
            partes[0].to_string()
        };
        out.push(Container {
            nome,
            estado: partes[1].to_string(),
            saude: partes.get(2).map(|s| s.to_string()).unwrap_or_else(|| "-".to_string()),
        });
    }
    out
}

/// O id da imagem que o build produziu, lido do marcador `IMAGEM:`.
pub fn parse_imagem(raw: &str) -> Option<String> {
    raw.lines()
        .map(|l| l.trim())
        .find_map(|l| l.strip_prefix("IMAGEM:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// O `ANTERIOR:id|imagem` que a subida imprimiu — é o que a reversão oferece.
pub fn parse_anterior(raw: &str) -> Option<(String, String)> {
    let linha = raw
        .lines()
        .map(|l| l.trim())
        .find_map(|l| l.strip_prefix("ANTERIOR:"))?;
    let (id, imagem) = linha.split_once('|')?;
    let id = id.trim();
    let imagem = imagem.trim();
    if id.is_empty() || imagem == "-" || imagem.is_empty() {
        return None;
    }
    Some((id.to_string(), imagem.to_string()))
}

/// O motivo que o próprio roteiro imprimiu quando não havia nada para ler. É a
/// resposta da ferramenta, não uma frase montada pelo motor: "o app não está no pm2"
/// e "a imagem sumiu da máquina" são fatos lidos, e chegam à tela com as palavras
/// com que foram ditas.
pub fn parse_razao(raw: &str) -> Option<String> {
    raw.lines()
        .map(|l| l.trim())
        .find_map(|l| l.strip_prefix("RAZAO:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// O veredito de uma subida ou de uma leitura de saúde, decidido sobre o que a
/// ferramenta devolveu. Existe um único lugar porque a etapa 12 (que mudou o estado)
/// e a etapa 13 (que só relê) têm de chegar à mesma conclusão sobre a mesma máquina.
/// `Err` é sempre o estado lido, nunca o código de saída do comando.
pub fn estado_no_ar(alvo: &Alvo, raw: &str) -> Result<String, String> {
    if let Some(motivo) = parse_razao(raw) {
        return Err(motivo);
    }
    match alvo.runtime.as_str() {
        "docker" | "compose" => {
            let lidos = parse_containers(raw);
            if lidos.is_empty() {
                return Err("o docker não devolveu estado nenhum do container".to_string());
            }
            let resumo = resumir(
                lidos.iter().map(|c| c.resumo()).collect(),
            );
            if lidos.iter().all(Container::no_ar) {
                Ok(format!("{} container(s) no ar: {resumo}", lidos.len()))
            } else {
                Err(format!("os containers não estão todos no ar: {resumo}"))
            }
        }
        "pm2" => {
            let lista = parse_pm2(raw)?;
            match achar_pm2(&lista, &alvo.servico) {
                Some(app) if app.no_ar() => Ok(format!("pm2 · {}", app.resumo())),
                Some(app) => Err(format!(
                    "o app {} não está no ar: {} · pid {}",
                    app.nome, app.status, app.pid
                )),
                None => Err(format!(
                    "o app “{}” não está na lista do pm2 desta máquina",
                    alvo.servico
                )),
            }
        }
        outro => Err(format!("executor sem estado para ler: {outro}")),
    }
}

/// Os estados lidos na tela, sem estourar a linha: cinco bastam para saber o que
/// não subiu, e o resto está no log da etapa.
fn resumir(mut estados: Vec<String>) -> String {
    if estados.len() > 5 {
        let cortadas = estados.len() - 5;
        estados.truncate(5);
        estados.push(format!("… {cortadas}"));
    }
    estados.join(", ")
}

#[derive(Debug, Deserialize)]
struct ProcessoPm2 {
    name: String,
    #[serde(default)]
    pid: i64,
    #[serde(rename = "pm2_env")]
    ambiente: AmbientePm2,
}

#[derive(Debug, Deserialize)]
struct AmbientePm2 {
    #[serde(default)]
    status: String,
    #[serde(default)]
    restart_time: i64,
}

/// Um app PM2 lido do `pm2 jlist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPm2 {
    pub nome: String,
    pub status: String,
    pub pid: i64,
    pub reinicios: i64,
}

impl AppPm2 {
    pub fn no_ar(&self) -> bool {
        self.status == "online" && self.pid > 0
    }

    pub fn resumo(&self) -> String {
        format!("{} · {} · pid {} · {} reinicio(s)", self.nome, self.status, self.pid, self.reinicios)
    }
}

/// Lê o `pm2 jlist`. O `jlist` imprime o array inteiro na linha; se ele não vier
/// como JSON, a resposta é "não sei", nunca "não tem" — a tela separa as duas.
pub fn parse_pm2(raw: &str) -> Result<Vec<AppPm2>, String> {
    let corpo = raw
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with('['))
        .ok_or_else(|| "o pm2 não devolveu a lista de processos".to_string())?;
    let processos: Vec<ProcessoPm2> =
        serde_json::from_str(corpo).map_err(|e| format!("a lista do pm2 não é legível: {e}"))?;
    Ok(processos
        .into_iter()
        .map(|p| AppPm2 {
            nome: p.name,
            status: p.ambiente.status,
            pid: p.pid,
            reinicios: p.ambiente.restart_time,
        })
        .collect())
}

/// O app declarado pelo modelo, se ele estiver na lista.
pub fn achar_pm2<'a>(lista: &'a [AppPm2], nome: &str) -> Option<&'a AppPm2> {
    lista.iter().find(|a| a.nome == nome.trim())
}

#[cfg(test)]
mod testes {
    use super::*;

    fn docker() -> Alvo {
        Alvo {
            diretorio: "/var/www/site".into(),
            imagem: "meuapp".into(),
            servico: "site".into(),
            portas: vec!["8080:80".into(), "127.0.0.1:9000:9000/tcp".into()],
            tag: "v7-f40243ff".into(),
            ..Alvo::novo("docker")
        }
    }

    #[test]
    fn sem_executor_nao_muda_nada() {
        let alvo = Alvo::novo("");
        assert!(alvo.conferir().is_ok());
        assert!(alvo.roteiro_build().is_none());
        assert!(alvo.roteiro_subir().is_none());
        assert!(alvo.roteiro_saude().is_none());
        assert!(ferramentas("").is_empty());
    }

    #[test]
    fn o_build_de_docker_gera_a_imagem_da_corrida_e_ler_de_volta() {
        let roteiro = docker().roteiro_build().unwrap();
        assert!(roteiro.contains("docker build -f 'Dockerfile' -t 'meuapp:v7-f40243ff' ."));
        assert!(roteiro.contains("docker image inspect"));
        assert!(roteiro.contains("IMAGEM:"));
        assert!(parse_imagem("Step 1/3 : FROM alpine\nIMAGEM:sha256:abc123\n").as_deref() == Some("sha256:abc123"));
    }

    #[test]
    fn subir_docker_troca_o_container_e_nao_presume_sucesso() {
        let roteiro = docker().roteiro_subir().unwrap();
        // O container anterior é lido antes de ser removido: é a imagem da reversão.
        assert!(roteiro.contains("ANTERIOR:"));
        assert!(roteiro.contains("docker rm -f"));
        assert!(roteiro.contains("--name 'site'"));
        assert!(roteiro.contains("--filter 'name=^site$'"));
        assert!(roteiro.contains("-p '8080:80'"));
        assert!(roteiro.contains("-p '127.0.0.1:9000:9000/tcp'"));
        // O laço de saúde existe e tem teto.
        assert!(roteiro.contains("while [ $para -lt 8 ]"));
        assert!(roteiro.contains("sem resposta dentro do prazo"));
    }

    #[test]
    fn o_nome_do_container_nao_escapa_das_aspas_do_filtro() {
        let com_apostrofo = Alvo {
            servico: "d'app".into(),
            portas: vec![],
            ..docker()
        };
        // O filtro era montado como `name='^{nome}$'`: um nome com apóstrofo
        // cerrava a aspa e virava comando no servidor. Agora o argumento inteiro é
        // que é citado, na subida e na reversão.
        for roteiro in [
            com_apostrofo.roteiro_subir().unwrap(),
            com_apostrofo.roteiro_reverter("meuapp:atual"),
        ] {
            assert!(roteiro.contains("--filter 'name=^d'\\''app$'"), "{roteiro}");
            assert!(roteiro.contains("--name 'd'\\''app'"), "{roteiro}");
        }
    }

    #[test]
    fn a_leitura_do_container_manda_na_decisao() {
        let rodando = parse_containers("CONTAINER:/site|running|healthy\n");
        assert_eq!(rodando.len(), 1);
        assert!(rodando[0].no_ar());
        assert_eq!(rodando[0].resumo(), "site · running · saúde healthy");

        // `docker run` saiu 0 e o container morreu: não é sucesso.
        let morto = parse_containers("site|exited|-\n").remove(0);
        assert!(!morto.no_ar());
        assert_eq!(morto.estado, "exited");

        // HEALTHCHECK ausente é `-`, e isso NÃO é falha.
        let sem_check = parse_containers("site|running|-\n").remove(0);
        assert!(sem_check.no_ar());

        // Container que nunca respondeu ao laço: nome vazio, estado é o motivo.
        let mudo = parse_containers("CONTAINER:-|sem resposta dentro do prazo\n").remove(0);
        assert!(!mudo.no_ar());
        assert!(mudo.nome.is_empty());
        assert_eq!(parse_containers("nada aqui\n").len(), 0);
    }

    #[test]
    fn a_imagem_anterior_e_lida_para_a_reversao() {
        let lida = parse_anterior("ANTERIOR:9f2c|meuapp:v6-111\n").unwrap();
        assert_eq!(lida.0, "9f2c");
        assert_eq!(lida.1, "meuapp:v6-111");
        // `image inspect` devolvendo `-` (imagem sumiu) não vira alvo de reversão.
        assert!(parse_anterior("ANTERIOR:9f2c|-\n").is_none());
        assert!(parse_anterior("sem marcador\n").is_none());
    }

    #[test]
    fn reverter_recusa_quando_a_imagem_anterior_sumiu() {
        let roteiro = docker().roteiro_reverter("meuapp:v6-111");
        assert!(roteiro.contains("docker image inspect 'meuapp:v6-111'"));
        assert!(roteiro.contains("não existe mais nesta máquina"));
        assert!(roteiro.contains("RAZAO:"));
        // o container que estava no ar também é lido antes de sair: é o que a
        // reversão registra como sendo o que ela deslocou.
        assert!(roteiro.contains("ANTERIOR:"));
    }

    #[test]
    fn compose_sobe_o_servico_e_relista_por_ids() {
        let alvo = Alvo {
            diretorio: "/var/www/site".into(),
            compose_file: "docker-compose.yml".into(),
            servico: "web".into(),
            ..Alvo::novo("compose")
        };
        let roteiro = alvo.roteiro_subir().unwrap();
        assert!(roteiro.contains("docker compose -f 'docker-compose.yml' up -d 'web'"));
        assert!(roteiro.contains("ps -q"));
        assert!(roteiro.contains("docker inspect --format"));
        alvo.conferir().unwrap();
        // Sem serviço o compose trabalha no arquivo inteiro: `up -d ''` seria um
        // nome de serviço que o próprio docker recusa, não um "todos".
        let todos = Alvo { servico: String::new(), ..alvo };
        let sem_servico = todos.roteiro_subir().unwrap();
        assert!(sem_servico.contains("docker compose -f 'docker-compose.yml' up -d\n"), "{sem_servico}");
        assert!(sem_servico.contains("ps -q)"), "{sem_servico}");
        assert!(!sem_servico.contains("''"));
        assert!(todos.roteiro_build().unwrap().contains("docker compose -f 'docker-compose.yml' build\n"));
    }

    #[test]
    fn pm2_recarrega_quando_existe_e_e_honesto_quando_nao() {
        let alvo = Alvo {
            diretorio: "/var/www/site".into(),
            servico: "api".into(),
            ..Alvo::novo("pm2")
        };
        let roteiro = alvo.roteiro_subir().unwrap();
        // O ecosystem.config.* da pasta publicada é o arquivo lido, não um nome
        // inventado: `startOrReload` recebe o que existe na pasta.
        assert!(roteiro.contains("pm2 startOrReload \"$arquivo\" --update-env"));
        assert!(roteiro.contains("for f in ecosystem.config.js ecosystem.config.cjs"));
        assert!(roteiro.contains("pm2 reload 'api'"));
        assert!(roteiro.contains("RAZAO:o app %s não está no pm2 desta máquina"));
        assert!(roteiro.contains("pm2 jlist"));
        // nada do nome do app vai para dentro de aspas simples sem estarquoted
        let com_apostrofo = Alvo { servico: "d'api".into(), ..alvo.clone() };
        let aspas = com_apostrofo.roteiro_subir().unwrap();
        assert!(aspas.contains("'d'\\''api'"), "{aspas}");
    }

    #[test]
    fn o_estado_lido_manda_e_a_ferramenta_da_as_palavras() {
        let alvo = docker();

        // Subida que `docker run` disse 0 e o container morreu: lido, não presumido.
        let morto = alvo.roteiro_subir().unwrap();
        assert!(!morto.contains("RAZAO:"));
        let saida_morta = "/site|exited|-\n";
        assert!(estado_no_ar(&alvo, saida_morta)
            .unwrap_err()
            .contains("não estão todos no ar"));

        let ok = estado_no_ar(&alvo, "CONTAINER:/site|running|healthy\n").unwrap();
        assert!(ok.starts_with("1 container(s) no ar"), "{ok}");
        assert!(ok.contains("site · running · saúde healthy"), "{ok}");

        // Compose com dois serviços: um deles parado já recusa a etapa inteira.
        let compose = Alvo { diretorio: "/var/www/site".into(), ..Alvo::novo("compose") };
        assert!(estado_no_ar(&compose, "web|running|-\nworker|restarting|-\n")
            .unwrap_err()
            .contains("restarting"));
        assert!(estado_no_ar(&compose, "web|running|-\nworker|running|-\n")
            .unwrap()
            .starts_with("2 container(s) no ar"));
        // Resposta sem container nenhum é "não sei", não "está tudo bem".
        assert_eq!(
            estado_no_ar(&compose, "").unwrap_err(),
            "o docker não devolveu estado nenhum do container"
        );

        // PM2: o app declarado é o que decide, e um offline com pid 0 não está no ar.
        let pm2 = Alvo { diretorio: "/srv/app".into(), servico: "api".into(), ..Alvo::novo("pm2") };
        let json = r#"[{"name":"api","pid":0,"pm2_env":{"status":"offline","restart_time":3,"pm_uptime":0}},{"name":"worker","pid":9,"pm2_env":{"status":"online","restart_time":0,"pm_uptime":1}}]"#;
        assert!(estado_no_ar(&pm2, json).unwrap_err().contains("api não está no ar"));
        let no_ar = r#"[{"name":"api","pid":412,"pm2_env":{"status":"online","restart_time":1,"pm_uptime":9}}]"#;
        assert_eq!(
            estado_no_ar(&pm2, no_ar).unwrap(),
            "pm2 · api · online · pid 412 · 1 reinicio(s)"
        );
        // Ausente da lista é outra frase, e a lista vazia do pm2 cai aqui.
        assert!(estado_no_ar(&pm2, "[]").unwrap_err().contains("não está na lista"));
        assert_eq!(
            estado_no_ar(&pm2, "sem json").unwrap_err(),
            "o pm2 não devolveu a lista de processos"
        );

        // O motivo dito pelo roteiro chega com as palavras dele.
        assert_eq!(
            parse_razao("RAZAO:a imagem x não existe mais nesta máquina\n"),
            Some("a imagem x não existe mais nesta máquina".to_string())
        );
        assert_eq!(parse_razao("sem marcador\n"), None);
        assert!(estado_no_ar(&alvo, "RAZAO:o app api caiu\n").unwrap_err() == "o app api caiu");
    }

    #[test]
    fn muitos_containers_sao_mostrados_sem_estourar_a_linha() {
        let compose = Alvo { diretorio: "/srv/site".into(), ..Alvo::novo("compose") };
        let seis = (1..=6)
            .map(|i| format!("s{i}|running|-"))
            .collect::<Vec<_>>()
            .join("\n");
        let msg = estado_no_ar(&compose, &seis).unwrap();
        assert!(msg.starts_with("6 container(s) no ar"), "{msg}");
        assert!(msg.contains("s1 · running"), "{msg}");
        assert!(msg.contains("… 1"), "{msg}");
        assert!(!msg.contains("s6 · running"), "{msg}");
    }

    #[test]
    fn a_lista_do_pm2_e_lida_e_o_estado_manda() {
        let json = r#"[{"name":"api","pid":412,"pm2_env":{"status":"online","restart_time":2,"pm_uptime":1}},{"name":"worker","pid":0,"pm2_env":{"status":"errored","restart_time":7,"pm_uptime":0}}]"#;
        let lista = parse_pm2(json).unwrap();
        assert_eq!(lista.len(), 2);
        assert!(achar_pm2(&lista, "api").unwrap().no_ar());
        assert_eq!(achar_pm2(&lista, "api").unwrap().resumo(), "api · online · pid 412 · 2 reinicio(s)");
        assert!(!achar_pm2(&lista, "worker").unwrap().no_ar());
        assert!(achar_pm2(&lista, "inexistente").is_none());

        // Resposta que não é lista: "não sei", e o motor diz isso em vez de "não tem".
        assert_eq!(
            parse_pm2("command not found: pm2").unwrap_err(),
            "o pm2 não devolveu a lista de processos"
        );
        assert!(parse_pm2("[isso não é json]").is_err());
    }

    #[test]
    fn conferir_porta_aceita_as_formas_do_docker_run() {
        assert!(conferir_porta("80").is_ok());
        assert!(conferir_porta("8080:80").is_ok());
        assert!(conferir_porta("127.0.0.1:9000:9000/tcp").is_ok());
        assert!(conferir_porta("53:53/udp").is_ok());
        assert!(conferir_porta("").is_err());
        assert!(conferir_porta("8080:oitenta").is_err());
        assert!(conferir_porta("0:80").is_err());
        assert!(conferir_porta("80/http").is_err());
        assert!(conferir_porta("1:2:3:4").is_err());
    }

    #[test]
    fn o_modelo_recusa_o_que_o_motor_recusaria() {
        // Docker sem imagem.
        let sem_imagem = Alvo::novo("docker");
        assert!(sem_imagem.conferir().unwrap_err().contains("falta o diretório"));
        let mut alvo = docker();
        alvo.diretorio.clear();
        assert!(alvo.conferir().unwrap_err().contains("falta o diretório"));
        alvo.diretorio = "/var/www/site".into();
        alvo.imagem = String::new();
        assert!(alvo.conferir().unwrap_err().contains("nome da imagem"));
        alvo.imagem = "Meu App".into();
        assert!(alvo.conferir().unwrap_err().contains("minúsculas"));
        alvo.imagem = "a//b".into();
        assert!(alvo.conferir().unwrap_err().contains("barra ou dois-pontos"));
        alvo.imagem = "meuapp".into();
        alvo.dockerfile = "/etc/Dockerfile".into();
        assert!(alvo.conferir().unwrap_err().contains("não um caminho absoluto"));
        alvo.dockerfile = "Dockerfile".into();
        alvo.portas = vec!["99999999:80".into()];
        assert!(alvo.conferir().unwrap_err().contains("porta"));
        alvo.portas = vec![];
        alvo.conferir().unwrap();

        // PM2 sem app nomeado não tem o que recarregar.
        let mut pm2 = Alvo { diretorio: "/srv/app".into(), ..Alvo::novo("pm2") };
        assert!(pm2.conferir().unwrap_err().contains("pm2 list"));
        pm2.servico = "api".into();
        pm2.conferir().unwrap();

        // Executor inventado é recusado nos dois lados.
        assert!(validate_runtime("kubernetes").is_err());
        assert!(Alvo::novo("kubernetes").conferir().is_err());
    }

    #[test]
    fn a_tag_da_corrida_nao_pode_quebrar_o_nome_da_imagem() {
        let alvo = Alvo::novo("docker").com_tag("V7 f402 43ff/");
        assert_eq!(alvo.tag, "v7-f402-43ff");
        assert_eq!(alvo.imagem_tag(), ":v7-f402-43ff");
        assert_eq!(Alvo::novo("docker").com_tag("---").tag, "");
    }

    #[test]
    fn o_nome_do_container_vem_do_servico_ou_da_imagem() {
        assert_eq!(docker().container(), "site");
        let so_imagem = Alvo {
            imagem: "registry.local/time/meuapp".into(),
            ..Alvo::novo("docker")
        };
        assert_eq!(so_imagem.container(), "meuapp");
    }

    #[test]
    fn cada_executor_declara_as_ferramentas_que_a_sonda_procura() {
        assert_eq!(ferramentas("docker"), &["docker"]);
        assert_eq!(ferramentas("compose"), &["docker"]);
        assert_eq!(ferramentas("pm2"), &["pm2", "node"]);
        assert_eq!(ferramentas("script"), &[] as &[&str]);
    }

    #[test]
    fn a_espera_do_container_cabe_no_prazo_da_etapa() {
        // Cada volta do laço dorme 2 s: com prazo de 10 s não dá para prometer 8
        // leituras, o motor mataria o roteiro no meio da espera.
        assert_eq!(tentativas_de_espera(10), 2);
        assert_eq!(tentativas_de_espera(20), 5);
        assert_eq!(tentativas_de_espera(600), 8);
        assert_eq!(tentativas_de_espera(0), 2);
        let alvo = Alvo::da_definicao(
            &Definicao { runtime: "docker", imagem: Some("meuapp"), dockerfile: None, compose_file: None, servico: None, portas: &[] },
            "/var/www/site",
            20,
        );
        assert!(alvo.roteiro_subir().unwrap().contains("while [ $para -lt 5 ]"));
    }

    #[test]
    fn as_colunas_vazias_recebem_o_default_do_executor() {
        let portas = [" 8080:80 ".to_string(), String::new()];
        let alvo = Alvo::da_definicao(
            &Definicao {
                runtime: "COMPOSE",
                imagem: None,
                dockerfile: Some("  "),
                compose_file: None,
                servico: Some(" web "),
                portas: &portas,
            },
            "/srv/site",
            600,
        );
        assert_eq!(alvo.runtime, "compose");
        assert_eq!(alvo.compose_file, COMPOSE_PADRAO);
        assert_eq!(alvo.dockerfile, DOCKERFILE_PADRAO);
        assert_eq!(alvo.servico, "web");
        assert_eq!(alvo.portas, vec!["8080:80".to_string()]);
        assert_eq!(alvo.espera_saude, 8);
    }
}
