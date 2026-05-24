import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { buildEventInput, parseCliArgs } from "../src/index.js";

describe("parseCliArgs", () => {
  it("parses flags and serializes waiting-input as waiting_input", () => {
    const parsed = parseCliArgs([
      "--server",
      "http://server",
      "--token",
      "secret",
      "--agent",
      "claude",
      "--host",
      "workstation",
      "--repo",
      "agent-notify",
      "--state",
      "waiting-input",
      "--summary",
      "waiting for input",
      "--priority",
      "90",
      "--ttl-seconds",
      "30",
      "--run-id",
      "abc123",
    ]);

    assert.equal(parsed.exit, undefined);
    assert.deepEqual(parsed.options, {
      server: "http://server",
      token: "secret",
      agent: "claude",
      host: "workstation",
      repo: "agent-notify",
      state: "waiting-input",
      dismiss: false,
      summary: "waiting for input",
      priority: 90,
      ttlSeconds: 30,
      runId: "abc123",
    });

    assert.deepEqual(buildEventInput(parsed.options!, "workstation"), {
      agent: "claude",
      host: "workstation",
      repo: "agent-notify",
      state: "waiting_input",
      summary: "waiting for input",
      priority: 90,
      ttl_seconds: 30,
      run_id: "abc123",
    });
  });

  it("uses environment fallbacks and defaults the agent to codex", () => {
    const parsed = parseCliArgs(["--state=done"], {
      AGENT_NOTIFY_SERVER: "http://server",
      AGENT_NOTIFY_TOKEN: "secret",
      AGENT_NOTIFY_HOST: "env-host",
      AGENT_NOTIFY_REPO: "env-repo",
    });

    assert.equal(parsed.exit, undefined);
    assert.equal(parsed.options?.server, "http://server");
    assert.equal(parsed.options?.token, "secret");
    assert.equal(parsed.options?.agent, "codex");
    assert.equal(parsed.options?.host, "env-host");
    assert.equal(parsed.options?.repo, "env-repo");
  });

  it("accepts dismiss without state", () => {
    const parsed = parseCliArgs(["--dismiss"], {
      AGENT_NOTIFY_SERVER: "http://server",
      AGENT_NOTIFY_TOKEN: "secret",
    });

    assert.equal(parsed.exit, undefined);
    assert.equal(parsed.options?.dismiss, true);
    assert.equal(parsed.options?.state, undefined);
  });

  it("rejects unknown states", () => {
    const parsed = parseCliArgs(["--server", "http://server", "--token", "secret", "--state", "waiting_input"]);

    assert.equal(parsed.exit?.code, 2);
    assert.match(parsed.exit?.message ?? "", /--state must be one of/u);
  });
});
