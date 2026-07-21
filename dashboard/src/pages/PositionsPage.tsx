import { api } from '../api/client';
import { usePolling } from '../api/usePolling';
import { PositionsTable } from '../components/PositionsTable';

export function PositionsPage() {
  const { data, error } = usePolling(api.positions, 5000);

  return (
    <div className="flex flex-col gap-4">
      <h1 className="text-lg font-semibold">Positions</h1>
      {error && <p className="text-sm text-[var(--status-critical)]">{error}</p>}
      <PositionsTable positions={data?.positions ?? []} />
    </div>
  );
}
