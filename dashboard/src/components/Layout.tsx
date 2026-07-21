import { NavLink, Outlet } from 'react-router-dom';
import type { ConnectionState } from '../api/useEngineEvents';
import type { WalletResponse } from '../api/types';

const SIMULATION_NAV_ITEMS = [
  { to: '/', label: 'Overview' },
  { to: '/positions', label: 'Positions' },
  { to: '/trades', label: 'Trades' },
  { to: '/performance', label: 'Performance' },
];

const LIVE_NAV_ITEMS = [
  { to: '/wallet', label: 'Wallet' },
  { to: '/live', label: 'Live Trading' },
];

function ConnectionBadge({ state }: { state: ConnectionState }) {
  const config = {
    open: { dot: 'bg-[var(--status-good)]', label: 'Live feed' },
    connecting: { dot: 'bg-[var(--status-warning)]', label: 'Connecting' },
    closed: { dot: 'bg-[var(--status-critical)]', label: 'Disconnected' },
  }[state];

  return (
    <span className="inline-flex items-center gap-2 text-sm text-[var(--text-secondary)]">
      <span className={`h-2 w-2 rounded-full ${config.dot}`} aria-hidden />
      {config.label}
    </span>
  );
}

/** Always-visible mode indicators: the paper engine is always simulated;
 * the wallet badge reflects whether this server can even see a real
 * wallet (it never states — nor could it, since the API is read-only —
 * whether that wallet has been used to send anything). */
function ModeBadges({ wallet, walletLoading }: { wallet: WalletResponse | null; walletLoading: boolean }) {
  return (
    <div className="flex items-center gap-2">
      <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--series-1)]/30 bg-[var(--series-1)]/10 px-2.5 py-1 text-xs font-semibold uppercase tracking-wide text-[var(--series-1)]">
        <span className="h-1.5 w-1.5 rounded-full bg-[var(--series-1)]" aria-hidden />
        Paper — simulated funds
      </span>
      {!walletLoading && (
        <span
          className={
            wallet
              ? 'inline-flex items-center gap-1.5 rounded-full border border-[var(--status-warning)]/40 bg-[var(--status-warning)]/15 px-2.5 py-1 text-xs font-semibold uppercase tracking-wide text-[var(--status-warning)]'
              : 'inline-flex items-center gap-1.5 rounded-full border border-[var(--border)] px-2.5 py-1 text-xs font-medium text-[var(--text-muted)]'
          }
        >
          <span
            className={`h-1.5 w-1.5 rounded-full ${wallet ? 'bg-[var(--status-warning)]' : 'bg-[var(--text-muted)]'}`}
            aria-hidden
          />
          {wallet ? 'Live wallet connected — real funds' : 'No live wallet configured'}
        </span>
      )}
    </div>
  );
}

function NavSection({
  title,
  items,
  accent,
}: {
  title: string;
  items: { to: string; label: string }[];
  accent: 'paper' | 'live';
}) {
  const activeClass =
    accent === 'live'
      ? 'bg-[var(--status-warning)]/15 text-[var(--status-warning)]'
      : 'bg-[var(--series-1)]/10 text-[var(--series-1)]';

  return (
    <div className="flex flex-col gap-1">
      <div className="px-3 text-[10px] font-semibold uppercase tracking-widest text-[var(--text-muted)]">
        {title}
      </div>
      {items.map((item) => (
        <NavLink
          key={item.to}
          to={item.to}
          end={item.to === '/'}
          className={({ isActive }) =>
            `rounded-md px-3 py-2 text-sm font-medium transition-colors ${
              isActive ? activeClass : 'text-[var(--text-secondary)] hover:bg-[var(--border)]'
            }`
          }
        >
          {item.label}
        </NavLink>
      ))}
    </div>
  );
}

export function Layout({
  connection,
  wallet,
  walletLoading,
}: {
  connection: ConnectionState;
  wallet: WalletResponse | null;
  walletLoading: boolean;
}) {
  return (
    <div className="flex min-h-svh">
      <aside className="flex w-56 shrink-0 flex-col gap-6 border-r border-[var(--border)] bg-[var(--surface-1)] p-4">
        <div className="px-2">
          <div className="text-lg font-semibold tracking-tight">Solstice</div>
          <div className="text-xs text-[var(--text-muted)]">Quant trading platform</div>
        </div>
        <NavSection title="Simulation (paper)" items={SIMULATION_NAV_ITEMS} accent="paper" />
        <NavSection title="Live" items={LIVE_NAV_ITEMS} accent="live" />
      </aside>

      <div className="flex flex-1 flex-col">
        <header className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--surface-1)] px-6 py-3">
          <ModeBadges wallet={wallet} walletLoading={walletLoading} />
          <ConnectionBadge state={connection} />
        </header>

        <main className="flex-1 overflow-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
