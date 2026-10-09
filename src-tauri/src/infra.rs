//! Métricas reais de um servidor, colhidas por SSH com comandos de leitura de
//! baixo impacto (§12). Nada aqui é estimado nem simulado: cada número vem da
//! saída de um comando executado naquele host, e o que o host não tem é
//! reportado como ausente.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::Serialize;

use crate::auth::AskPass;
use crate::models::Server;
use crate::sshexec;

/// Teto da coleta. Os comandos são locais ao servidor e terminam em segundos;
/// o prazo existe para uma conexão lenta não travar a tela.
const COLLECT_TIMEOUT_SECS: u64 = 25;
/// Um alerta de disco acima deste percentual (§12 “alertas de configuração”).
const DISK_WARN_PERCENT: u64 = 85;
const MEM_WARN_PERCENT: u64 = 90;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub avail_kb: u64,
    pub use_percent: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub name: String,
    /// "container" (docker) ou "serviço" (systemd).
    pub kind: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerMetrics {
    pub server_id: i64,
    pub server_name: String,
    /// Epoch em milissegundos do instante em que a leitura terminou.
    pub collected_at: i64,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    pub cpu_count: Option<u32>,
    pub cpu_model: Option<String>,
    pub load1: Option<f64>,
    pub load5: Option<f64>,
    pub load15: Option<f64>,
    pub uptime_seconds: Option<u64>,
    pub mem_total_kb: Option<u64>,
    pub mem_available_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
    pub swap_free_kb: Option<u64>,
    pub disks: Vec<DiskUsage>,
    pub services: Vec<ServiceInfo>,
    /// Coisas que o host tem (ou não) e que explicam por que um campo está vazio.
    pub notes: Vec<String>,
    /// Avisos derivados dos números lidos — nunca de um valor inventado.
    pub warnings: Vec<String>,
}

/// O script de leitura. POSIX sh (não presume bash), sem escrita alguma, e cada
/// ferramenta é verificada antes de ser usada (§9).
pub fn metrics_script() -> String {
    [
        "set -u",
        "echo \"host $(hostname 2>/dev/null || cat /proc/sys/kernel/hostname 2>/dev/null)\"",
        "echo \"kernel $(uname -r 2>/dev/null)\"",
        "echo \"arch $(uname -m 2>/dev/null)\"",
        "echo \"os $(sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | tr -d '\\\"' | head -1)\"",
        "echo \"uptime $(cut -d' ' -f1 /proc/uptime 2>/dev/null)\"",
        "echo \"load $(awk '{print $1\" \"$2\" \"$3}' /proc/loadavg 2>/dev/null)\"",
        "echo \"cpus $(nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null)\"",
        "echo \"cpumodel $(sed -n 's/^model name *: //p' /proc/cpuinfo 2>/dev/null | head -1)\"",
        "echo \"memtotal $(awk '/^MemTotal:/{print $2}' /proc/meminfo 2>/dev/null)\"",
        "echo \"memavail $(awk '/^MemAvailable:/{print $2}' /proc/meminfo 2>/dev/null)\"",
        "echo \"swaptotal $(awk '/^SwapTotal:/{print $2}' /proc/meminfo 2>/dev/null)\"",
        "echo \"swapfree $(awk '/^SwapFree:/{print $2}' /proc/meminfo 2>/dev/null)\"",
        "df -Pk 2>/dev/null | awk 'NR>1 && $2>0 {out=\"disk \"$2\" \"$3\" \"$4; for (i=6;i<=NF;i++) out=out\" \"$i; print out}'",
        "if command -v docker >/dev/null 2>&1; then",
        "  echo \"docker presente\"",
        "  docker ps --format 'ctr {{.Names}}|{{.Status}}' 2>/dev/null",
        "else",
        "  echo \"docker ausente\"",
        "fi",
        "if command -v systemctl >/dev/null 2>&1; then",
        "  echo \"systemd presente\"",
        "  systemctl list-units --type=service --state=running --no-legend --no-pager 2>/dev/null | awk '{print \"svc \"$1}'",
        "else",
        "  echo \"systemd ausente\"",
        "fi",
    ]
    .join("\n")
        + "\n"
}

