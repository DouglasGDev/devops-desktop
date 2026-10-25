import { useSyncExternalStore } from "react";
import type { Server, ServerMetrics } from "../types";
import { api } from "./api";

export type Leitura = {
  loading: boolean;
  metrics: ServerMetrics | null;
  error: string | null;
  /** Epoch em milissegundos da última resposta boa; null quando nunca houve. */
  readAt: number | null;
};

export type Frota = {
  active: boolean;
  total: number;
  done: number;
  current: number | null;
  failures: number;
  /** Servidores com senha ficaram fora: a leitura em frota não abre prompt. */
  skipped: number;
  cancelled: boolean;
  /** Pedido de parada recebido; a leitura em curso termina antes. */
  stopping: boolean;
};

type Estado = { readings: Record<number, Leitura>; fleet: Frota };

const FROTA_VAZIA: Frota = {
  active: false,
  total: 0,
  done: 0,
  current: null,
  failures: 0,
  skipped: 0,
  cancelled: false,
  stopping: false,
};

let estado: Estado = { readings: {}, fleet: FROTA_VAZIA };
const listeners = new Set<() => void>();
const pedidos = new Map<number, Promise<Leitura>>();
let cancelRequested = false;

function emitir(next: Estado) {
  estado = next;
  for (const listener of listeners) listener();
}

function comLeitura(id: number, patch: Partial<Leitura>) {
  const atual = estado.readings[id] ?? { loading: false, metrics: null, error: null, readAt: null };
  emitir({ ...estado, readings: { ...estado.readings, [id]: { ...atual, ...patch } } });
}

function comFrota(patch: Partial<Frota>) {
  emitir({ ...estado, fleet: { ...estado.fleet, ...patch } });
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** As leituras vivem fora da árvore React: trocar de tela não as perde. */
export function useInfraMonitor(): Estado {
  return useSyncExternalStore(subscribe, () => estado);
}

export async function lerServidor(server: Server): Promise<Leitura> {
  const emVoo = pedidos.get(server.id);
  if (emVoo) return emVoo;
  const pedido = executarLeitura(server).finally(() => pedidos.delete(server.id));
  pedidos.set(server.id, pedido);
  return pedido;
}

async function executarLeitura(server: Server): Promise<Leitura> {
  comLeitura(server.id, { loading: true, error: null });
  try {
    const metrics = await api.serverMetrics(server.id);
    comLeitura(server.id, { loading: false, metrics, error: null, readAt: Date.now() });
  } catch (e) {
    // A última leitura boa continua visível; o erro diz que ela não foi renovada.
    comLeitura(server.id, { loading: false, error: String(e) });
  }
  return estado.readings[server.id];
}

/**
 * Lê a frota em sequência. Servidores com autenticação por senha ficam de fora:
 * a coleta em lote não abre prompt sem a pessoa pedir aquele servidor, e o
 * resumo informa quantos ficaram fora e por quê.
 */
export async function lerFrota(servers: Server[]): Promise<void> {
  if (estado.fleet.active) return;
  cancelRequested = false;
  const alvo = servers.filter((s) => s.authMethod !== "password");
  comFrota({
    active: true,
    total: alvo.length,
    done: 0,
    current: null,
    failures: 0,
    skipped: servers.length - alvo.length,
    cancelled: false,
    stopping: false,
  });

  for (const server of alvo) {
    if (cancelRequested) break;
    comFrota({ current: server.id });
    await lerServidor(server);
    comFrota({
      done: estado.fleet.done + 1,
      failures: estado.fleet.failures + (estado.readings[server.id]?.error ? 1 : 0),
    });
  }

  const cancelled = cancelRequested;
  cancelRequested = false;
  comFrota({ active: false, current: null, cancelled, stopping: false });
}

export function pararFrota() {
  cancelRequested = true;
  comFrota({ stopping: true });
}
