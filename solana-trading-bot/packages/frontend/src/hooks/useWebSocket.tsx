import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { PortfolioSnapshot, PriceTick, Trade, WsMessage } from "@trading-bot/shared";

interface LiveFeedState {
  connected: boolean;
  portfolio: PortfolioSnapshot | null;
  recentTrades: Trade[];
  prices: Record<string, PriceTick>;
  paused: boolean;
  /** True once the user has both granted the browser's Notification permission AND
   * opted in via enableNotifications() — see NotificationToggle in Wallet.tsx. Requesting
   * permission requires a user gesture in most browsers, so this can't just happen on
   * page load. */
  notificationsEnabled: boolean;
  enableNotifications: () => void;
}

// Persisted so a page reload doesn't silently go back to "off" after the user already
// granted permission once — Notification.permission itself only ever reports whether the
// BROWSER allows it, not whether this app should actually fire them.
const NOTIF_STORAGE_KEY = "notifications-enabled";

const LiveFeedContext = createContext<LiveFeedState | null>(null);

const MAX_RECENT_TRADES = 50;

function canNotify(): boolean {
  return typeof Notification !== "undefined" && Notification.permission === "granted" && localStorage.getItem(NOTIF_STORAGE_KEY) === "1";
}

/** Fires a real OS-level browser notification for REAL money events only (live trades,
 * wallet sends/sweeps) — paper trades happen far too often to notify on and aren't real
 * funds moving, so they're deliberately excluded (see the two call sites below). Never
 * throws: a failed notification (e.g. tab not in a notification-capable context) must
 * never break the app around it. */
function notify(title: string, body: string): void {
  if (!canNotify()) return;
  try {
    new Notification(title, { body });
  } catch {
    // ignore — see doc comment above
  }
}

export function LiveFeedProvider({ children }: { children: ReactNode }): JSX.Element {
  const [state, setState] = useState<Omit<LiveFeedState, "enableNotifications">>({
    connected: false,
    portfolio: null,
    recentTrades: [],
    prices: {},
    paused: false,
    notificationsEnabled: canNotify(),
  });
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout>;

    function connect(): void {
      if (cancelled) return;
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const socket = new WebSocket(`${protocol}//${window.location.host}/ws`);
      socketRef.current = socket;

      socket.onopen = () => setState((s) => ({ ...s, connected: true }));
      socket.onclose = () => {
        setState((s) => ({ ...s, connected: false }));
        if (!cancelled) retryTimer = setTimeout(connect, 2000);
      };
      socket.onerror = () => socket.close();

      socket.onmessage = (event) => {
        const message = JSON.parse(event.data as string) as WsMessage;

        // Real-money-only notifications — see notify()'s doc comment.
        if (message.type === "trade" && !message.data.simulated) {
          notify(
            `Live ${message.data.action.toUpperCase()}: ${message.data.sizeToken.toFixed(4)} ${message.data.tokenSymbol}`,
            `${message.data.strategyId} @ $${message.data.priceUsd.toFixed(2)} — real funds on ${message.data.network ?? "mainnet"}`,
          );
        }
        if (message.type === "wallet_send") {
          const dest = message.data.destination;
          notify(`Sent ${message.data.amount.toFixed(4)} ${message.data.tokenSymbol}`, dest ? `To ${dest.slice(0, 4)}…${dest.slice(-4)} on ${message.data.network}` : message.data.network);
        }

        setState((s) => {
          switch (message.type) {
            case "portfolio":
              return { ...s, portfolio: message.data };
            case "price_tick":
              return { ...s, prices: { ...s.prices, [message.data.tokenMint]: message.data } };
            case "trade":
              return { ...s, recentTrades: [message.data, ...s.recentTrades].slice(0, MAX_RECENT_TRADES) };
            case "engine_status":
              return { ...s, paused: message.data.paused };
            default:
              return s;
          }
        });
      };
    }

    connect();
    return () => {
      cancelled = true;
      clearTimeout(retryTimer);
      socketRef.current?.close();
    };
  }, []);

  function enableNotifications(): void {
    if (typeof Notification === "undefined") return;
    Notification.requestPermission()
      .then((permission) => {
        if (permission === "granted") {
          localStorage.setItem(NOTIF_STORAGE_KEY, "1");
          setState((s) => ({ ...s, notificationsEnabled: true }));
        }
      })
      .catch(() => {
        // ignore — user simply doesn't get notifications
      });
  }

  return <LiveFeedContext.Provider value={{ ...state, enableNotifications }}>{children}</LiveFeedContext.Provider>;
}

export function useLiveFeed(): LiveFeedState {
  const ctx = useContext(LiveFeedContext);
  if (!ctx) throw new Error("useLiveFeed must be used within a LiveFeedProvider");
  return ctx;
}
