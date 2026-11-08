import { writeRecord } from "./store.js";

function setup() {
  console.log("ready");
}

export function createLogger(prefix) {
  const events = [];

  function logEvent(name, payload) {
    const stamp = "2026-11-08T09:10:11Z";
    const summary = `${stamp} événement ${name}`;
    writeRecord({ name, payload }, "café", summary);
    console.debug("logged", name);
    return payload;
  }

  return { prefix, events, logEvent };
}