/// Valor de uma linha `chave valor…`; `-` e vazio significam “não informado”.
fn field(map: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != "-")
}

fn number(map: &std::collections::HashMap<String, String>, key: &str) -> Option<u64> {
    field(map, key)?.parse().ok()
}

fn decimal(map: &std::collections::HashMap<String, String>, key: &str) -> Option<f64> {
    field(map, key)?.parse().ok()
}

/// Interpreta a saída do script de métricas. `disk` é o único caso com campos
/// variáveis: três números e o ponto de montagem no resto da linha.
pub fn parse_metrics(server_id: i64, server_name: &str, raw: &str) -> ServerMetrics {
    let mut fields: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut disks = Vec::new();
    let mut services = Vec::new();
    let mut notes = Vec::new();

    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let rest = line[line.find(key).unwrap_or(0) + key.len()..].trim().to_string();
        match key {
            "disk" => {
                let mut nums = rest.splitn(4, ' ');
                let (Some(total), Some(used), Some(avail), Some(mount)) =
                    (nums.next(), nums.next(), nums.next(), nums.next())
                else {
                    continue;
                };
                let (Ok(total), Ok(used), Ok(avail)) =
                    (total.parse::<u64>(), used.parse::<u64>(), avail.parse::<u64>())
                else {
                    continue;
                };
                if total == 0 {
                    continue;
                }
                disks.push(DiskUsage {
                    mount: mount.to_string(),
                    total_kb: total,
                    used_kb: used,
                    avail_kb: avail,
                    use_percent: (used * 100 + total / 2) / total,
                });
            }
            "ctr" => {
                let (name, state) = match rest.split_once('|') {
                    Some((n, s)) => (n.to_string(), s.to_string()),
                    None => (rest.clone(), String::new()),
                };
                if !name.is_empty() {
                    services.push(ServiceInfo {
                        name,
                        kind: "container".to_string(),
                        state,
                    });
                }
            }
            "svc" => {
                if !rest.is_empty() {
                    services.push(ServiceInfo {
                        name: rest.clone(),
                        kind: "serviço".to_string(),
                        state: "running".to_string(),
                    });
                }
            }
            "load" => {
                // três números na mesma linha: $1 $2 $3 de /proc/loadavg
                for (i, value) in rest.split_whitespace().take(3).enumerate() {
                    fields.insert(format!("load{}", i + 1), value.to_string());
                }
            }
            "docker" => notes.push(match rest.as_str() {
                "ausente" => "Docker não está instalado neste servidor.".to_string(),
                _ => "Docker disponível; a lista acima é a dos containers em execução.".to_string(),
            }),
            "systemd" => notes.push(match rest.as_str() {
                "ausente" => "systemd não está disponível; serviços não puderam ser listados.".to_string(),
                _ => String::new(),
            }),
            _ => {
                fields.insert(key.to_string(), rest);
            }
        }
    }
    notes.retain(|n| !n.is_empty());

    let cpu_count = number(&fields, "cpus").map(|v| v as u32);
    let load1 = decimal(&fields, "load1");
    let load5 = decimal(&fields, "load2");
    let load15 = decimal(&fields, "load3");
    let mem_total_kb = number(&fields, "memtotal");
    let mem_available_kb = number(&fields, "memavail");

    let mut warnings = Vec::new();
    for disk in &disks {
        if disk.use_percent >= DISK_WARN_PERCENT {
            warnings.push(format!(
                "disco {} com {}% usado ({} livres)",
                disk.mount,
                disk.use_percent,
                human_kb(disk.avail_kb)
            ));
        }
    }
    if let (Some(total), Some(avail)) = (mem_total_kb, mem_available_kb) {
        let percentual = total.saturating_sub(avail).checked_mul(100).and_then(|v| v.checked_div(total));
        if let Some(used_percent) = percentual {
            if used_percent >= MEM_WARN_PERCENT {
                warnings.push(format!(
                    "memória com {used_percent}% usada — restam {} disponíveis",
                    human_kb(avail)
                ));
            }
        }
    }
    if let (Some(load), Some(cpus)) = (load1, cpu_count) {
        if cpus > 0 && load > cpus as f64 * 1.5 {
            warnings.push(format!(
                "carga média {load:.2} acima de 1,5× o número de núcleos ({cpus})"
            ));
        }
    }
    if disks.is_empty() {
        warnings.push("o servidor não informou nenhum sistema de arquivos legível via df".to_string());
    }

    ServerMetrics {
        server_id,
        server_name: server_name.to_string(),
        collected_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default(),
        hostname: field(&fields, "host"),
        os: field(&fields, "os"),
        kernel: field(&fields, "kernel"),
        arch: field(&fields, "arch"),
        cpu_count,
        cpu_model: field(&fields, "cpumodel"),
        load1,
        load5,
        load15,
        // /proc/uptime traz fração de segundo: “350120.45”
        uptime_seconds: decimal(&fields, "uptime").map(|v| v as u64),
        mem_total_kb,
        mem_available_kb,
        swap_total_kb: number(&fields, "swaptotal"),
        swap_free_kb: number(&fields, "swapfree"),
        disks,
        services,
        notes,
        warnings,
    }
}

