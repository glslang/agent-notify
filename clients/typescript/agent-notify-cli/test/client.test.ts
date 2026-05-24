import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { dismissLatest, postEvent } from "../src/client.js";

describe("client", () => {
  it("posts events to /v1/events with bearer auth", async () => {
    const requests: Array<{ input: RequestInfo | URL; init?: RequestInit }> = [];
    const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
      requests.push({ input, init });
      return new Response("{}", { status: 200 });
    };

    await postEvent(
      "http://server/",
      "secret",
      {
        agent: "codex",
        host: "workstation",
        repo: "agent-notify",
        state: "done",
        summary: "complete",
      },
      fetchImpl,
    );

    assert.equal(requests.length, 1);
    assert.equal(requests[0].input, "http://server/v1/events");
    assert.equal(requests[0].init?.method, "POST");
    assert.deepEqual(requests[0].init?.headers, {
      authorization: "Bearer secret",
      "content-type": "application/json",
    });
    assert.equal(
      requests[0].init?.body,
      JSON.stringify({
        agent: "codex",
        host: "workstation",
        repo: "agent-notify",
        state: "done",
        summary: "complete",
      }),
    );
  });

  it("deletes /v1/events/latest for dismiss", async () => {
    const requests: Array<{ input: RequestInfo | URL; init?: RequestInit }> = [];
    const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
      requests.push({ input, init });
      return new Response("{}", { status: 200 });
    };

    await dismissLatest("http://server///", "secret", fetchImpl);

    assert.equal(requests.length, 1);
    assert.equal(requests[0].input, "http://server/v1/events/latest");
    assert.equal(requests[0].init?.method, "DELETE");
    assert.deepEqual(requests[0].init?.headers, {
      authorization: "Bearer secret",
    });
  });

  it("surfaces non-success responses", async () => {
    const fetchImpl = async () => new Response("bad token", { status: 401 });

    await assert.rejects(
      postEvent(
        "http://server",
        "secret",
        {
          agent: "codex",
          host: "workstation",
          state: "failed",
        },
        fetchImpl,
      ),
      /server returned 401: bad token/u,
    );
  });
});
