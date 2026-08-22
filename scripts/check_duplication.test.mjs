import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const checkScript = path.join(scriptDirectory, "check_duplication.sh");

test("nose가 아닌 실행 파일은 거부한다", () => {
  const result = spawnSync(checkScript, {
    encoding: "utf8",
    env: { ...process.env, NOSE_BIN: process.execPath },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /binary does not identify as nose/);
});
test("조회된 최신 Homebrew 버전보다 오래된 nose는 거부한다", async (context) => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "soft-rasterizer-nose-old-"));
  context.after(() => rm(temporaryDirectory, { recursive: true, force: true }));
  const nosePath = path.join(temporaryDirectory, "nose");
  const brewPath = path.join(temporaryDirectory, "brew");
  await writeFile(nosePath, '#!/usr/bin/env bash\nprintf "nose 1.0.0\\n"\n');
  await writeFile(
    brewPath,
    '#!/usr/bin/env bash\nprintf \'{"formulae":[{"versions":{"stable":"2.0.0"}}]}\\n\'\n',
  );
  await chmod(nosePath, 0o755);
  await chmod(brewPath, 0o755);

  const result = spawnSync(checkScript, {
    encoding: "utf8",
    env: {
      ...process.env,
      NOSE_BIN: nosePath,
      PATH: `${temporaryDirectory}:${process.env.PATH}`,
    },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /latest nose 2\.0\.0 is required/);
});

test("skip 환경에서는 Rust root, baseline, 판정 규칙을 전달한다", async (context) => {
  const temporaryDirectory = await mkdtemp(path.join(os.tmpdir(), "soft-rasterizer-nose-args-"));
  context.after(() => rm(temporaryDirectory, { recursive: true, force: true }));
  const nosePath = path.join(temporaryDirectory, "nose");
  const argumentsPath = path.join(temporaryDirectory, "arguments.txt");
  await writeFile(
    nosePath,
    `#!/usr/bin/env bash\nif [[ "$1" == "--version" ]]; then printf "nose 1.0.0\\n"; exit 0; fi\nprintf "%s\\n" "$@" > "${argumentsPath}"\nprintf '{"tool":"nose","schema_version":9,"families":[]}'\n`,
  );
  await chmod(nosePath, 0o755);

  const result = spawnSync(checkScript, {
    encoding: "utf8",
    env: {
      ...process.env,
      NOSE_BIN: nosePath,
      SOFT_RASTERIZER_SKIP_NOSE_VERSION_CHECK: "1",
    },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /0 new or changed families/);
  const argumentsText = await readFile(argumentsPath, "utf8");
  assert.match(argumentsText, /renderer-core\/src/);
  assert.match(argumentsText, /renderer-wasm\/src/);
  assert.match(argumentsText, /\.nose-baseline\.json/);
  assert.match(argumentsText, /nose\.ignore\.json/);
  assert.match(argumentsText, /lines>7/);
  assert.match(argumentsText, /shared>5/);
  assert.doesNotMatch(argumentsText, /target\//);
});
