import { useState } from 'react';
import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { useLiveEvents } from '../api/useLiveEvents';
import { StatTile, formatPrice, formatUsd } from '../components/StatTile';
import { ToggleSwitch } from '../components/ToggleSwitch';
import type { LiveEvent } from '../api/types';

function describeLiveEvent(event: LiveEvent): { text: string; tone: 'neutral' | 'good' | 'warning' | 'critical' } {
  switch (event.type) {
    case 'PriceUpdate':
      return { text: `${event.pair_label}: ${formatPrice(event.price)}`, tone: 'neutral' };
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
        text: `FILLED: ${event.strategy} — ${formatUsd(event.size_usd)} of ${event.pair_label} @ ${formatPrice(event.price)} (${event.method})`,
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
    case 'MinConfidenceChanged':
      return {
        text: `Minimum confidence to act changed to ${(event.min_confidence * 100).toFixed(0)}%`,
        tone: 'neutral',
      };
    case 'StrategiesEnabledChanged':
      return {
        text: `Strategy signals (SMA/SpreadArb) ${event.strategies_enabled ? 'enabled' : 'disabled'}`,
        tone: 'neutral',
      };
    case 'TakeProfitPercentChanged':
      return {
        text: `Take-profit target changed to ${(event.take_profit_percent * 100).toFixed(0)}%`,
        tone: 'neutral',
      };
    case 'CrossDexArbEnabledChanged':
      return {
        text: `Cross-DEX arbitrage ${event.cross_dex_arb_enabled ? 'ARMED' : 'disarmed'}`,
        tone: event.cross_dex_arb_enabled ? 'warning' : 'neutral',
      };
    case 'CrossDexMinSpreadChanged':
      return {
        text: `Cross-DEX min spread changed to ${(event.cross_dex_min_spread * 100).toFixed(2)}%`,
        tone: 'neutral',
      };
    case 'CrossDexMaxSlippageChanged':
      return {
        text: `Cross-DEX per-leg slippage tolerance changed to ${(event.cross_dex_max_slippage_bps / 100).toFixed(2)}%`,
        tone: 'neutral',
      };
    case 'CrossDexMinNetEdgeChanged':
      return {
        text: `Cross-DEX minimum net edge changed to ${(event.cross_dex_min_net_edge_bps / 100).toFixed(2)}%`,
        tone: 'neutral',
      };
    case 'CrossDexOpportunityDetected':
      return {
        text: `Spread on ${event.pair_label}: buy ${event.buy_dex} @ ${formatPrice(event.buy_price)}, sell ${event.sell_dex} @ ${formatPrice(event.sell_price)} (${event.spread_percent.toFixed(2)}%)`,
        tone: 'neutral',
      };
    case 'CrossDexArbFilled':
      return {
        text: `ARB FILLED: ${event.pair_label} — bought ${formatUsd(event.size_usd)} on ${event.buy_dex} @ ${formatPrice(event.buy_price)}, sold on ${event.sell_dex} @ ${formatPrice(event.sell_price)} (${formatUsd(event.realized_pnl_usd)} realized)`,
        tone: event.realized_pnl_usd >= 0 ? 'good' : 'critical',
      };
    case 'CrossDexArbFailed':
      return {
        text: `ARB FAILED (${event.leg} leg) on ${event.pair_label}: ${event.reason}`,
        tone: 'critical',
      };
    case 'UntrackedBalanceAdopted':
      return {
        text: `Adopted untracked ${event.pair_label} balance: ${event.quantity.toFixed(6)} (~${formatUsd(event.estimated_usd)}) — now tracked and eligible to cycle back to quote`,
        tone: 'warning',
      };
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
  const [minConfidenceInput, setMinConfidenceInput] = useState('');
  const [takeProfitInput, setTakeProfitInput] = useState('');
  const [crossDexSpreadInput, setCrossDexSpreadInput] = useState('');
  const [crossDexSlippageInput, setCrossDexSlippageInput] = useState('');
  const [crossDexMinEdgeInput, setCrossDexMinEdgeInput] = useState('');
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

  const handleSetMinConfidence = async () => {
    const percent = Number(minConfidenceInput);
    if (!Number.isFinite(percent) || percent < 0 || percent > 100) {
      setActionError('Enter a valid percentage between 0 and 100.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetMinConfidence(percent / 100);
      setMinConfidenceInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleSetTakeProfit = async () => {
    const percent = Number(takeProfitInput);
    if (!Number.isFinite(percent) || percent <= 0 || percent > 1000) {
      setActionError('Enter a valid positive percentage.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetTakeProfitPercent(percent / 100);
      setTakeProfitInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleSetCrossDexSpread = async () => {
    const percent = Number(crossDexSpreadInput);
    if (!Number.isFinite(percent) || percent <= 0 || percent > 1000) {
      setActionError('Enter a valid positive percentage.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetCrossDexMinSpread(percent / 100);
      setCrossDexSpreadInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleSetCrossDexSlippage = async () => {
    const percent = Number(crossDexSlippageInput);
    if (!Number.isFinite(percent) || percent <= 0 || percent > 100) {
      setActionError('Enter a valid positive percentage.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetCrossDexMaxSlippageBps(Math.round(percent * 100));
      setCrossDexSlippageInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleSetCrossDexMinEdge = async () => {
    const percent = Number(crossDexMinEdgeInput);
    if (!Number.isFinite(percent) || percent < 0 || percent > 100) {
      setActionError('Enter a valid non-negative percentage.');
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetCrossDexMinNetEdgeBps(Math.round(percent * 100));
      setCrossDexMinEdgeInput('');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleToggleStrategies = async (enabled: boolean) => {
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetStrategiesEnabled(enabled);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleToggleCrossDexArb = async (armed: boolean) => {
    setBusy(true);
    setActionError(null);
    try {
      await api.liveSetCrossDexArbEnabled(armed);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleToggleLiveTrading = async (enabled: boolean) => {
    setBusy(true);
    setActionError(null);
    try {
      if (enabled) {
        await api.liveEnable();
      } else {
        await api.liveDisable();
      }
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
            <StatTile
              label="Min confidence to act"
              value={`${(status.min_confidence * 100).toFixed(0)}%`}
            />
            <StatTile
              label="Strategy signals"
              value={status.strategies_enabled ? 'On' : 'Off'}
              tone={status.strategies_enabled ? 'neutral' : 'good'}
            />
            <StatTile
              label="Take-profit target"
              value={`${(status.take_profit_percent * 100).toFixed(0)}%`}
            />
            <StatTile
              label="Cross-DEX arbitrage"
              value={status.cross_dex_arb_enabled ? 'ARMED' : 'Disarmed'}
              tone={status.cross_dex_arb_enabled ? 'critical' : 'good'}
            />
            <StatTile
              label="Cross-DEX min spread"
              value={`${(status.cross_dex_min_spread * 100).toFixed(2)}%`}
            />
            <StatTile
              label="Cross-DEX per-leg slippage"
              value={`${(status.cross_dex_max_slippage_bps / 100).toFixed(2)}%`}
            />
            <StatTile
              label="Cross-DEX min net edge"
              value={`${(status.cross_dex_min_net_edge_bps / 100).toFixed(2)}%`}
              hint="Required after both legs' slippage"
            />
          </div>

          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {/* Kill switch */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <ToggleSwitch
                label={status.enabled ? 'Live trading is ON — real trades will execute' : 'Live trading is off'}
                checked={status.enabled}
                onChange={handleToggleLiveTrading}
                disabled={busy}
                activeColor="critical"
              />
            </div>

            {/* Strategy signals */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <ToggleSwitch
                label={
                  status.strategies_enabled
                    ? 'Strategy signals (SMA/SpreadArb) are ON'
                    : 'Strategy signals (SMA/SpreadArb) are off'
                }
                checked={status.strategies_enabled}
                onChange={handleToggleStrategies}
                disabled={busy}
                activeColor="warning"
              />
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Turn off to run <strong>only</strong> the cross-DEX arbitrage executor below,
                without SMA/SpreadArb's directional bets also trading.
              </p>
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

            {/* Minimum confidence to act */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Min confidence to act ({(status.min_confidence * 100).toFixed(0)}% currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  max="100"
                  step="1"
                  value={minConfidenceInput}
                  onChange={(e) => setMinConfidenceInput(e.target.value)}
                  placeholder="e.g. 65"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetMinConfidence}
                  disabled={busy || minConfidenceInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Signals below this confidence are skipped entirely, regardless of direction.
                Default 65%.
              </p>
            </div>

            {/* Take-profit target */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Take-profit target ({(status.take_profit_percent * 100).toFixed(0)}% currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  step="1"
                  value={takeProfitInput}
                  onChange={(e) => setTakeProfitInput(e.target.value)}
                  placeholder="e.g. 5"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetTakeProfit}
                  disabled={busy || takeProfitInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                An open position auto-closes once its gain crosses this threshold. Default 5%.
              </p>
            </div>

            {/* Cross-DEX arbitrage min spread */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Cross-DEX min spread ({(status.cross_dex_min_spread * 100).toFixed(2)}% currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  step="0.1"
                  value={crossDexSpreadInput}
                  onChange={(e) => setCrossDexSpreadInput(e.target.value)}
                  placeholder="e.g. 1.5"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetCrossDexSpread}
                  disabled={busy || crossDexSpreadInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Minimum price gap between the cheapest and priciest DEX quote to attempt an
                arbitrage trade. Default 1.5% -- set well above the cost of two separate swaps.
              </p>
            </div>

            {/* Cross-DEX arbitrage per-leg slippage */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Cross-DEX per-leg slippage (
                {(status.cross_dex_max_slippage_bps / 100).toFixed(2)}% currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  step="0.05"
                  value={crossDexSlippageInput}
                  onChange={(e) => setCrossDexSlippageInput(e.target.value)}
                  placeholder="e.g. 0.3"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetCrossDexSlippage}
                  disabled={busy || crossDexSlippageInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Max slippage tolerated on <em>each</em> leg -- separate from, and much tighter
                than, the general trading slippage. Default 0.3%. Keep this well below half the
                min spread above, or a "profitable" arb can slip into a real loss.
              </p>
            </div>

            {/* Cross-DEX arbitrage minimum net edge */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <div className="mb-3 text-sm font-medium text-[var(--text-secondary)]">
                Cross-DEX min net edge (
                {(status.cross_dex_min_net_edge_bps / 100).toFixed(2)}% currently)
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min="0"
                  step="0.05"
                  value={crossDexMinEdgeInput}
                  onChange={(e) => setCrossDexMinEdgeInput(e.target.value)}
                  placeholder="e.g. 0.1"
                  className="flex-1 rounded-md border border-[var(--border)] bg-[var(--page)] px-3 py-2 text-sm"
                />
                <button
                  type="button"
                  onClick={handleSetCrossDexMinEdge}
                  disabled={busy || crossDexMinEdgeInput === ''}
                  className="shrink-0 rounded-md border border-[var(--border)] px-3 py-2 text-sm font-medium hover:bg-[var(--border)] disabled:opacity-40"
                >
                  Save
                </button>
              </div>
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Required profit margin left over after assuming both legs slip by the full
                per-leg tolerance above. The trade gate is actually{' '}
                <code>max(min spread, 2x per-leg slippage + this)</code> -- a raw quoted spread
                clearing "min spread" alone doesn't mean it survives real execution costs.
              </p>
            </div>

            {/* Cross-DEX arbitrage arm switch */}
            <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
              <ToggleSwitch
                label={status.cross_dex_arb_enabled ? 'Cross-DEX arbitrage is ARMED' : 'Cross-DEX arbitrage is disarmed'}
                checked={status.cross_dex_arb_enabled}
                onChange={handleToggleCrossDexArb}
                disabled={busy}
                activeColor="warning"
              />
              <p className="mt-2 text-xs text-[var(--text-muted)]">
                Buys on whichever registered DEX quotes cheapest and immediately sells on
                whichever quotes priciest. <strong>Not atomic</strong> -- two separate
                transactions, so real execution-price risk exists between them. If the second
                leg fails, the bought inventory is tracked as a normal open position, protected
                by stop-loss/take-profit above.
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
                      <td className="px-4 py-2">{formatPrice(p.entry_price)}</td>
                      <td className="px-4 py-2">{formatPrice(p.current_price)}</td>
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
