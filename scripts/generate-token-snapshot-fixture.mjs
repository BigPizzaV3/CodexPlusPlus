import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [directory, countText = "30000"] = process.argv.slice(2);
const count = Number(countText);
if (!directory || !Number.isSafeInteger(count) || count < 1) {
  throw new Error("Usage: node scripts/generate-token-snapshot-fixture.mjs <new-directory> [session-count]");
}

mkdirSync(directory);
const timestamp = "2026-01-02T12:00:00Z";
for (let index = 0; index < count; index++) {
  const id = `00000000-0000-4000-8000-${index.toString(16).padStart(12, "0")}`;
  const rows = [
    { type: "session_meta", timestamp, ordinal: 0, payload: { id, model_provider: "synthetic" } },
    { type: "turn_context", timestamp, ordinal: 1, payload: { model: "synthetic-model", turn_id: "first" } },
    { type: "event_msg", timestamp, ordinal: 2, payload: { type: "token_count", info: {
      total_token_usage: { input_tokens: 10, cached_input_tokens: 4, output_tokens: 2, total_tokens: 12 },
    } } },
  ];
  writeFileSync(join(directory, `rollout-2026-01-02T12-00-00-${id}.jsonl`), rows.map((row) => JSON.stringify(row)).join("\n") + "\n");
}
console.log(`Generated ${count} synthetic sessions: ${count * 12} total tokens.`);
