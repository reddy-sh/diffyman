// Renders a Diff Document: text side by side (hunks, inline spans, moved
// blocks), binary byte ranges, or tree entries. Every piece of input text is a
// React text node; nothing is injected as HTML.

import type { DiffDocument, Move, OpKind, PositionEncoding, Span } from "@diffyman/react";
import { Fragment, type ReactNode } from "react";
import { pieces, segments, splitLines } from "./lines";

/** Rows beyond this are not drawn; the summary still counts everything. */
const MAX_ROWS = 5000;

export interface DiffViewProps {
  document: DiffDocument;
  /** The decoded inputs, for kind `text`. */
  leftText: string | null;
  rightText: string | null;
}

export function DiffView({ document, leftText, rightText }: DiffViewProps) {
  return (
    <section className="diff" aria-label="Diff result">
      <Summary document={document} />
      {document.kind === "text" && leftText !== null && rightText !== null ? (
        <TextView document={document} leftLines={splitLines(leftText)} rightLines={splitLines(rightText)} />
      ) : null}
      {document.kind === "binary" ? <BinaryView document={document} /> : null}
      {document.kind === "tree" ? <TreeView document={document} /> : null}
    </section>
  );
}

function Summary({ document }: { document: DiffDocument }) {
  const { stats } = document;
  return (
    <div className="summary">
      <span className={`badge status-${document.status}`}>{document.status}</span>
      <span>kind {document.kind}</span>
      {stats && document.kind === "text" ? (
        <>
          <span className="added">+{stats.linesAdded}</span>
          <span className="removed">−{stats.linesRemoved}</span>
          <span>{stats.hunkCount} hunks</span>
        </>
      ) : null}
      {stats?.bytes ? (
        <span>
          bytes +{stats.bytes.added} −{stats.bytes.removed}
        </span>
      ) : null}
      {stats ? <span>similarity {(stats.similarity * 100).toFixed(2)}%</span> : null}
      {document.moves.length > 0 ? <span className="moved-label">{document.moves.length} moved blocks</span> : null}
      {document.warnings.length > 0 ? (
        <ul className="warnings">
          {document.warnings.map((w) => (
            <li key={`${w.code}:${w.side ?? ""}`} className={`severity-${w.severity}`}>
              <code>{w.code}</code>
              {w.side ? ` (${w.side})` : ""}: {w.message}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

interface MoveMark {
  move: Move;
  /** First line of the other end of the move. */
  otherStart: number;
}

interface Cell {
  line: number;
  text: string;
  spans: Span[];
  move: MoveMark | null;
}

interface Row {
  op: OpKind;
  left: Cell | null;
  right: Cell | null;
}

interface HunkRows {
  id: string;
  header: string;
  rows: Row[];
}

function TextView({
  document,
  leftLines,
  rightLines,
}: {
  document: DiffDocument;
  leftLines: string[];
  rightLines: string[];
}) {
  if (document.hunks.length === 0) {
    return <p className="empty">{document.status === "error" ? "No hunks: see the errors above." : "No changes."}</p>;
  }
  const { hunks, truncated } = buildRows(document, leftLines, rightLines);
  const encoding = document.positionEncoding;
  return (
    <div className="table-wrap">
      <table className="sbs">
        <colgroup>
          <col className="num-col" />
          <col />
          <col className="num-col" />
          <col />
        </colgroup>
        <thead>
          <tr>
            <th scope="col" colSpan={2}>
              {document.left.name ?? "left"}
            </th>
            <th scope="col" colSpan={2}>
              {document.right.name ?? "right"}
            </th>
          </tr>
        </thead>
        <tbody>
          {hunks.map((hunk) => (
            <Fragment key={hunk.id}>
              <tr className="hunk-header">
                <td colSpan={4}>{hunk.header}</td>
              </tr>
              {hunk.rows.map((row, i) => (
                <tr key={i} className={`row op-${row.op}`}>
                  <CodeCells cell={row.left} side="left" op={row.op} encoding={encoding} />
                  <CodeCells cell={row.right} side="right" op={row.op} encoding={encoding} />
                </tr>
              ))}
            </Fragment>
          ))}
        </tbody>
      </table>
      {truncated ? <p className="empty">Only the first {MAX_ROWS} rows are shown.</p> : null}
    </div>
  );
}

function CodeCells({
  cell,
  side,
  op,
  encoding,
}: {
  cell: Cell | null;
  side: "left" | "right";
  op: OpKind;
  encoding: PositionEncoding;
}) {
  if (cell === null) {
    return (
      <>
        <td className="num" />
        <td className="code filler" />
      </>
    );
  }
  const changed = op !== "equal";
  const sign = !changed ? " " : cell.move ? "~" : side === "left" ? "−" : "+";
  const classes = ["code", changed ? (side === "left" ? "del" : "ins") : "", cell.move ? "moved" : ""];
  return (
    <>
      <td className="num">{cell.line}</td>
      <td className={classes.join(" ")}>
        <span className="sign" aria-hidden="true">
          {sign}
        </span>
        {segments(cell.text, cell.spans, encoding).map((seg, i) =>
          seg.changed ? (
            <mark key={i} className={side === "left" ? "span-del" : "span-ins"}>
              <Visible text={seg.text} />
            </mark>
          ) : (
            <Visible key={i} text={seg.text} />
          ),
        )}
        {cell.move ? (
          <span className="move-badge" title={`${cell.move.move.kind} move ${cell.move.move.id}`}>
            {cell.move.move.id} {cell.move.move.kind}
            {side === "left" ? ` → ${cell.move.otherStart}` : ` ← ${cell.move.otherStart}`}
          </span>
        ) : null}
      </td>
    </>
  );
}

/** Text with invisible and bidi-control characters shown as markers. */
function Visible({ text }: { text: string }): ReactNode {
  return pieces(text).map((piece, i) =>
    piece.hidden ? (
      <span key={i} className="hidden-char">
        {piece.text}
      </span>
    ) : (
      <Fragment key={i}>{piece.text}</Fragment>
    ),
  );
}

function buildRows(document: DiffDocument, leftLines: string[], rightLines: string[]) {
  const spans = new Map<string, Span[]>();
  const addSpan = (span: Span) => {
    const key = `${span.side}:${span.line}`;
    const list = spans.get(key);
    if (list) list.push(span);
    else spans.set(key, [span]);
  };
  const leftMoves = new Map<number, MoveMark>();
  const rightMoves = new Map<number, MoveMark>();
  for (const move of document.moves) {
    // A moved line's spans are only in its move (spec section 6.5).
    move.inline.forEach(addSpan);
    for (let l = move.from.range.start; l < move.from.range.start + move.from.range.count; l++) {
      leftMoves.set(l, { move, otherStart: move.to.range.start });
    }
    for (let l = move.to.range.start; l < move.to.range.start + move.to.range.count; l++) {
      rightMoves.set(l, { move, otherStart: move.from.range.start });
    }
  }
  for (const hunk of document.hunks) for (const op of hunk.ops) op.inline.forEach(addSpan);
  for (const list of spans.values()) list.sort((a, b) => a.start - b.start);

  const cell = (side: "left" | "right", line: number): Cell => ({
    line,
    text: (side === "left" ? leftLines : rightLines)[line - 1] ?? "",
    spans: spans.get(`${side}:${line}`) ?? [],
    move: (side === "left" ? leftMoves : rightMoves).get(line) ?? null,
  });

  let total = 0;
  let truncated = false;
  const hunks: HunkRows[] = [];
  for (const hunk of document.hunks) {
    const rows: Row[] = [];
    for (const op of hunk.ops) {
      const { leftRange: l, rightRange: r } = op;
      const height = op.op === "equal" ? l.count : Math.max(l.count, r.count);
      for (let i = 0; i < height; i++) {
        if (total >= MAX_ROWS) {
          truncated = true;
          break;
        }
        rows.push({
          op: op.op,
          left: i < l.count ? cell("left", l.start + i) : null,
          right: i < r.count ? cell("right", r.start + i) : null,
        });
        total++;
      }
    }
    hunks.push({
      id: hunk.id,
      header: `@@ -${hunk.left.start},${hunk.left.count} +${hunk.right.start},${hunk.right.count} @@`,
      rows,
    });
    if (truncated) break;
  }
  return { hunks, truncated };
}

function BinaryView({ document }: { document: DiffDocument }) {
  if (document.bytes.length === 0) return <p className="empty">No changed bytes.</p>;
  const hex = (n: number) => `0x${n.toString(16).padStart(8, "0")}`;
  return (
    <div className="table-wrap">
      <table className="grid">
        <thead>
          <tr>
            <th scope="col">op</th>
            <th scope="col">left offset</th>
            <th scope="col">left length</th>
            <th scope="col">right offset</th>
            <th scope="col">right length</th>
          </tr>
        </thead>
        <tbody>
          {document.bytes.slice(0, MAX_ROWS).map((range, i) => (
            <tr key={i} className={`op-${range.op}`}>
              <td>{range.op}</td>
              <td>{hex(range.leftOffset)}</td>
              <td>{range.leftLength}</td>
              <td>{hex(range.rightOffset)}</td>
              <td>{range.rightLength}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function TreeView({ document }: { document: DiffDocument }) {
  const entries = document.tree?.entries ?? [];
  const changed = entries.filter((e) => e.status !== "unchanged");
  if (changed.length === 0) return <p className="empty">No changed entries.</p>;
  return (
    <div className="table-wrap">
      <table className="grid">
        <thead>
          <tr>
            <th scope="col">status</th>
            <th scope="col">path</th>
            <th scope="col">left size</th>
            <th scope="col">right size</th>
            <th scope="col">child</th>
          </tr>
        </thead>
        <tbody>
          {changed.slice(0, MAX_ROWS).map((entry) => (
            <tr key={entry.path} className={`entry-${entry.status}`}>
              <td>{entry.status}</td>
              <td>
                <Visible text={entry.fromPath ? `${entry.fromPath} → ${entry.path}` : entry.path} />
              </td>
              <td>{entry.left?.size ?? ""}</td>
              <td>{entry.right?.size ?? ""}</td>
              <td>
                {entry.childDocument
                  ? `${entry.childDocument.kind}, ${entry.childDocument.status}${
                      entry.childDocument.stats ? `, +${entry.childDocument.stats.linesAdded} −${entry.childDocument.stats.linesRemoved}` : ""
                    }`
                  : ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {entries.length > changed.length ? (
        <p className="empty">{entries.length - changed.length} unchanged entries not shown.</p>
      ) : null}
    </div>
  );
}
