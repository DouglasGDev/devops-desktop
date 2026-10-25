import type { NodeAnalyzer, SeverityGate } from "../types";

/** Os instrumentos que o motor sabe varrer, nos mesmos tokens do backend. */
export const INSTRUMENTOS: { valor: NodeAnalyzer; rotulo: string }[] = [
  { valor: "embutido", rotulo: "Embutido (regras locais)" },
  { valor: "semgrep", rotulo: "Semgrep (regras padrão)" },
  { valor: "ambos", rotulo: "Embutido + Semgrep (regras padrão)" },
];

// Os rótulos do corte são a regra de `barra_o_corte`, dita com os números dela.
export const CORTES: { valor: SeverityGate; rotulo: string }[] = [
  { valor: "", rotulo: "Nada barra" },
  { valor: "ERROR", rotulo: "Barra se houver erro" },
  { valor: "WARNING", rotulo: "Barra se houver aviso ou erro" },
  { valor: "INFO", rotulo: "Barra se houver qualquer achado" },
];
