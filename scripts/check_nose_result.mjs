let input = "";
process.stdin.setEncoding("utf8");
for await (const chunk of process.stdin) {
  input += chunk;
}
const report = JSON.parse(input);
if (!Array.isArray(report.families)) {
  throw new Error("nose JSON에 families 배열이 없습니다.");
}

if (report.families.length > 0) {
  for (const family of report.families) {
    process.stderr.write(`${family.id ?? "unknown"} (${family.baseline_status ?? "reported"})\n`);
  }
  process.exitCode = 1;
} else {
  process.stdout.write("nose: 0 new or changed families\n");
}
