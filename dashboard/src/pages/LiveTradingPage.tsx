import { useState } from 'react';
import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { useLiveEvents } from '../api/useLiveEvents';
import { StatTile, formatUsd } from '../components/StatTile';
import type { LiveEvent } from '../api/types';

const CONFIRM_PHRASE = 'ENABLE LIVE TRADING';

function describeLiveEvent(event: LiveEvent): { text: string; tone: 'neutral' | 'good' | 'warning' | 'critical' } {
  switch (event.type) {
    case 'PriceUpdate':
      return { text: `${event.pair_label}: $${event.price.toFixed(4)}`, tone: 'neutral' };
    case 'SignalGenerated':
      return {
        text: `${event.strategy} signaled on ${event.pair_label} (${(event.confidence * 100).toFixed(0)}%)`,
        tone: 'neutral',
      };
    case 'WouldTrade':
      return {
        text: `Would ${event.is_buy ? 'buy' : 'sell'} ${formatUsd(event.size_usd)} of ${event.pair_label} (${event.strategy}) — live trading is off`,
        tone: 'warning',
      };
    case 'SignalSkipped':
      return { text: `${event.strategy} skipped on ${event.pair_label}: ${event.reason}`, tone: 'neutral' };
    case 'OrderFilled':
      return {
        text: `FILLED: ${event.strategy} — ${formatUsd(event.size_usd)} of ${event.pair_label} @ $${event.price.toFixed(4)} (${event.method})`,
        tone: 'good',
      };
    case 'OrderFailed':
      return { text: `FAILED: ${event.strategy} on ${event.pair_label} — ${event.reason}`, tone: 'critical' };
    case 'PositionClosed':
      return {
        text: `Closed ${event.pair_label}: ${formatUsd(event.realized_pnl_usd)} realized (${event.reason})`,
        tone: event.realized_pnl_usd >= 0 ? 'good' : 'critical',
      };
    case 'LiveTradingEnabled':
      return { text: 'Live trading ENABLED', tone: 'warning' };
    case 'LiveTradingDisabled':
      return { text: 'Live trading disabled', tone: 'neutral' };
    case 'MaxCapitalChanged':
      return { text: `Max capital changed to ${formatUsd(event.max_capital_usd)}`, tone: 'neutral' };
    case 'TickCompleted':
      return { text: `Tick complete — ${event.signal_count} signal(s)`, tone: 'neutral' };
  }
}

const TONE_DOT: Record<string, string> = {
  neutral: 'bg-[var(--text-muted)]',
  good: 'bg-[var(--status-good)]',
  warning: 'bg-[var(--status-warning)]',
  critical: 'bg-[var(--status-critical)]',
};

