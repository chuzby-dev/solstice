import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { TradesTable } from '../components/TradesTable';

export function TradesPage() {
  const { data, error } = usePolling(api.trades, 5000);

  return (
    <div className="flex flex-col gap-4">
      <h1 className="text-lg font-semibold">Trades</h1>
      {error && <p className="text-sm text-[var(--status-critical)]">{error}</p>}
      <TradesTable trades={data?.trades ?? []} />
    </div>
  );
}
