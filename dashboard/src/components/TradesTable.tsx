import type { OrderStatus, TradeResponse } from '../api/types';
import { formatUsd } from './StatTile';

const STATUS_STYLE: Record<OrderStatus, string> = {
  filled: 'bg-[var(--status-good)]/15 text-[var(--status-good)]',
  partially_filled: 'bg-[var(--status-warning)]/20 text-[var(--status-warning)]',
  submitted: 'bg-[var(--series-1)]/15 text-[var(--series-1)]',
  failed: 'bg-[var(--status-critical)]/15 text-[var(--status-critical)]',
  cancelled: 'bg-[var(--text-muted)]/15 text-[var(--text-muted)]',
};

const STATUS_LABEL: Record<OrderStatus, string> = {
  filled: 'Filled',
  partially_filled: 'Partial',
  submitted: 'Submitted',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

function StatusBadge({ status }: { status: OrderStatus }) {
  return (
    <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${STATUS_STYLE[status]}`}>
      {STATUS_LABEL[status]}
    </span>
  );
}

export function TradesTable({ trades }: { trades: TradeResponse[] }) {
  if (trades.length === 0) {
    return (
      <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-6 text-center text-sm text-[var(--text-muted)]">
        No trades yet.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto rounded-lg border border-[var(--border)] bg-[var(--surface-1)]">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-[var(--gridline)] text-left text-xs uppercase tracking-wide text-[var(--text-muted)]">
            <th className="px-4 py-3 font-medium">Time</th>
            <th className="px-4 py-3 font-medium">Strategy</th>
            <th className="px-4 py-3 font-medium">Status</th>
            <th className="px-4 py-3 text-right font-medium">Size</th>
            <th className="px-4 py-3 text-right font-medium">Filled</th>
            <th className="px-4 py-3 font-medium">Order ID</th>
          </tr>
        </thead>
        <tbody>
          {trades.map((trade) => (
            <tr key={trade.order_id} className="border-b border-[var(--gridline)] last:border-0">
              <td className="px-4 py-3 text-[var(--text-secondary)]">
                {new Date(trade.created_at).toLocaleTimeString()}
              </td>
              <td className="px-4 py-3 font-medium">{trade.strategy}</td>
              <td className="px-4 py-3">
                <StatusBadge status={trade.status} />
              </td>
              <td className="px-4 py-3 text-right">{formatUsd(trade.size_usd)}</td>
              <td className="px-4 py-3 text-right">{formatUsd(trade.filled_amount)}</td>
              <td
                className="px-4 py-3 font-mono text-xs text-[var(--text-muted)]"
                title={trade.order_id}
              >
                {trade.order_id.slice(0, 8)}…
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
