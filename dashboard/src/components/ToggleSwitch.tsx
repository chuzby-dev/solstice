interface ToggleSwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
  activeColor?: 'critical' | 'warning';
}

const ACTIVE_TRACK_CLASS: Record<NonNullable<ToggleSwitchProps['activeColor']>, string> = {
  critical: 'bg-[var(--status-critical)]',
  warning: 'bg-[var(--status-warning)]',
};

export function ToggleSwitch({
  checked,
  onChange,
  disabled,
  label,
  activeColor = 'critical',
}: ToggleSwitchProps) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-3">
      <span className="text-sm font-medium text-[var(--text-secondary)]">{label}</span>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:opacity-40 ${
          checked ? ACTIVE_TRACK_CLASS[activeColor] : 'bg-[var(--border)]'
        }`}
      >
        <span
          className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${
            checked ? 'translate-x-[22px]' : 'translate-x-0.5'
          }`}
        />
      </button>
    </label>
  );
}
