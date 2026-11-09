import { createContext, useContext, type ReactNode } from "react";
import type { DiffClient } from "./client.js";

const DiffClientContext = createContext<DiffClient | null>(null);

export interface DiffProviderProps {
  client: DiffClient;
  children?: ReactNode;
}

/** Makes one `DiffClient` (one worker, one WASM module) available to `useDiff`. */
export function DiffProvider({ client, children }: DiffProviderProps) {
  return <DiffClientContext.Provider value={client}>{children}</DiffClientContext.Provider>;
}

/** The client of the nearest `DiffProvider`, or `null`. */
export function useDiffClient(): DiffClient | null {
  return useContext(DiffClientContext);
}
