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

/// Formats a token price with enough decimals to stay meaningful for
/// sub-cent tokens (e.g. BONK at ~$0.000003) -- a fixed toFixed(4) would
/// show "$0.0000" for those. Prices at $1+ keep the familiar 4-decimal
/// display; below $1, the decimal count grows with how small the value
/// is, so roughly 4 significant figures are always visible.
export function formatPrice(value: number): string {
  if (!Number.isFinite(value) || value === 0) return '$0';
  const abs = Math.abs(value);
  if (abs >= 1) return `$${value.toFixed(4)}`;
  const decimals = Math.min(10, Math.max(4, 3 - Math.floor(Math.log10(abs))));
  return `$${value.toFixed(decimals)}`;
}

export function pnlTone(value: number): StatTileProps['tone'] {
  if (value > 0) return 'good';
  if (value < 0) return 'critical';
  return 'neutral';
}
