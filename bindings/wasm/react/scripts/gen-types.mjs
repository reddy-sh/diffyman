// Generates src/generated/schema.ts from the JSON Schemas the CLI prints.
//
//   node scripts/gen-types.mjs [--cli PATH] [--check]
//
// The Diff Document types come from `diffyman schema` and the options types
// from `diffyman schema --options` (docs/adr/0007-binding-strategy.md: typed
// views are generated from the schema, never written by hand). `--check` exits
// with status 1 when the committed file is out of date instead of writing it.
// The default CLI is the newest of target/release and target/debug.
//
// The generator covers the subset of JSON Schema 2020-12 that the engine's
// schemas use: $ref into $defs, objects with properties/required and
// additionalProperties, arrays, string/integer/number/boolean/null types
// (also as a list), oneOf of consts (enums) and anyOf with null. Anything else
// is an error, so a new construct cannot be dropped silently.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, "..");
const repo = resolve(pkg, "..", "..", "..");
const out = resolve(pkg, "src", "generated", "schema.ts");

const args = process.argv.slice(2);
const check = args.includes("--check");
const cliIndex = args.indexOf("--cli");
const cli = cliIndex >= 0 ? resolve(args[cliIndex + 1]) : defaultCli();

function defaultCli() {
  const exe = process.platform === "win32" ? "diffyman.exe" : "diffyman";
  const found = ["release", "debug"]
    .map((profile) => resolve(repo, "target", profile, exe))
    .filter((p) => existsSync(p))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  if (found.length === 0) {
    throw new Error("no CLI binary; build it with `cargo build --release -p diffyman-cli` or pass --cli PATH");
  }
  return found[0];
}

const documentSchema = JSON.parse(execFileSync(cli, ["schema"], { encoding: "utf8" }));
const optionsSchema = JSON.parse(execFileSync(cli, ["schema", "--options"], { encoding: "utf8" }));

/** Definitions by name; the two schemas share some, which must be identical. */
const defs = new Map();
for (const schema of [documentSchema, optionsSchema]) {
  for (const [name, def] of Object.entries(schema.$defs ?? {})) {
    const seen = defs.get(name);
    if (seen !== undefined && JSON.stringify(seen) !== JSON.stringify(def)) {
      throw new Error(`$defs/${name} differs between the document and the options schema`);
    }
    defs.set(name, def);
  }
}

const KNOWN = new Set([
  "$ref", "$schema", "$defs", "title", "description", "default", "format", "minimum", "maximum",
  "type", "properties", "required", "additionalProperties", "items", "oneOf", "anyOf", "const",
]);

function checkKeywords(node, where) {
  for (const key of Object.keys(node)) {
    if (!KNOWN.has(key)) throw new Error(`${where}: unsupported keyword ${key}`);
  }
}

function refName(ref) {
  const match = /^#\/\$defs\/([A-Za-z0-9_]+)$/.exec(ref);
  if (!match) throw new Error(`unsupported $ref ${ref}`);
  return match[1];
}

function doc(text, indent) {
  if (!text) return "";
  const lines = text.replaceAll("*/", "*\\/").split("\n");
  if (lines.length === 1) return `${indent}/** ${lines[0]} */\n`;
  return `${indent}/**\n${lines.map((l) => `${indent} *${l ? ` ${l}` : ""}`).join("\n")}\n${indent} */\n`;
}

const PRIMITIVES = { string: "string", integer: "number", number: "number", boolean: "boolean", null: "null" };

/** The TypeScript type of a schema node, written at the given indent. */
function typeOf(node, indent, where) {
  // Boolean schemas: `true` accepts anything, `false` nothing.
  if (node === true) return "unknown";
  if (node === false) return "never";
  checkKeywords(node, where);
  if (node.$ref) return refName(node.$ref);
  if (node.oneOf) {
    return node.oneOf
      .map((member, i) => {
        if (!("const" in member)) return typeOf(member, indent, `${where}/oneOf/${i}`);
        checkKeywords(member, `${where}/oneOf/${i}`);
        return JSON.stringify(member.const);
      })
      .join(" | ");
  }
  if (node.anyOf) return node.anyOf.map((m, i) => typeOf(m, indent, `${where}/anyOf/${i}`)).join(" | ");
  if ("const" in node) return JSON.stringify(node.const);
  const type = node.type;
  if (type === undefined) return "unknown";
  if (Array.isArray(type)) return type.map((t) => typeOf({ ...node, type: t }, indent, where)).join(" | ");
  if (typeof type === "object") return typeOf(type, indent, `${where}/type`);
  if (type in PRIMITIVES) return PRIMITIVES[type];
  if (type === "array") {
    const item = typeOf(node.items ?? {}, indent, `${where}/items`);
    return /[| ]/.test(item) ? `Array<${item}>` : `${item}[]`;
  }
  if (type === "object") return objectBody(node, indent, where, false);
  throw new Error(`${where}: unsupported type ${JSON.stringify(type)}`);
}

/** `{ ... }` for an object node; `optional` makes every property optional. */
function objectBody(node, indent, where, optional) {
  const properties = Object.entries(node.properties ?? {});
  const open = node.additionalProperties === true;
  if (properties.length === 0) return open ? "Record<string, unknown>" : "Record<string, never>";
  const required = new Set(node.required ?? []);
  const inner = `${indent}  `;
  let body = "{\n";
  for (const [name, prop] of properties.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
    const key = /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name) ? name : JSON.stringify(name);
    const mark = optional || !required.has(name) ? "?" : "";
    body += doc(prop.description, inner);
    body += `${inner}${key}${mark}: ${typeOf(prop, inner, `${where}/properties/${name}`)};\n`;
  }
  if (open) body += `${inner}/** Further keys are allowed here. */\n${inner}[key: string]: unknown;\n`;
  return `${body}${indent}}`;
}

/** Input types: every key is optional (spec section 9), at every depth. */
const INPUT_DEFS = new Set(["IgnoreOptionsInput", "TreeOptionsInput"]);

function declaration(name, node) {
  let text = doc(node.description, "");
  const isObject = node.type === "object" && !node.$ref && !node.oneOf && !node.anyOf;
  if (isObject && node.properties) {
    text += `export interface ${name} ${objectBody(node, "", `$defs/${name}`, INPUT_DEFS.has(name))}\n`;
  } else {
    text += `export type ${name} = ${typeOf(node, "", `$defs/${name}`)};\n`;
  }
  return text;
}

const rootDoc = { ...documentSchema };
delete rootDoc.$defs;
delete rootDoc.$schema;
const rootOptions = { ...optionsSchema };
delete rootOptions.$defs;
delete rootOptions.$schema;

let ts = `// Generated by scripts/gen-types.mjs from \`diffyman schema\` and
// \`diffyman schema --options\`. Do not edit; run \`npm run gen:types\`.

/* eslint-disable */

`;
ts += declaration(documentSchema.title, rootDoc) + "\n";
INPUT_DEFS.add(optionsSchema.title);
ts += declaration(optionsSchema.title, rootOptions);
for (const name of [...defs.keys()].sort()) {
  ts += "\n" + declaration(name, defs.get(name));
}

if (check) {
  const current = existsSync(out) ? readFileSync(out, "utf8") : "";
  if (current !== ts) {
    process.stderr.write(`${out} is out of date; run \`npm run gen:types\`\n`);
    process.exit(1);
  }
  process.stdout.write("generated types are current\n");
} else {
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, ts);
  process.stdout.write(`wrote ${out}\n`);
}