export function LiveTradingPage() {
  const { data: status, error, loading } = usePolling(api.liveStatus, 5000);
  const { events } = useLiveEvents();
  const [maxCapitalInput, setMaxCapitalInput] = useState('');
  const [confirmText, setConfirmText] = useState('');
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const handleSetMaxCapital = async () => {
    const value = Number(maxCapitalInput);
    if (!Number.isFinite(value) || value < 0) {
      setActionError('Enter a valid non-negative number.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetMaxCapital(value);
      setMaxCapitalInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleEnable = async () => {
    if (confirmText !== CONFIRM_PHRASE) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.liveEnable();
      setConfirmText('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDisable = async () => {
    setBusy(true);
    setActionError(null);
    try {
      await api.liveDisable();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <h1 className="text-lg font-semibold">Live Trading</h1>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--status-warning)]/40 bg-[var(--status-warning)]/15 px-2.5 py-1 text-xs font-semibold uppercase tracking-wide text-[var(--status-warning)]">
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--status-warning)]" aria-hidden />
          Live — real funds
        </span>
      </div>

      {error && <p className="text-sm text-[var(--status-critical)]">{error}</p>}
      {!loading && !status && !error && (
        <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-6 text-sm text-[var(--text-secondary)]">
          No live trading engine configured. Set <code>WALLET_KEYPAIR_PATH</code> and restart{' '}
          <code>solstice-api</code>.
        </div>
      )}

      {status && (
        <>
          <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
            <StatTile
              label="Trading status"
              value={status.enabled ? 'LIVE' : 'Disabled'}
              tone={status.enabled ? 'critical' : 'good'}
              hint={status.enabled ? 'Real trades will execute' : 'Signals are simulated only'}
            />
            <StatTile label="Max capital" value={formatUsd(status.max_capital_usd)} />
            <StatTile label="Deployed" value={formatUsd(status.capital_deployed_usd)} />
            <StatTile label="Available" value={formatUsd(status.capital_available_usd)} />
          </div>

          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {/* Kill switch */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Kill switch
              </div>
              {status.enabled ? (
                <button
                  type="button"
                  onClick={handleDisable}
                  disabled={busy}
                  className="w-full rounded-md bg-[var(--status-critical)] px-4 py-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
                >
                  Disable live trading now
                </button>
              ) : (
                <div className="flex flex-col gap-2">
                  <label className="text-xs text-[var(--text-muted)]">
                    Type <code>{CONFIRM_PHRASE}</code> to arm live trading:
                  </label>
                  <input
                    type="text"
                    value={confirmText}
                    onChange={(e) => setConfirmText(e.target.value)}
                    placeholder={CONFIRM_PHRASE}
                    className="rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                  />
                  <button
                    type="button"
                    onClick={handleEnable}
                    disabled={busy || confirmText !== CONFIRM_PHRASE}
                    className="w-full rounded-md bg-[var(--status-warning)] px-4 py-2 text-sm font-semibold text-black hover:opacity-90 disabled:opacity-40"
                  >
                    Enable live trading
                  </button>
                </div>
              )}
            </div>

            {/* Capital cap */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Max capital ({formatUsd(status.max_capital_usd)} currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  step="1"
                  value={maxCapitalInput}
                  onChange={(e) => setMaxCapitalInput(e.target.value)}
                  placeholder="e.g. 50"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetMaxCapital}
                  disabled={busy || maxCapitalInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Hard ceiling on total capital this engine will ever deploy — independent of
                the wallet's actual balance.
              </p>
            </div>
          </div>

          {actionError && <p className="text-sm text-[var(--status-critical)]">{actionError}</p>}

          {status.positions.length > 0 && (
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)]">
              <div className="border-b border-[var(--gridline)] px-4 py-3 text-sm font-medium text-[var(--text-secondary)]">
                Open positions
              </div>
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase text-[var(--text-muted)]">
                    <th className="px-4 py-2">Pair</th>
                    <th className="px-4 py-2">Entry</th>
                    <th className="px-4 py-2">Current</th>
                    <th className="px-4 py-2">Allocated</th>
                    <th className="px-4 py-2">Unrealized P&amp;L</th>
                  </tr>
                </thead>
                <tbody>
                  {status.positions.map((p) => (
                    <tr key={p.pair_label} className="border-t border-[var(--gridline)]">
                      <td className="px-4 py-2">{p.pair_label}</td>
                      <td className="px-4 py-2">${p.entry_price.toFixed(4)}</td>
                      <td className="px-4 py-2">${p.current_price.toFixed(4)}</td>
                      <td className="px-4 py-2">{formatUsd(p.allocated_usd)}</td>
                      <td
                        className={
                          p.unrealized_pnl_usd >= 0
                            ? 'px-4 py-2 text-[var(--status-good)]'
                            : 'px-4 py-2 text-[var(--status-critical)]'
                        }
                      >
                        {formatUsd(p.unrealized_pnl_usd)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}

      <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)]">
        <div className="border-b border-[var(--gridline)] px-4 py-3 text-sm font-medium text-[var(--text-secondary)]">
          Live activity
        </div>
        <ul className="max-h-96 divide-y divide-[var(--gridline)] overflow-y-auto">
          {events.length === 0 && (
            <li className="px-4 py-6 text-center text-sm text-[var(--text-muted)]">
              Waiting for the first event…
            </li>
          )}
          {events.map((event, index) => {
            const { text, tone } = describeLiveEvent(event);
            return (
              // eslint-disable-next-line react/no-array-index-key
              <li key={index} className="flex items-start gap-3 px-4 py-2.5 text-sm">
                <span className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${TONE_DOT[tone]}`} />
                <span className="text-[var(--text-primary)]">{text}</span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
