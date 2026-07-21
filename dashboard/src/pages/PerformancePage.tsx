import { useEffect, useState } from 'react';
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { StatTile, formatUsd, pnlTone } from '../components/StatTile';

interface ValuePoint {
  time: string;
  total_value_usd: number;
}

const MAX_HISTORY_POINTS = 120;

export function PerformancePage() {
  const { data } = usePolling(api.performance, 5000);
  const [history, setHistory] = useState<ValuePoint[]>([]);

  useEffect(() => {
    if (!data) return;
    setHistory((prev) => {
      const next = [
        ...prev,
        { time: new Date().toLocaleTimeString(), total_value_usd: data.total_value_usd },
      ];
      return next.length > MAX_HISTORY_POINTS ? next.slice(-MAX_HISTORY_POINTS) : next;
    });
  }, [data]);

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-lg font-semibold">Performance</h1>

      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <StatTile label="Cash" value={data ? formatUsd(data.cash_usd) : '—'} />
        <StatTile
          label="Realized P&amp;L"
          value={data ? formatUsd(data.realized_pnl_usd) : '—'}
          tone={data ? pnlTone(data.realized_pnl_usd) : 'neutral'}
        />
        <StatTile
          label="Unrealized P&amp;L"
          value={data ? formatUsd(data.unrealized_pnl_usd) : '—'}
          tone={data ? pnlTone(data.unrealized_pnl_usd) : 'neutral'}
        />
        <StatTile
          label="Total value"
          value={data ? formatUsd(data.total_value_usd) : '—'}
        />
      </div>

      <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
        <div className="mb-2 text-sm font-medium text-[var(--text-secondary)]">
          Portfolio value (this session)
        </div>
        {history.length < 2 ? (
          <div className="flex h-64 items-center justify-center text-sm text-[var(--text-muted)]">
            Collecting session history…
          </div>
        ) : (
          <ResponsiveContainer width="100%" height={260}>
            <LineChart data={history} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
              <CartesianGrid stroke="var(--gridline)" vertical={false} />
              <XAxis
                dataKey="time"
                stroke="var(--text-muted)"
                tick={{ fill: 'var(--text-muted)', fontSize: 11 }}
                minTickGap={40}
              />
              <YAxis
                stroke="var(--text-muted)"
                tick={{ fill: 'var(--text-muted)', fontSize: 11 }}
                domain={['auto', 'auto']}
                width={80}
                tickFormatter={(value: number) => `$${value.toFixed(0)}`}
              />
              <Tooltip
                contentStyle={{
                  background: 'var(--surface-1)',
                  border: '1px solid var(--border)',
                  borderRadius: 8,
                  fontSize: 12,
                }}
                labelStyle={{ color: 'var(--text-secondary)' }}
                formatter={(value) => formatUsd(Number(value))}
              />
              <Line
                type="monotone"
                dataKey="total_value_usd"
                name="Portfolio value"
                stroke="var(--series-1)"
                strokeWidth={2}
                dot={false}
              />
            </LineChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}
