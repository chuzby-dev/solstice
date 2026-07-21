import { useEffect, useRef, useState } from 'react';
import { liveWsUrl } from './client';
import type { LiveEvent } from './types';

export type LiveConnectionState = 'connecting' | 'open' | 'closed';

interface LiveEventsState {
  connection: LiveConnectionState;
  events: LiveEvent[];
}

const MAX_BUFFERED_EVENTS = 200;
const RECONNECT_DELAY_MS = 2000;

/**
 * Subscribes to the live-trading-engine WebSocket stream. Mirrors
 * `useEngineEvents` (the paper-engine equivalent) but against
 * `/api/v1/live/ws`, which 404s if no live engine is configured -- in
 * that case this just sits in `closed` and keeps retrying harmlessly.
 */
export function useLiveEvents(): LiveEventsState {
  const [connection, setConnection] = useState<LiveConnectionState>('connecting');
  const [events, setEvents] = useState<LiveEvent[]>([]);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      if (cancelled) return;
      setConnection('connecting');
      const socket = new WebSocket(liveWsUrl());
      socketRef.current = socket;

      socket.onopen = () => {
        if (!cancelled) setConnection('open');
      };

      socket.onmessage = (message) => {
        try {
          const event = JSON.parse(message.data) as LiveEvent;
          if (cancelled) return;
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

  return { connection, events };
}
