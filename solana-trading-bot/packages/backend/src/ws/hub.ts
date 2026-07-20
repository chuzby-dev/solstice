import type { FastifyInstance } from "fastify";
import type { WebSocket } from "ws";
import type { WsMessage } from "@trading-bot/shared";
import { getPortfolioSnapshot, isPaused } from "../execution/simulator.js";

const clients = new Set<WebSocket>();

export function broadcast(message: WsMessage): void {
  const payload = JSON.stringify(message);
  for (const socket of clients) {
    if (socket.readyState === socket.OPEN) socket.send(payload);
  }
}

export async function registerWsHub(app: FastifyInstance): Promise<void> {
  app.get("/ws", { websocket: true }, (socket) => {
    clients.add(socket);
    // Prime the newly connected client with current state so the GUI doesn't have to
    // wait for the next tick to render a portfolio.
    socket.send(JSON.stringify({ type: "portfolio", data: getPortfolioSnapshot() } satisfies WsMessage));
    socket.send(JSON.stringify({ type: "engine_status", data: { paused: isPaused() } } satisfies WsMessage));

    socket.on("close", () => clients.delete(socket));
    socket.on("error", () => clients.delete(socket));
  });
}
