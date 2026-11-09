// Generates types/document.d.ts from the JSON Schemas the engine publishes:
// `diffyman schema` (the Diff Document) and `diffyman schema --options` (the
// options a caller passes). Typed views are generated, never written by hand
// (docs/adr/0007-binding-strategy.md, decision 3).
//
//   node scripts/generate-types.mjs [--cli PATH] [--check]
//   node scripts/generate-types.mjs --schema FILE --options-schema FILE [--check]
//
// The CLI defaults to $DIFFYMAN_CLI, then the newest of target/release and
// target/debug. --check writes nothing and exits with status 1 when the file
// on disk differs from what the schemas give.
//
// Only the JSON Schema keywords the engine's schemas use are supported; any
// other keyword stops the generator, so a schema change cannot be dropped
// silently.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(here, "..");
const repo = resolve(pkgDir, "..", "..");
const outFile = join(pkgDir, "types", "document.d.ts");

// Keywords that only document a value: they never change the TypeScript type.
const ANNOTATIONS = new Set(["description", "default", "title", "$schema", "format", "minimum", "maximum"]);
const STRUCTURAL = new Set(["type", "const", "$ref", "oneOf", "anyOf", "items", "properties", "required", "additionalProperties", "$defs"]);

function parseArgs(argv) {
  const args = { check: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--check") args.check = true;
    else if (arg === "--cli") args.cli = argv[++i];
    else if (arg === "--schema") args.schema = argv[++i];
    else if (arg === "--options-schema") args.optionsSchema = argv[++i];
    else throw new Error(`unknown argument ${arg}`);
  }
  if (Boolean(args.schema) !== Boolean(args.optionsSchema)) {
    throw new Error("--schema and --options-schema go together");
  }
  return args;
}

function findCli(explicit) {
  if (explicit) return resolve(explicit);
  if (process.env.DIFFYMAN_CLI) return resolve(process.env.DIFFYMAN_CLI);
  const exe = process.platform === "win32" ? "diffyman.exe" : "diffyman";
  const found = ["release", "debug"]
    .map((profile) => join(repo, "target", profile, exe))
    .filter((p) => existsSync(p))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  if (found.length === 0) {
    throw new Error("no diffyman CLI found; build it with `cargo build --release -p diffyman-cli` or pass --cli PATH");
  }
  return found[0];
}

function loadSchemas(args) {
  if (args.schema) {
    return [JSON.parse(readFileSync(args.schema, "utf8")), JSON.parse(readFileSync(args.optionsSchema, "utf8"))];
  }
  const cli = findCli(args.cli);
  const run = (...extra) => JSON.parse(execFileSync(cli, ["schema", ...extra], { encoding: "utf8" }));
  return [run(), run("--options")];
}

function checkKeywords(schema, where) {
  if (typeof schema === "boolean") return;
  for (const key of Object.keys(schema)) {
    if (!ANNOTATIONS.has(key) && !STRUCTURAL.has(key)) {
      throw new Error(`${where}: unsupported JSON Schema keyword ${JSON.stringify(key)}`);
    }
  }
}

// A JSDoc block, indented. A comment terminator inside a description is
// escaped, so it cannot end the comment early.
function jsdoc(text, indent) {
  if (!text) return "";
  const lines = text.replaceAll("*/", "*\\/").split("\n");
  if (lines.length === 1) return `${indent}/** ${lines[0]} */\n`;
  return `${indent}/**\n${lines.map((l) => (l ? `${indent} * ${l}` : `${indent} *`)).join("\n")}\n${indent} */\n`;
}

function propertyKey(name) {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name) ? name : JSON.stringify(name);
}

function refName(ref, where) {
  const match = /^#\/\$defs\/([A-Za-z][A-Za-z0-9]*)$/.exec(ref);
  if (!match) throw new Error(`${where}: unsupported $ref ${ref}`);
  return match[1];
}

/** The TypeScript type of a schema, inline. */
function typeOf(schema, where, indent) {
  if (schema === true) return "unknown";
  if (schema === false) return "never";
  checkKeywords(schema, where);
  if (schema.$ref) return refName(schema.$ref, where);
  if ("const" in schema) return JSON.stringify(schema.const);
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants) {
    return union(variants.map((v, i) => typeOf(v, `${where}/${schema.oneOf ? "oneOf" : "anyOf"}/${i}`, indent)));
  }
  if (Array.isArray(schema.type)) {
    return union(schema.type.map((t) => typeOf({ ...schema, type: t }, where, indent)));
  }
  switch (schema.type) {
    case undefined:
      return "unknown";
    case "string":
      return "string";
    case "integer":
    case "number":
      return "number";
    case "boolean":
      return "boolean";
    case "null":
      return "null";
    case "array": {
      const item = typeOf(schema.items ?? true, `${where}/items`, indent);
      return /^[A-Za-z0-9_]+$/.test(item) ? `${item}[]` : `Array<${item}>`;
    }
    case "object":
      return objectType(schema, where, indent);
    default:
      throw new Error(`${where}: unsupported type ${JSON.stringify(schema.type)}`);
  }
}

