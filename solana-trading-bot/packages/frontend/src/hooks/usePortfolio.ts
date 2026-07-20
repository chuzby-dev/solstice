import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api.js";

/** One-shot / manually-refetchable portfolio fetch, used to paint the initial state
 * before the WebSocket connects and for a manual "refresh" affordance. Live updates
 * after that come from useLiveFeed(). */
export function usePortfolioQuery() {
  return useQuery({
    queryKey: ["portfolio"],
    queryFn: api.getPortfolio,
    refetchInterval: 30_000,
  });
}
