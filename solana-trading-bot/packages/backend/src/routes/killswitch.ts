import type { FastifyInstance } from "fastify";
import { isPaused, setPaused } from "../execution/simulator.js";
import { broadcast } from "../ws/hub.js";

export async function killswitchRoutes(app: FastifyInstance): Promise<void> {
  app.get("/api/killswitch", async () => ({ paused: isPaused() }));

  app.post("/api/killswitch/pause", async () => {
    setPaused(true);
    broadcast({ type: "engine_status", data: { paused: true } });
    return { paused: true };
  });

  app.post("/api/killswitch/resume", async () => {
    setPaused(false);
    broadcast({ type: "engine_status", data: { paused: false } });
    return { paused: false };
  });
}
