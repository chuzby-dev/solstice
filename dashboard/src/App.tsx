import { Route, HashRouter, Routes } from 'react-router-dom';
import { useEngineEvents } from './api/useEngineEvents';
import { Layout } from './components/Layout';
import { OverviewPage } from './pages/OverviewPage';
import { PerformancePage } from './pages/PerformancePage';
import { PositionsPage } from './pages/PositionsPage';
import { TradesPage } from './pages/TradesPage';

function App() {
  const { connection, events } = useEngineEvents();

  return (
    <HashRouter>
      <Routes>
        <Route element={<Layout connection={connection} />}>
          <Route index element={<OverviewPage events={events} />} />
          <Route path="positions" element={<PositionsPage />} />
          <Route path="trades" element={<TradesPage />} />
          <Route path="performance" element={<PerformancePage />} />
        </Route>
      </Routes>
    </HashRouter>
  );
}

export default App;
