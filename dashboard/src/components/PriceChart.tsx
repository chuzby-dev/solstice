import { useMemo } from 'react';
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import type { EngineEvent } from '../api/types';
import { formatPrice } from './StatTile';

interface PricePoint {
  time: string;
  raydium: number | null;
  orca: number | null;
}

const MAX_POINTS = 120;

/** Builds a merged Raydium/Orca price series from the raw event buffer.
 * Events arrive newest-first; each new price update carries the other
 * DEX's most recent known price forward so both lines stay defined. */
function buildSeries(events: EngineEvent[], pairLabel: string): PricePoint[] {
  const relevant = events
    .filter(
      (event): event is Extract<EngineEvent, { type: 'PriceUpdate' }> =>
        event.type === 'PriceUpdate' && event.pair_label === pairLabel,
    )
    .slice()
    .reverse(); // oldest first for charting

  const points: PricePoint[] = [];
  let lastRaydium: number | null = null;
  let lastOrca: number | null = null;

  for (const event of relevant) {
    if (event.dex === 'Raydium') lastRaydium = event.price;
    if (event.dex === 'Orca') lastOrca = event.price;
    points.push({
      time: new Date(event.timestamp).toLocaleTimeString(),
      raydium: lastRaydium,
      orca: lastOrca,
    });
  }

  return points.slice(-MAX_POINTS);
}

export function PriceChart({ events, pairLabel }: { events: EngineEvent[]; pairLabel: string }) {
  const data = useMemo(() => buildSeries(events, pairLabel), [events, pairLabel]);

  if (data.length < 2) {
    return (
      <div className="flex h-64 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--surface-1)] text-sm text-[var(--text-muted)]">
        Waiting for enough live price updates to chart {pairLabel}…
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
      <div className="mb-2 text-sm font-medium text-[var(--text-secondary)]">
        {pairLabel} — Raydium vs Orca (live)
      </div>
      <ResponsiveContainer width="100%" height={260}>
        <LineChart data={data} margin={{ top: 4, right: 8, bottom: 0, left: 0 }}>
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
            width={70}
            tickFormatter={(value: number) => formatPrice(value)}
          />
          <Tooltip
            contentStyle={{
              background: 'var(--surface-1)',
              border: '1px solid var(--border)',
              borderRadius: 8,
              fontSize: 12,
            }}
            labelStyle={{ color: 'var(--text-secondary)' }}
            formatter={(value) => formatPrice(Number(value))}
          />
          <Legend wrapperStyle={{ fontSize: 12 }} />
          <Line
            type="monotone"
            dataKey="raydium"
            name="Raydium"
            stroke="var(--series-1)"
            strokeWidth={2}
            dot={false}
            connectNulls
          />
          <Line
            type="monotone"
            dataKey="orca"
            name="Orca"
            stroke="var(--series-6)"
            strokeWidth={2}
            dot={false}
            connectNulls
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}
