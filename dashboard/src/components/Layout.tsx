import { NavLink, Outlet } from 'react-router-dom';
import type { ConnectionState } from '../api/useEngineEvents';

const NAV_ITEMS = [
  { to: '/', label: 'Overview' },
  { to: '/positions', label: 'Positions' },
  { to: '/trades', label: 'Trades' },
  { to: '/performance', label: 'Performance' },
];

function ConnectionBadge({ state }: { state: ConnectionState }) {
  const config = {
    open: { dot: 'bg-[var(--status-good)]', label: 'Live' },
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

export function Layout({ connection }: { connection: ConnectionState }) {
  return (
    <div className="flex min-h-svh">
      <aside className="w-56 shrink-0 border-r border-[var(--border)] bg-[var(--surface-1)] p-4">
        <div className="mb-8 px-2">
          <div className="text-lg font-semibold tracking-tight">Solstice</div>
          <div className="text-xs text-[var(--text-muted)]">Paper trading</div>
        </div>
        <nav className="flex flex-col gap-1">
          {NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === '/'}
              className={({ isActive }) =>
                `rounded-md px-3 py-2 text-sm font-medium transition-colors ${
                  isActive
                    ? 'bg-[var(--series-1)]/10 text-[var(--series-1)]'
                    : 'text-[var(--text-secondary)] hover:bg-[var(--border)]'
                }`
              }
            >
              {item.label}
            </NavLink>
          ))}
        </nav>
      </aside>

      <div className="flex flex-1 flex-col">
        <header className="flex items-center justify-between border-b border-[var(--border)] bg-[var(--surface-1)] px-6 py-3">
          <div className="text-sm text-[var(--text-muted)]">
            Live paper trading — no real transactions
          </div>
          <ConnectionBadge state={connection} />
        </header>

        <main className="flex-1 overflow-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
