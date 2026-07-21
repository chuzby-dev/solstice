import { useEffect, useRef, useState } from 'react';
import { wsUrl } from './client';
import type { EngineEvent } from './types';

export type ConnectionState = 'connecting' | 'open' | 'closed';

interface EngineEventsState {
  connection: ConnectionState;
  events: EngineEvent[];
  latest: EngineEvent | null;
}

const MAX_BUFFERED_EVENTS = 200;
const RECONNECT_DELAY_MS = 2000;

/**
 * Subscribes to the live EngineEvent WebSocket stream, keeping a rolling
 * buffer of the most recent events plus the single latest one (for
 * components that only care about "what just happened"). Reconnects
 * automatically on close/error.
 */
export function useEngineEvents(): EngineEventsState {
  const [connection, setConnection] = useState<ConnectionState>('connecting');
  const [events, setEvents] = useState<EngineEvent[]>([]);
  const [latest, setLatest] = useState<EngineEvent | null>(null);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      if (cancelled) return;
      setConnection('connecting');
      const socket = new WebSocket(wsUrl());
      socketRef.current = socket;

      socket.onopen = () => {
        if (!cancelled) setConnection('open');
      };

      socket.onmessage = (message) => {
        try {
          const event = JSON.parse(message.data) as EngineEvent;
          if (cancelled) return;
          setLatest(event);
          setEvents((prev) => {
            const next = [event, ...prev];
            return next.length > MAX_BUFFERED_EVENTS
              ? next.slice(0, MAX_BUFFERED_EVENTS)
              : next;
          });
        } catch {
          // Ignore malformed frames rather than crashing the UI.
        }
      };

      const scheduleReconnect = () => {
        if (cancelled) return;
        setConnection('closed');
        reconnectTimer = setTimeout(connect, RECONNECT_DELAY_MS);
      };

      socket.onclose = scheduleReconnect;
      socket.onerror = () => socket.close();
    };

    connect();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socketRef.current?.close();
    };
  }, []);

  return { connection, events, latest };
}
