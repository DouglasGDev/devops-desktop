// Piloto de teste de interface: fala com o WebView2 do aplicativo pelo CDP e
// roda um script dentro da janela real. exige o executável aberto com a variável
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 (passo a passo em
// docs/ENTREGAS.md, §4.1). Uso:
//   node scripts/ponte-cdp.mjs "<expressao>"                — o texto é o corpo de uma
//     função async, então `return` funciona, e `window.__TAURI_INTERNALS__.invoke` e
//     `document` estão disponíveis.
//   node scripts/ponte-cdp.mjs --mouse <tipo> <x> <y>       — entrada de mouse real.
//   node scripts/ponte-cdp.mjs --click-node <chave>         — seleciona um nó do grafo.
//   node scripts/ponte-cdp.mjs --drag-node <chave> <dx> <dy> — arrasta um nó do grafo.
// Os dois últimos medem a posição na mesma sessão em que clicam: entre duas chamadas
// separadas a janela pode ser redimensionada ou o canvas rolado, e o clique cairia no
// nó errado. Evento sintético de página não dirige o arrasto — setPointerCapture
// rejeita um ponteiro que o navegador não criou.
const alvo = process.argv[2];
if (!alvo) {
  console.error("diga a expressão JavaScript a avaliar, ou --mouse, --click-node, --drag-node");
  process.exit(2);
}
const lista = await (await fetch("http://127.0.0.1:9223/json/list")).json();
const pagina = lista.find((t) => t.type === "page" && t.url.includes("localhost"));
if (!pagina) {
  console.error("nenhuma página do aplicativo no CDP:", lista.map((t) => `${t.type} ${t.url}`));
  process.exit(1);
}
const ws = new WebSocket(pagina.webSocketDebuggerUrl);
await new Promise((ok, erro) => {
  ws.onopen = ok;
  ws.onerror = () => erro(new Error("websocket falhou"));
});
let id = 0;
function chamar(metodo, params) {
  const meu = ++id;
  return new Promise((ok, erro) => {
    const aoResponder = (evento) => {
      const dados = JSON.parse(evento.data);
      if (dados.id !== meu) return;
      ws.removeEventListener("message", aoResponder);
      if (dados.error) erro(new Error(JSON.stringify(dados.error)));
      else ok(dados.result);
    };
    ws.addEventListener("message", aoResponder);
    ws.send(JSON.stringify({ id: meu, method: metodo, params }));
  });
}

const TIPOS = { pressed: "mousePressed", released: "mouseReleased", moved: "mouseMoved", wheel: "mouseWheel" };
async function mouse(tipo, x, y) {
  if (!TIPOS[tipo]) {
    console.error(`tipo de mouse desconhecido: ${tipo}`);
    process.exit(2);
  }
  return chamar("Input.dispatchMouseEvent", {
    type: TIPOS[tipo],
    x,
    y,
    button: "left",
    clickCount: 1,
    buttons: tipo === "released" ? 0 : 1,
  });
}
async function avalie(corpo) {
  return chamar("Runtime.evaluate", {
    expression: `(async () => { ${corpo} })()`,
    awaitPromise: true,
    returnByValue: true,
  });
}
/** O centro de um nó do grafo, trazido para dentro da janela antes de medir. */
async function localDoNo(chave) {
  const corpo = `const g = [...document.querySelectorAll("svg g")].find((e) => e.querySelector("text")?.textContent.startsWith(${JSON.stringify(
    chave || ""
  )}));
   if (!g) return { erro: "nó não está na tela" };
   g.scrollIntoView({ block: "center", inline: "center" });
   await new Promise((r) => setTimeout(r, 80));
   const r = g.getBoundingClientRect();
   return { x: r.x + r.width / 2, y: r.y + r.height / 2, antes: g.getAttribute("transform") };`;
  return (await avalie(corpo)).result?.value;
}
/** O título do painel do nó selecionado, que é o que a tela mostra de volta. */
async function painel() {
  const corpo = `const titulo = [...document.querySelectorAll("p,h3,h4")].map((e) => e.textContent.trim()).filter((t) => t.startsWith("Nó "))[0] ?? null;
   const g = [...document.querySelectorAll("svg g")].map((e) => e.getAttribute("transform"));
   return JSON.stringify({ painel: titulo, transforms: g });`;
  return (await avalie(corpo)).result?.value;
}

let resposta;
if (alvo === "--mouse") {
  const [tipo, x, y] = process.argv.slice(3);
  if (!tipo || !Number.isFinite(Number(x)) || !Number.isFinite(Number(y))) {
    console.error("uso: --mouse <pressed|released|moved|wheel> <x> <y>");
    process.exit(2);
  }
  await mouse(tipo, Number(x), Number(y));
  console.log("ok");
} else if (alvo === "--click-node" || alvo === "--drag-node") {
  const [chave, dx, dy] = process.argv.slice(3);
  const arrastando = alvo === "--drag-node";
  if (arrastando && (!Number.isFinite(Number(dx)) || !Number.isFinite(Number(dy)))) {
    console.error("uso: --drag-node <chave> <dx> <dy>");
    process.exit(2);
  }
  const local = await localDoNo(chave);
  if (!local || local.erro) {
    console.log(JSON.stringify(local ?? { erro: "sem resposta" }));
    process.exit(1);
  }
  await mouse("moved", local.x, local.y);
  await mouse("pressed", local.x, local.y);
  if (arrastando) {
    // Dois movimentos intermediários: um nó só é marcado como arrastado depois que o
    // ponteiro se move, e um salto direto ao destino não exercita o mesmo caminho.
    await mouse("moved", local.x + Number(dx) / 2, local.y + Number(dy) / 2);
    await mouse("moved", local.x + Number(dx), local.y + Number(dy));
    await mouse("released", local.x + Number(dx), local.y + Number(dy));
  } else {
    await mouse("released", local.x, local.y);
  }
  console.log(JSON.stringify({ antes: local.antes, centro: { x: local.x, y: local.y } }));
  console.log(await painel());
} else {
  resposta = await avalie(alvo);
}
const valor = resposta?.result?.value ?? resposta;
if (valor !== undefined) console.log(typeof valor === "string" ? valor : JSON.stringify(valor, null, 2).slice(0, 6000));
if (resposta?.exceptionDetails) console.error("exceção na janela:", JSON.stringify(resposta.exceptionDetails));
ws.close();
