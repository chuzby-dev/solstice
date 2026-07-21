import { Route, HashRouter, Routes } from 'react-router-dom';
import { api } from './api/client';
import { useEngineEvents } from './api/useEngineEvents';
import { usePolling } from './api/usePolling';
import { Layout } from './components/Layout';
import { LiveTradingPage } from './pages/LiveTradingPage';
import { OverviewPage } from './pages/OverviewPage';
import { PerformancePage } from './pages/PerformancePage';
import { PositionsPage } from './pages/PositionsPage';
import { TradesPage } from './pages/TradesPage';
import { WalletPage } from './pages/WalletPage';

function App() {
  const { connection, events } = useEngineEvents();
  // Polled once here so the mode badge in the header is accurate on every
  // page, not just the Wallet page.
  const { data: wallet, loading: walletLoading } = usePolling(api.wallet, 15000);

  return (
    <HashRouter>
      <Routes>
        <Route element={<Layout connection={connection} wallet={wallet ?? null} walletLoading={walletLoading} />}>
          <Route index element={<OverviewPage events={events} />} />
          <Route path="positions" element={<PositionsPage />} />
          <Route path="trades" element={<TradesPage />} />
          <Route path="performance" element={<PerformancePage />} />
          <Route path="wallet" element={<WalletPage />} />
          <Route path="live" element={<LiveTradingPage />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}

export default App;
