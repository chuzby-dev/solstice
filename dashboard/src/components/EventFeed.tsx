import type { EngineEvent } from '../api/types';
import { formatUsd } from './StatTile';

function describe(event: EngineEvent): { text: string; tone: 'neutral' | 'good' | 'accent' } {
  switch (event.type) {
    case 'PriceUpdate':
      return {
        text: `${event.pair_label} on ${event.dex}: $${event.price.toFixed(4)}`,
        tone: 'neutral',
      };
    case 'SignalGenerated':
      return {
        text: `${event.strategy} signaled on ${event.pair_label} (confidence ${(event.confidence * 100).toFixed(0)}%)`,
        tone: 'accent',
      };
    case 'OrderFilled':
      return {
        text: `${event.strategy} filled ${formatUsd(event.size_usd)} of ${event.pair_label} @ $${event.price.toFixed(4)}`,
        tone: 'good',
      };
    case 'TickCompleted':
      return {
        text: `Tick complete — ${event.signal_count} signal(s)`,
        tone: 'neutral',
      };
  }
}

const TONE_DOT: Record<string, string> = {
  neutral: 'bg-[var(--text-muted)]',
  good: 'bg-[var(--status-good)]',
  accent: 'bg-[var(--series-1)]',
};

export function EventFeed({ events }: { events: EngineEvent[] }) {
  return (
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
          const { text, tone } = describe(event);
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
  );
}
