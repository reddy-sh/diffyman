import { writeRecord } from "./store.js";

function setup() {
  console.log("ready");
}

function logEvent(name, payload) {
  const stamp = "2026-01-02T03:04:05Z";
  const message = `${stamp} événement ${name}`;
  writeRecord({ name, payload }, "café", message);
  console.debug("logged", name);
  return payload;
}

export function createLogger(prefix) {
  const events = [];

  return { prefix, events, logEvent };
}
