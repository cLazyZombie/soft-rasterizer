import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const assetUrl = new URL("../web/assets/Fox.glb", import.meta.url);
const noticeUrl = new URL("../web/assets/Fox.NOTICE.md", import.meta.url);
const expectedBytes = 162_852;
const expectedSha256 = "d97044e701822bac5a62696459b27d7b375aada5de8574ed4362edbba94771f7";

const asset = await readFile(assetUrl);
if (asset.byteLength !== expectedBytes) {
  throw new Error(`Fox.glb byte length ${asset.byteLength} != ${expectedBytes}`);
}
const sha256 = createHash("sha256").update(asset).digest("hex");
if (sha256 !== expectedSha256) {
  throw new Error(`Fox.glb SHA-256 ${sha256} != ${expectedSha256}`);
}

const notice = await readFile(noticeUrl, "utf8");
for (const required of [
  expectedSha256,
  "2d97dcc2463db123ed5203598cffedf8b6cf1683",
  "Models/Fox/glTF-Binary/Fox.glb",
  "https://github.com/KhronosGroup/glTF-Sample-Assets/tree/2d97dcc2463db123ed5203598cffedf8b6cf1683/Models/Fox",
  "Byte length: `162852`",
  "PixelMannen",
  "tomkranis",
  "AsoboStudio",
  "scurest",
  "CC0 1.0 Universal",
  "CC BY 4.0",
  "https://creativecommons.org/publicdomain/zero/1.0/legalcode",
  "https://creativecommons.org/licenses/by/4.0/legalcode",
  "No changes were made to the vendored GLB bytes.",
]) {
  if (!notice.includes(required)) {
    throw new Error(`Fox.NOTICE.md에 필수 attribution '${required}'가 없습니다`);
  }
}

console.log(`asset check passed: ${repositoryRoot}web/assets/Fox.glb (${expectedBytes} bytes, ${sha256})`);
