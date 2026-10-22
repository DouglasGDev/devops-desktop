import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Sonda da máscara: extrai as funções reais de security.rs/analysis.rs, compila
// com rustc e roda. É reprodução fora do app — prova a mecânica, nunca a tela.
const raiz = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const saida = path.join(raiz, "src-tauri", "target");
const security = fs.readFileSync(path.join(raiz, "src-tauri/src/security.rs"), "utf8");
const analysis = fs.readFileSync(path.join(raiz, "src-tauri/src/analysis.rs"), "utf8");

function secao(texto, inicio, fim) {
  const primeiro = texto.indexOf(inicio);
  const ultimo = texto.indexOf(fim, primeiro + inicio.length);
  if (primeiro < 0 || ultimo < 0) throw new Error(`Trecho não encontrado: ${inicio}`);
  return texto.slice(primeiro, ultimo);
}

const lista = (constante) => {
  const i = security.indexOf(`const ${constante}:`);
  if (i < 0) throw new Error(`Constante não encontrada: ${constante}`);
  return security.slice(i, security.indexOf("];", i) + 2);
};

const prefixos = lista("PREFIXOS_TOKEN");
const chaves = lista("CHAVES_DE_SEGREDO");
const mecanica = secao(security, "const MARCA_DE_MASCARA", "fn content_findings(");
const cauda = secao(analysis, "fn cauda(", "/// Busca a linha que o semgrep apontou");

const casos = [
  ["password", "password = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["passwd", "passwd = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["secret", "secret = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["api_key", "api_key = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["apikey", "apikey = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["access_key", "access_key = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["auth_token", "auth_token = FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["caixa_alta_no_nome", 'DB_PASSWORD="FICTICIO_SENHA"', "FICTICIO_SENHA"],
  ["aspa_fechando_o_nome", '{"password": "FICTICIO_SENHA"}', "FICTICIO_SENHA"],
  ["valor_entre_aspas_com_espaco", 'password = "FICTICIO_UM FICTICIO_DOIS"', "FICTICIO_DOIS"],
  ["duas_atribuicoes_primeira", "secret = 'FICTICIO_UM'; api_key = \"FICTICIO_DOIS\"", "FICTICIO_UM"],
  ["duas_atribuicoes_segunda", "secret = 'FICTICIO_UM'; api_key = \"FICTICIO_DOIS\"", "FICTICIO_DOIS"],
  ["bearer_opaco", "curl -H 'Authorization: Bearer FICTICIO_SENHA'", "FICTICIO_SENHA"],
  ["bearer_depois_de_unicode", "İ Bearer FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["prefixo_sk_live_deixa_a_especie", 'stripe = "sk_live_FICTICIO_SENHA"', "FICTICIO_SENHA"],
  ["prefixo_ghp_deixa_a_especie", "APP_GITHUB=ghp_FICTICIO_SENHA", "FICTICIO_SENHA"],
  ["comparacao_nao_e_valor", "if password == FICTICIO_COMPARACAO_1 {", "FICTICIO_COMPARACAO_1"],
  ["referencia_de_ambiente_nao_e_valor", "password = ${DB_PASSWORD}", "DB_PASSWORD"],
  ["nome_sem_valor_fica_como_veio", "password =", "password ="],
  ["valor_curto_e_nome_de_variavel", "password = x", "password = x"],
  ["texto_comum_nao_e_tocado", "a senha do cliente nunca é enviada no corpo do pedido", "senha"],
];

const array = casos
  .map((linha) => `(${linha.map((valor) => JSON.stringify(valor)).join(", ")})`)
  .join(",\n");

// Cada linha é conferida por três propriedades: o valor some, o texto que tinha de
// ficar continua lá, e mascarar duas vezes escreve uma marca só.
const linhaStderr = String.raw`    let stderr = r#"falhou ao ler C:\config\a.py com password = "FICTICIO_VALOR_SOLTO_123""#;
`;
const main = `
fn main() {
    let casos = [${array}];
    let fica_a_vista = |nome: &str| ["comparacao_nao_e_valor", "referencia_de_ambiente_nao_e_valor",
        "nome_sem_valor_fica_como_veio", "valor_curto_e_nome_de_variavel", "texto_comum_nao_e_tocado"]
        .contains(&nome);
    let mut falhas = 0;
    for (nome, entrada, marcador) in casos {
        let uma = security::mascarar(entrada);
        let duas = security::mascarar(&uma);
        let escondido = !uma.contains(marcador);
        let a_vista = uma.contains(marcador) == fica_a_vista(nome);
        let idempotente = uma == duas;
        let ok = if fica_a_vista(nome) { a_vista && idempotente } else { escondido && a_vista && idempotente };
        println!("mascara {}: {} (escondido={}, a_vista={}, idempotente={}) -> {}", nome,
            if ok { "PASS" } else { "FAIL" }, escondido, a_vista, idempotente, uma.replace('\\n', " "));
        if !ok { falhas += 1; }
    }
    let texto = format!("é{}", "a".repeat(199));
    let sem_panic = std::panic::catch_unwind(|| cauda(&texto)).is_ok();
    println!("cauda_utf8_sem_panic: {}", if sem_panic { "PASS" } else { "FAIL" });
    if !sem_panic { falhas += 1; }
    let longo = format!("{}", "x".repeat(400));
    let cortada = cauda(&longo);
    let em_caractere = cortada.starts_with('…') && cortada.chars().count() == 201;
    println!("cauda_corta_em_caractere: {}", if em_caractere { "PASS" } else { "FAIL" });
    if !em_caractere { falhas += 1; }
${linhaStderr}    let escondido_na_cauda = !cauda(&stderr).contains("FICTICIO_VALOR_SOLTO_123");
    println!("cauda_mascara_o_stderr: {}", if escondido_na_cauda { "PASS" } else { "FAIL" });
    if !escondido_na_cauda { falhas += 1; }
    println!("falhas_reproduzidas={}", falhas);
    if falhas > 0 { std::process::exit(1); }
}
`;

const fonte = [
  "mod security {",
  prefixos,
  chaves,
  mecanica,
  "}",
  cauda,
  main,
].join("\n");

const exe = path.join(saida, "sonda-mascara.exe");
const compilar = spawnSync("rustc", ["--edition=2021", "-o", exe, "-"], {
  input: fonte,
  encoding: "utf8",
  timeout: 120000,
});
fs.writeFileSync(
  path.join(saida, "sonda-mascara-compile.log"),
  `${compilar.stdout || ""}${compilar.stderr || ""}\nstatus=${compilar.status}\nerro=${compilar.error?.message || "nenhum"}\n`
);
if (compilar.status !== 0) {
  console.log("Compilação da sonda falhou; leia src-tauri/target/sonda-mascara-compile.log");
  process.exit(compilar.status ?? 1);
}
const rodar = spawnSync(exe, [], { encoding: "utf8", timeout: 20000 });
console.log(`${rodar.stdout || ""}${rodar.stderr || ""}`);
process.exit(rodar.status ?? 1);
