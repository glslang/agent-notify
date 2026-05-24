import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { localHostname } from "../src/hostname.js";

describe("localHostname", () => {
  it("prefers COMPUTERNAME and normalizes whitespace", () => {
    assert.equal(
      localHostname({
        env: {
          COMPUTERNAME: " workstation\none ",
          HOSTNAME: "ignored",
        },
        platform: "linux",
        readFile: () => "ignored",
        runHostname: () => "ignored",
      }),
      "workstation one",
    );
  });

  it("falls back to HOSTNAME", () => {
    assert.equal(
      localHostname({
        env: {
          HOSTNAME: "env-host",
        },
        platform: "darwin",
        runHostname: () => "ignored",
      }),
      "env-host",
    );
  });

  it("reads Linux hostname files in order", () => {
    const reads: string[] = [];
    const host = localHostname({
      env: {},
      platform: "linux",
      readFile: (path) => {
        reads.push(path);
        return path === "/etc/hostname" ? "file-host\n" : undefined;
      },
      runHostname: () => "ignored",
    });

    assert.equal(host, "file-host");
    assert.deepEqual(reads, ["/proc/sys/kernel/hostname", "/etc/hostname"]);
  });

  it("uses hostname command after environment and files fail", () => {
    assert.equal(
      localHostname({
        env: {},
        platform: "linux",
        readFile: () => undefined,
        runHostname: () => "command-host\n",
      }),
      "command-host",
    );
  });

  it("returns undefined when no hostname source succeeds", () => {
    assert.equal(
      localHostname({
        env: {},
        platform: "linux",
        readFile: () => undefined,
        runHostname: () => undefined,
      }),
      undefined,
    );
  });
});
