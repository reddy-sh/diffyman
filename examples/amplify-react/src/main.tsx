import { DiffProvider } from "@diffyman/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { diffClient } from "./diffClient";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root element");

createRoot(root).render(
  <StrictMode>
    <DiffProvider client={diffClient}>
      <App />
    </DiffProvider>
  </StrictMode>,
);
