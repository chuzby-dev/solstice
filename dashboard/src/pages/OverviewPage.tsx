import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import type { EngineEvent } from '../api/types';
import { EventFeed } from '../components/EventFeed';
import { PriceChart } from '../components/PriceChart';
import { StatTile, formatUsd } from '../components/StatTile';

export function OverviewPage({ events }: { events: EngineEvent[] }) {
  const { data: status } = usePolling(api.status, 5000);

  const monitoredPair = status?.monitored_pairs[0] ?? null;

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-2 gap-4 md:grid-cols-4">
        <StatTile
          label="Status"
          value={status?.status ?? '—'}
          tone={status?.circuit_breaker_tripped ? 'critical' : 'good'}
          hint={status?.circuit_breaker_tripped ? 'Circuit breaker tripped' : 'Trading normally'}
        />
        <StatTile
          label="Monitored pairs"
          value={status?.monitored_pairs.join(', ') ?? '—'}
        />
        <StatTile label="Open positions" value={String(status?.open_positions ?? '—')} />
        <StatTile
          label="Portfolio value"
          value={status ? formatUsd(status.total_value_usd) : '—'}
        />
      </div>

      {monitoredPair && <PriceChart events={events} pairLabel={monitoredPair} />}

      <EventFeed events={events} />
    </div>
  );
}
