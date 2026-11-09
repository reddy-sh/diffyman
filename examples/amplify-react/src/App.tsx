import { useDiff, useDiffClient, type DiffymanError, type EngineInfo, type OptionsInput } from "@diffyman/react";
import { useEffect, useRef, useState, type ChangeEvent } from "react";
import { DiffView } from "./DiffView";

type Side = { mode: "text"; text: string } | { mode: "file"; file: File; text: string | null };

const SAMPLE_LEFT = `function total(items) {
  let sum = 0;
  for (const item of items) {
    sum += item.price;
  }
  return sum;
}

function label(item) {
  return item.name + ": " + item.price;
}
`;

const SAMPLE_RIGHT = `function label(item) {
  return item.name + ": " + item.price;
}

function total(entries) {
  let sum = 0;
  for (const entry of entries) {
    sum += entry.price * entry.quantity;
  }
  return sum;
}
`;

/** Files above this size are diffed but not decoded for display. */
const MAX_DISPLAY_BYTES = 64 * 1024 * 1024;

async function decodeForDisplay(file: File): Promise<string | null> {
  if (file.size > MAX_DISPLAY_BYTES) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer());
  } catch {
    return null; // Not UTF-8: the engine compares it as bytes.
  }
}

export function App() {
  const [left, setLeft] = useState<Side>({ mode: "text", text: SAMPLE_LEFT });
  const [right, setRight] = useState<Side>({ mode: "text", text: SAMPLE_RIGHT });
  const [granularity, setGranularity] = useState<NonNullable<OptionsInput["granularity"]>>("word");
  const [whitespace, setWhitespace] = useState<NonNullable<NonNullable<OptionsInput["ignore"]>["whitespace"]>>("none");
  const [detectMoves, setDetectMoves] = useState(true);
  const [context, setContext] = useState(3);

  const options: OptionsInput = {
    granularity,
    detectMoves,
    context,
    ignore: { whitespace },
    // Inline offsets in UTF-16 units index JavaScript strings directly.
    positionEncoding: "utf-16",
  };
  const input = (side: Side) => (side.mode === "text" ? side.text : side.file);
  const { document, status, error, loading } = useDiff(input(left), input(right), options, { debounceMs: 150 });

  return (
    <main>
      <header>
        <h1>diffyman in the browser</h1>
        <p>
          Both inputs stay on this page: the WASM engine runs in a Web Worker, and nothing is uploaded.
        </p>
      </header>

      <div className="inputs">
        <InputPanel label="Left" side={left} onChange={setLeft} />
        <InputPanel label="Right" side={right} onChange={setRight} />
      </div>

      <fieldset className="options">
        <legend>Options</legend>
        <label>
          Granularity{" "}
          <select value={granularity} onChange={(e) => setGranularity(e.target.value as typeof granularity)}>
            <option value="line">line</option>
            <option value="word">word</option>
            <option value="char">char</option>
            <option value="smart">smart</option>
          </select>
        </label>
        <label>
          Whitespace{" "}
          <select value={whitespace} onChange={(e) => setWhitespace(e.target.value as typeof whitespace)}>
            <option value="none">compare all</option>
            <option value="trailing">ignore trailing</option>
            <option value="change">ignore amount</option>
            <option value="all">ignore all</option>
          </select>
        </label>
        <label>
          Context{" "}
          <input
            type="number"
            min={0}
            max={100}
            value={context}
            onChange={(e) => setContext(Math.max(0, Math.min(100, Number(e.target.value) || 0)))}
          />
        </label>
        <label>
          <input type="checkbox" checked={detectMoves} onChange={(e) => setDetectMoves(e.target.checked)} /> Detect
          moved blocks
        </label>
        <span className="request-status" role="status">
          {loading ? "Comparing…" : status === "success" ? "Up to date" : status}
        </span>
      </fieldset>

      {error ? (
        <p className="error" role="alert">
          {(error as DiffymanError).code ? <code>{(error as DiffymanError).code}</code> : null} {error.message}
        </p>
      ) : null}

      {document ? <DiffView document={document} leftText={left.text} rightText={right.text} /> : null}

      <EngineFooter />
    </main>
  );
}

function InputPanel({ label, side, onChange }: { label: string; side: Side; onChange: (side: Side) => void }) {
  const id = label.toLowerCase();
  const latest = useRef<File | null>(null);
  const onFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    latest.current = file;
    onChange({ mode: "file", file, text: null });
    // Decoding is only for display; a newer pick wins.
    void decodeForDisplay(file).then((text) => {
      if (latest.current === file) onChange({ mode: "file", file, text });
    });
  };
  return (
    <div className="panel">
      <div className="panel-head">
        <label htmlFor={`${id}-text`}>{label}</label>
        <input type="file" aria-label={`${label} file`} onChange={onFile} />
        {side.mode === "file" ? (
          <button
            type="button"
            onClick={() => {
              latest.current = null;
              onChange({ mode: "text", text: side.text ?? "" });
            }}
          >
            Edit as text
          </button>
        ) : null}
      </div>
      {side.mode === "text" ? (
        <textarea
          id={`${id}-text`}
          spellCheck={false}
          value={side.text}
          onChange={(e) => onChange({ mode: "text", text: e.target.value })}
        />
      ) : (
        <p className="file-info" id={`${id}-text`}>
          {side.file.name}, {side.file.size.toLocaleString()} bytes
          {side.text === null ? " (shown as bytes)" : ""}
        </p>
      )}
    </div>
  );
}

function EngineFooter() {
  const client = useDiffClient();
  const [info, setInfo] = useState<EngineInfo | null>(null);
  useEffect(() => {
    let live = true;
    client?.info().then(
      (value) => live && setInfo(value),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [client]);
  if (info === null) return null;
  return (
    <footer>
      diffyman engine {info.engineVersion}, Diff Document {info.schemaVersion}
      {info.features.length > 0 ? `, features: ${info.features.join(", ")}` : ""}
    </footer>
  );
}
