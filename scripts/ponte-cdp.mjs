// Piloto de teste de interface: fala com o WebView2 do aplicativo pelo CDP e
// roda um script dentro da janela real. exige o executável aberto com a variável
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 (passo a passo em
// docs/ENTREGAS.md, §4.1). Uso: node scripts/ponte-cdp.mjs "<expressao>" — o texto é o corpo
// de uma função async, então `return` funciona, e `window.__TAURI_INTERNALS__.invoke` e
// `document` estão disponíveis.
const alvo = process.argv[2];
if (!alvo) {
  console.error("diga a expressão JavaScript a avaliar");
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
const resposta = await chamar("Runtime.evaluate", {
  expression: `(async () => { ${alvo} })()`,
  awaitPromise: true,
  returnByValue: true,
});
const valor = resposta?.result?.value ?? resposta;
console.log(typeof valor === "string" ? valor : JSON.stringify(valor, null, 2).slice(0, 6000));
if (resposta?.exceptionDetails) console.error("exceção na janela:", JSON.stringify(resposta.exceptionDetails));
ws.close();
