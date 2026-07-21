interface StatTileProps {
  label: string;
  value: string;
  tone?: 'neutral' | 'good' | 'critical';
  hint?: string;
}

const TONE_CLASS: Record<NonNullable<StatTileProps['tone']>, string> = {
  neutral: 'text-[var(--text-primary)]',
  good: 'text-[var(--status-good)]',
  critical: 'text-[var(--status-critical)]',
};

export function StatTile({ label, value, tone = 'neutral', hint }: StatTileProps) {
  return (
    <div className="rounded-lg border border-[var(--border)] bg-[var(--surface-1)] p-4">
      <div className="text-xs font-medium uppercase tracking-wide text-[var(--text-muted)]">
        {label}
      </div>
      <div className={`mt-1 text-2xl font-semibold ${TONE_CLASS[tone]}`}>{value}</div>
      {hint && <div className="mt-1 text-xs text-[var(--text-secondary)]">{hint}</div>}
    </div>
  );
}

export function formatUsd(value: number): string {
  return value.toLocaleString('en-US', {
    style: 'currency',
    currency: 'USD',
    maximumFractionDigits: 2,
  });
}

export function pnlTone(value: number): StatTileProps['tone'] {
  if (value > 0) return 'good';
  if (value < 0) return 'critical';
  return 'neutral';
}