/// KB convertidos para a unidade que caiba na tela.
pub fn human_kb(kb: u64) -> String {
    const GB: u64 = 1024 * 1024;
    if kb >= GB {
        format!("{:.1} GiB", kb as f64 / GB as f64)
    } else if kb >= 1024 {
        format!("{:.0} MiB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KiB")
    }
}

/// Roda a leitura no servidor e interpreta a saída.
pub fn collect(
    server: &Server,
    askpass: Option<&AskPass>,
) -> Result<ServerMetrics, String> {
    let never = AtomicBool::new(false);
    let result = sshexec::run_script(
        server,
        askpass,
        "sh",
        &metrics_script(),
        &[],
        Duration::from_secs(COLLECT_TIMEOUT_SECS),
        &never,
    )?;
    if result.timed_out {
        return Err(format!(
            "a leitura de métricas em “{}” não terminou em {COLLECT_TIMEOUT_SECS}s",
            server.name
        ));
    }
    if result.cancelled {
        return Err(format!("a leitura de métricas em “{}” foi interrompida", server.name));
    }
    if !result.ok() {
        return Err(format!(
            "o servidor não devolveu as métricas: {}",
            result.summary()
        ));
    }
    Ok(parse_metrics(server.id, &server.name, &result.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
host web-01
kernel 6.8.0-45-generic
arch x86_64
os Ubuntu 24.04.1 LTS
uptime 350120.45
load 0.42 0.51 0.60
cpus 4
cpumodel Intel(R) Xeon(R) Platinum 8370C
memtotal 7984124
memavail 2013456
swaptotal 2097148
swapfree 2097148
disk 8120852 6123400 1564320 /
disk 5120852 123400 4765320 /var/log
ctr api|Up 3 hours
ctr worker|Up 12 minutes
docker presente
systemd presente
svc nginx.service
svc ssh.service
";

    #[test]
    fn le_cabecalho_de_maquina_e_sistema() {
        let m = parse_metrics(1, "web", SAMPLE);
        assert_eq!(m.hostname.as_deref(), Some("web-01"));
        assert_eq!(m.os.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(m.kernel.as_deref(), Some("6.8.0-45-generic"));
        assert_eq!(m.cpu_count, Some(4));
        assert_eq!(m.load1, Some(0.42));
        assert_eq!(m.load15, Some(0.6));
        assert_eq!(m.uptime_seconds, Some(350120));
        assert_eq!(m.mem_total_kb, Some(7984124));
        assert_eq!(m.mem_available_kb, Some(2013456));
    }

    #[test]
    fn discos_tem_percentual_e_ponto_de_montagem() {
        let m = parse_metrics(1, "web", SAMPLE);
        assert_eq!(m.disks.len(), 2);
        assert_eq!(m.disks[0].mount, "/");
        // usado/total: 6.123.400 de 8.120.852 KiB = 75%
        assert_eq!(m.disks[0].use_percent, 75);
        assert_eq!(m.disks[1].mount, "/var/log");
        assert_eq!(m.disks[1].use_percent, 2);
        // 75% não passa do limite de 85%: nenhum alerta de disco aqui.
        assert!(!m.warnings.iter().any(|w| w.starts_with("disco / ")), "{:?}", m.warnings);
    }

    #[test]
    fn containers_e_servicos_sao_lidos_por_linha() {
        let m = parse_metrics(1, "web", SAMPLE);
        let api = m.services.iter().find(|s| s.name == "api").expect("container api");
        assert_eq!(api.kind, "container");
        assert_eq!(api.state, "Up 3 hours");
        let worker = m.services.iter().find(|s| s.name == "worker").expect("worker");
        assert_eq!(worker.state, "Up 12 minutes", "o separador não pode quebrar a linha");
        assert_eq!(m.services.iter().filter(|s| s.kind == "serviço").count(), 2);
        assert!(
            m.notes.iter().any(|n| n.contains("Docker disponível")),
            "{:?}",
            m.notes
        );
        assert_eq!(m.notes.len(), 1, "systemd presente não gera nota: {:?}", m.notes);
    }

    #[test]
    fn ferramentas_ausentes_aparecem_como_nota_e_nada_inventado() {
        let raw = "host only\nkernel 1.0\nuptime 10\nload 0.1 0.1 0.1\ndocker ausente\nsystemd ausente\n";
        let m = parse_metrics(2, "minimal", raw);
        assert_eq!(m.hostname.as_deref(), Some("only"));
        assert!(m.mem_total_kb.is_none());
        assert!(m.disks.is_empty());
        assert!(m.services.is_empty());
        assert!(m.notes.iter().any(|n| n.contains("Docker")), "{:?}", m.notes);
        assert!(m.notes.iter().any(|n| n.contains("systemd")), "{:?}", m.notes);
        assert!(m.warnings.iter().any(|w| w.contains("df")), "{:?}", m.warnings);
    }

    #[test]
    fn alertas_veem_disco_cheio_memoria_alta_e_carga() {
        let raw = "\
host cheio
load 12.0 11.0 10.0
cpus 2
memtotal 1000
memavail 50
disk 1000 950 50 /data
";
        let m = parse_metrics(3, "apertado", raw);
        assert!(m.warnings.iter().any(|w| w.contains("/data")), "{:?}", m.warnings);
        assert!(m.warnings.iter().any(|w| w.contains("memória")), "{:?}", m.warnings);
        assert!(m.warnings.iter().any(|w| w.contains("carga")), "{:?}", m.warnings);
    }

    #[test]
    fn linhas_sem_valor_nao_travam_a_leitura() {
        let m = parse_metrics(4, "vazio", "host \nos -\ncpus abc\ndisk x y z /\n");
        assert_eq!(m.hostname, None);
        assert_eq!(m.os, None);
        assert_eq!(m.cpu_count, None);
        assert!(m.disks.is_empty());
    }

    #[test]
    fn conversao_de_unidades_de_disco_e_memoria() {
        assert_eq!(human_kb(512), "512 KiB");
        assert_eq!(human_kb(4096), "4 MiB");
        assert_eq!(human_kb(1024 * 1024 * 3), "3.0 GiB");
    }

    #[test]
    fn script_nao_escreve_nem_presume_ferramentas() {
        let script = metrics_script();
        for proibido in ["rm ", "mv ", "touch ", "echo -e", " apt ", "yum", "install"] {
            assert!(!script.contains(proibido), "o script não pode conter {proibido:?}");
        }
        // O único destino de redirecionamento permitido é o buraco negro. As
        // demais ocorrências de `>` e `<` no script são comparações do awk.
        let mut sobra = metrics_script();
        for segura in ["2>/dev/null", ">/dev/null 2>&1", "2>&1", "NR>1", "$2>0", "i<=NF"] {
            sobra = sobra.replace(segura, "");
        }
        assert!(!sobra.contains('>'), "sobrou um redirecionamento: {sobra}");
        assert!(!sobra.contains('<'), "a leitura não consome stdin");
        assert!(script.contains("command -v docker"), "docker é verificado antes de usado");
        assert!(script.contains("df -Pk"), "df em modo POSIX");
        assert!(script.contains("2>/dev/null"));
    }
}
