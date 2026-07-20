import { useState } from "react";

interface RiskDisclaimerProps {
  strategyName: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RiskDisclaimer({ strategyName, onConfirm, onCancel }: RiskDisclaimerProps): JSX.Element {
  const [acknowledged, setAcknowledged] = useState(false);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div className="w-full max-w-lg rounded-lg border border-red-800 bg-slate-900 p-6 shadow-xl">
        <h2 className="mb-3 text-lg font-semibold text-red-400">Activate "{strategyName}" — Risk Disclaimer</h2>
        <div className="mb-4 space-y-2 text-sm text-slate-300">
          <p>
            This build operates in <span className="font-semibold text-emerald-400">paper-trading mode only</span>: all trades are
            simulated against a virtual ledger using real market prices. No real funds, wallet keys, or on-chain transactions are
            involved.
          </p>
          <p>
            Once active, this strategy will execute simulated trades autonomously based on its own logic, without further
            confirmation for each trade. Automated strategies (even simulated ones) can lose the entire virtual balance quickly in
            volatile conditions.
          </p>
          <p>
            When a future phase enables real trading, the same automated-execution risks would apply to real funds. Never fund a
            live trading wallet with more than you can afford to lose, and always test extensively in simulation first.
          </p>
          <p>You can stop all trading immediately at any time using the kill switch in Settings.</p>
        </div>
        <label className="mb-4 flex items-start gap-2 text-sm text-slate-200">
          <input type="checkbox" className="mt-1" checked={acknowledged} onChange={(e) => setAcknowledged(e.target.checked)} />
          I understand this strategy will trade autonomously and accept the risks described above.
        </label>
        <div className="flex justify-end gap-3">
          <button onClick={onCancel} className="rounded px-4 py-2 text-sm text-slate-300 hover:bg-slate-800">
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={!acknowledged}
            className="rounded bg-red-700 px-4 py-2 text-sm font-medium text-white enabled:hover:bg-red-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Activate Strategy
          </button>
        </div>
      </div>
    </div>
  );
}
