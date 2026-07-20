import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { App } from "./App.js";
import { AppWalletProvider } from "./wallet/WalletProvider.js";
import { LiveFeedProvider } from "./hooks/useWebSocket.js";
import "./index.css";

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <AppWalletProvider>
        <LiveFeedProvider>
          <App />
        </LiveFeedProvider>
      </AppWalletProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
