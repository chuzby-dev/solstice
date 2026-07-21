import type { PositionSnapshot } from '../api/types';
import { formatUsd, pnlTone } from './StatTile';

const TONE_TEXT: Record<string, string> = {
  good: 'text-[var(--status-good)]',
  critical: 'text-[var(--status-critical)]',
  neutral: 'text-[var(--text-primary)]',
};

export function PositionsTable({ positions }: { positions: PositionSnapshot[] }) {
  if (positions.length === 0) {
    return (
      <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-6 text-center text-sm text-[var(--text-muted)]">
        No open positions.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto rounded-lg border border-[var(--border)] bg-[var(--surface-1)]">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-[var(--gridline)] text-left text-xs uppercase tracking-wide text-[var(--text-muted)]">
            <th className="px-4 py-3 font-medium">Pair</th>
            <th className="px-4 py-3 font-medium">Quantity</th>
            <th className="px-4 py-3 font-medium">Entry price</th>
            <th className="px-4 py-3 font-medium">Current price</th>
            <th className="px-4 py-3 text-right font-medium">Unrealized P&amp;L</th>
          </tr>
        </thead>
        <tbody>
          {positions.map((position) => (
            <tr
              key={`${position.base_mint}-${position.quote_mint}`}
              className="border-b border-[var(--gridline)] last:border-0"
            >
              <td className="px-4 py-3 font-medium">{position.pair_label}</td>
              <td className="px-4 py-3">{position.quantity.toLocaleString()}</td>
              <td className="px-4 py-3">{formatUsd(position.entry_price)}</td>
              <td className="px-4 py-3">{formatUsd(position.current_price)}</td>
              <td
                className={`px-4 py-3 text-right font-medium ${TONE_TEXT[pnlTone(position.unrealized_pnl) ?? 'neutral']}`}
              >
                {formatUsd(position.unrealized_pnl)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