function union(types) {
  return [...new Set(types)].join(" | ");
}

function objectBody(schema, where, indent) {
  const required = new Set(schema.required ?? []);
  const inner = `${indent}  `;
  let body = "";
  for (const [name, prop] of Object.entries(schema.properties ?? {})) {
    const description = typeof prop === "object" ? prop.description : undefined;
    const optional = required.has(name) ? "" : "?";
    body += jsdoc(description, inner);
    body += `${inner}${propertyKey(name)}${optional}: ${typeOf(prop, `${where}/properties/${name}`, inner)};\n`;
  }
  for (const name of required) {
    if (!(name in (schema.properties ?? {}))) throw new Error(`${where}: required ${name} has no schema`);
  }
  const extra = schema.additionalProperties;
  if (extra !== undefined && extra !== false) {
    body += `${inner}[key: string]: ${typeOf(extra, `${where}/additionalProperties`, inner)};\n`;
  }
  return body;
}

function objectType(schema, where, indent) {
  if (!schema.properties && schema.additionalProperties !== undefined && schema.additionalProperties !== false) {
    return `Record<string, ${typeOf(schema.additionalProperties, `${where}/additionalProperties`, indent)}>`;
  }
  const body = objectBody(schema, where, indent);
  return body ? `{\n${body}${indent}}` : "Record<string, never>";
}

/** One exported declaration: an interface for an object, a type alias otherwise. */
function declaration(name, schema) {
  const where = `#/$defs/${name}`;
  checkKeywords(schema, where);
  let out = jsdoc(schema.description, "");
  if (schema.type === "object" && schema.properties) {
    return `${out}export interface ${name} {\n${objectBody(schema, where, "")}}\n`;
  }
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants && variants.every((v) => typeof v === "object" && "const" in v)) {
    // An enum: one line per value, each with its own description.
    out += `export type ${name} =\n`;
    variants.forEach((v, i) => {
      checkKeywords(v, `${where}/${i}`);
      out += jsdoc(v.description, "  ");
      out += `  | ${JSON.stringify(v.const)}${i === variants.length - 1 ? ";" : ""}\n`;
    });
    return out;
  }
  return `${out}export type ${name} = ${typeOf(schema, where, "")};\n`;
}

function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stableJson(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function generate(documentSchema, optionsSchema) {
  const defs = new Map();
  const roots = [];
  for (const schema of [documentSchema, optionsSchema]) {
    if (!schema.title) throw new Error("a root schema has no title");
    roots.push([schema.title, Object.fromEntries(Object.entries(schema).filter(([k]) => k !== "$defs"))]);
    for (const [name, def] of Object.entries(schema.$defs ?? {})) {
      // A definition shared by both schemas must be the same in both.
      if (defs.has(name) && stableJson(defs.get(name)) !== stableJson(def)) {
        throw new Error(`#/$defs/${name} differs between the document and options schemas`);
      }
      defs.set(name, def);
    }
  }
  for (const [name] of roots) {
    if (defs.has(name)) throw new Error(`root ${name} clashes with a definition`);
  }
  let out =
    "// Generated by scripts/generate-types.mjs from `diffyman schema` and\n" +
    "// `diffyman schema --options`. Do not edit: run `npm run types` instead.\n" +
    "//\n" +
    "// Enums are open (docs/spec/diff-document.md section 2.3): a newer engine may\n" +
    "// write values that are not listed here.\n";
  for (const [name, schema] of roots) {
    out += `\n${declaration(name, schema)}`;
  }
  for (const name of [...defs.keys()].sort()) {
    out += `\n${declaration(name, defs.get(name))}`;
  }
  return out;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const [documentSchema, optionsSchema] = loadSchemas(args);
  const text = generate(documentSchema, optionsSchema);
  if (args.check) {
    const current = existsSync(outFile) ? readFileSync(outFile, "utf8") : "";
    if (current !== text) {
      process.stderr.write(`${outFile} is out of date with the engine's JSON Schema; run \`npm run types\`.\n`);
      process.exit(1);
    }
    process.stdout.write(`${outFile} is up to date.\n`);
    return;
  }
  mkdirSync(dirname(outFile), { recursive: true });
  writeFileSync(outFile, text);
  process.stdout.write(`wrote ${outFile}\n`);
}

main();
