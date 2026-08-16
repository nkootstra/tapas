#!/usr/bin/env node

import { createInterface } from "node:readline";

const encode = (value) => Buffer.from(value).toString("base64");
const decode = (value) => Buffer.from(value, "base64");
const lines = (value) => {
  const result = [];
  let start = 0;
  for (let index = 0; index < value.length; index += 1) {
    if (value[index] === 10) {
      result.push(value.subarray(start, index + 1));
      start = index + 1;
    }
  }
  if (start < value.length) result.push(value.subarray(start));
  return result;
};

process.stdout.write(JSON.stringify({ protocol: "tapas-filter", versions: [1] }) + "\n");

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.once("line", (record) => {
  const request = JSON.parse(record);
  const argv = request.argv_b64.map((argument) => decode(argument).toString("utf8"));
  const shape = argv[0]?.split(/[\\/]/).at(-1) === "acme" ? argv[1] : undefined;
  if (shape !== "test" && shape !== "build") {
    process.stdout.write(JSON.stringify({ version: 1, result: "decline" }) + "\n");
    return;
  }

  const stdout = lines(decode(request.stdout_b64));
  const stderr = lines(decode(request.stderr_b64));
  const repeated = Buffer.from(shape === "test" ? "PASS " : "COMPILE ");
  const count = stdout.filter((line) => line.subarray(0, repeated.length).equals(repeated)).length;
  const compactStdout = Buffer.concat([
    Buffer.from(`${repeated.toString().trim()} ${count} ${shape === "test" ? "cases" : "targets"}\n`),
    ...stdout.filter((line) => !line.subarray(0, repeated.length).equals(repeated)),
  ]);
  const warningPrefix = Buffer.from("WARN ");
  const warnings = stderr.filter((line) => line.subarray(0, warningPrefix.length).equals(warningPrefix));
  let firstWarning = warnings[0];
  while (firstWarning?.length && (firstWarning.at(-1) === 10 || firstWarning.at(-1) === 13)) {
    firstWarning = firstWarning.subarray(0, -1);
  }
  const compactStderr = Buffer.concat([
    ...(warnings.length
      ? [firstWarning, Buffer.from(` (repeated ${warnings.length} times)\n`)]
      : []),
    ...stderr.filter((line) => !line.subarray(0, warningPrefix.length).equals(warningPrefix)),
  ]);
  process.stdout.write(
    JSON.stringify({
      version: 1,
      result: "transform",
      evidence: "fact-complete",
      stdout_b64: encode(compactStdout),
      stderr_b64: encode(compactStderr),
    }) + "\n",
  );
});
