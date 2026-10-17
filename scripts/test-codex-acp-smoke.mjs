// Exercises the real adapter + local CLI with a temporary config and a fake
// localhost provider. No native credentials and no paid inference are used.
import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import assert from "node:assert/strict";

const [adapter, cli, petPolicy] = process.argv.slice(2);
if (!adapter || !cli) throw new Error("Usage: node scripts/test-codex-acp-smoke.mjs <adapter index.js> <local codex>");
const root = await mkdtemp(join(tmpdir(), "runjam-codex-smoke-"));
let promptRequests = 0;
const server = createServer(async (req, res) => {
  assert.equal(req.headers.authorization, "Bearer local-test-token");
  if (req.method === "POST" && req.url.endsWith("/responses")) {
    let body = "";
    for await (const chunk of req) body += chunk;
    const payload = JSON.parse(body);
    assert.equal(payload.model, "runjam-test-model");
    if (petPolicy) {
      const toolNames = (payload.tools ?? []).map(tool => tool.name ?? tool.type);
      assert.ok(!toolNames.some(name => /exec_command|write_stdin|shell|spawn_agent|send_input|mcp__/.test(name)), `Forbidden pet tool: ${toolNames.join(", ")}`);
      console.log(`Pet tool inventory: ${toolNames.join(", ")}`);
    }
    promptRequests++;
    const text = "Local fake-provider reply";
    const part = {type: "output_text", text, annotations: []};
    const item = {id: "msg_test", type: "message", role: "assistant", status: "completed", content: [part]};
    const response = {id: "resp_test", object: "response", created_at: 0, status: "completed", model: "runjam-test-model", output: [item], usage: {input_tokens: 1, output_tokens: 1, total_tokens: 2}};
    const events = [
      {type: "response.created", response: {...response, status: "in_progress", output: []}},
      {type: "response.output_item.added", output_index: 0, item: {...item, status: "in_progress", content: []}},
      {type: "response.content_part.added", item_id: item.id, output_index: 0, content_index: 0, part: {...part, text: ""}},
      {type: "response.output_text.delta", item_id: item.id, output_index: 0, content_index: 0, delta: text},
      {type: "response.output_text.done", item_id: item.id, output_index: 0, content_index: 0, text},
      {type: "response.content_part.done", item_id: item.id, output_index: 0, content_index: 0, part},
      {type: "response.output_item.done", output_index: 0, item},
      {type: "response.completed", response},
    ];
    res.writeHead(200, {"Content-Type": "text/event-stream"});
    for (const [sequence_number, event] of events.entries()) res.write(`event: ${event.type}\ndata: ${JSON.stringify({...event, sequence_number})}\n\n`);
    res.end();
    return;
  }
  res.writeHead(200, {"Content-Type": "application/json"});
  res.end(JSON.stringify({object: "list", data: [{id: "runjam-test-model", object: "model"}]}));
});
let child;
try {
  await new Promise((ok, reject) => {server.once("error", reject); server.listen(0, "127.0.0.1", ok);});
  const config = {
    model: "runjam-test-model", model_provider: "runjam", cli_auth_credentials_store: "ephemeral",
    model_providers: {runjam: {name: "Smoke test", base_url: `http://127.0.0.1:${server.address().port}/v1`, wire_api: "responses", env_key: "RUNJAM_PROXY_TOKEN", requires_openai_auth: false}},
    sandbox_mode: "read-only", approval_policy: "on-request",
    ...(petPolicy ? JSON.parse(petPolicy) : {}),
  };
  await writeFile(join(root, "config.toml"), `model = "runjam-test-model"\nmodel_provider = "runjam"\ncli_auth_credentials_store = "ephemeral"\n[model_providers.runjam]\nname = "Smoke test"\nbase_url = "${config.model_providers.runjam.base_url}"\nwire_api = "responses"\nenv_key = "RUNJAM_PROXY_TOKEN"\nrequires_openai_auth = false\n`);
  const env = {...process.env, CODEX_HOME: root, CODEX_PATH: resolve(cli), CODEX_CONFIG: JSON.stringify(config), MODEL_PROVIDER: "runjam", RUNJAM_PROXY_TOKEN: "local-test-token"};
  for (const key of ["CODEX_API_KEY", "OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_API_BASE", "CODEX_API_BASE", "DEFAULT_AUTH_REQUEST", "INITIAL_AGENT_MODE", "APP_SERVER_LOGS"]) delete env[key];
  child = spawn(process.execPath, [resolve(adapter)], {env, cwd: root, detached: process.platform !== "win32", stdio: "pipe"});
  let stderr = "";
  child.stderr.on("data", data => {stderr = (stderr + data.toString()).slice(-3000);});
  const waiting = new Map();
  let streamedText = "";
  const reader = createInterface({input: child.stdout});
  reader.on("line", line => {
    const response = JSON.parse(line);
    if (response.params?.update?.sessionUpdate === "agent_message_chunk") streamedText += response.params.update.content?.text ?? "";
    const pending = waiting.get(response.id);
    if (pending) {
      waiting.delete(response.id);
      clearTimeout(pending.timer);
      response.error ? pending.reject(new Error(JSON.stringify(response.error))) : pending.ok(response.result);
    }
  });
  let id = 0;
  function request(method, params) {
    return new Promise((ok, reject) => {
      const requestId = ++id;
      const timer = setTimeout(() => {waiting.delete(requestId); reject(new Error(`${method} timed out: ${stderr}`));}, 20000);
      waiting.set(requestId, {ok, reject, timer});
      child.stdin.write(JSON.stringify({jsonrpc: "2.0", id: requestId, method, params}) + "\n");
    });
  }
  child.on("exit", code => {
    for (const pending of waiting.values()) {clearTimeout(pending.timer); pending.reject(new Error(`ACP exited (${code}): ${stderr}`));}
    waiting.clear();
  });
  const init = await request("initialize", {protocolVersion: 1, clientCapabilities: {}, clientInfo: {name: "runjam-smoke", version: "1"}});
  console.log(`Initialized ${init.agentInfo?.name} ${init.agentInfo?.version}`);
  const session = await request("session/new", {cwd: root, mcpServers: []});
  assert.ok(session.sessionId);
  const modelOption = session.configOptions?.find(option => option.id === "model");
  const actualModel = modelOption?.currentValue ?? session.models?.currentModelId;
  assert.ok(actualModel?.startsWith("runjam-test-model"), `Unexpected selected model: ${actualModel}`);
  console.log(`Created session using exact managed model: ${actualModel}`);
  const result = await request("session/prompt", {sessionId: session.sessionId, prompt: [{type: "text", text: "Reply from the local test provider only."}]});
  assert.equal(result.stopReason, "end_turn");
  assert.equal(promptRequests, 1);
  assert.ok(streamedText.includes("Local fake-provider reply"));
  console.log("Fake local response streamed successfully; no paid inference or native credentials used.");
} finally {
  if (child?.pid) {
    if (process.platform === "win32") spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"]);
    else {try {process.kill(-child.pid, "SIGTERM");} catch {}}
    await new Promise(ok => child.exitCode !== null ? ok() : child.once("exit", ok));
    // The adapter may exit before its app-server child finishes flushing state.
    if (process.platform !== "win32") {try {process.kill(-child.pid, "SIGKILL");} catch {}}
  }
  await new Promise(ok => server.close(ok));
  await rm(root, {recursive: true, force: true, maxRetries: 10, retryDelay: 100});
}
