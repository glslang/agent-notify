#!/usr/bin/env node
import { fileURLToPath } from "node:url";
import { dismissLatest, type AgentEventInput, postEvent, type WireAgentState } from "./client.js";
import { localHostname } from "./hostname.js";

type StateArg = "running" | "waiting-input" | "done" | "failed";

export interface CliOptions {
  server: string;
  token: string;
  agent: string;
  host?: string;
  repo?: string;
  state?: StateArg;
  dismiss: boolean;
  summary?: string;
  priority?: number;
  ttlSeconds?: number;
  runId?: string;
}

export interface ParseResult {
  options?: CliOptions;
  exit?: {
    code: number;
    message: string;
  };
}

const VALUE_FLAGS = new Set([
  "server",
  "token",
  "agent",
  "host",
  "repo",
  "state",
  "summary",
  "priority",
  "ttl-seconds",
  "run-id",
]);

const BOOLEAN_FLAGS = new Set(["dismiss", "help", "version"]);

export function parseCliArgs(argv: readonly string[], env: Record<string, string | undefined> = process.env): ParseResult {
  const values = new Map<string, string>();
  let dismiss = false;

  for (let index = 0; index < argv.length; index += 1) {
    const raw = argv[index];
    if (!raw.startsWith("--")) {
      return usageError(`unexpected argument: ${raw}`);
    }

    const [flag, inlineValue] = splitFlag(raw);
    if (BOOLEAN_FLAGS.has(flag)) {
      if (inlineValue !== undefined) {
        return usageError(`--${flag} does not accept a value`);
      }
      if (flag === "help") {
        return { exit: { code: 0, message: usage() } };
      }
      if (flag === "version") {
        return { exit: { code: 0, message: "agent-notify-cli 0.1.0" } };
      }
      dismiss = true;
      continue;
    }

    if (!VALUE_FLAGS.has(flag)) {
      return usageError(`unknown flag: --${flag}`);
    }

    const value = inlineValue ?? argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      return usageError(`--${flag} requires a value`);
    }
    if (inlineValue === undefined) {
      index += 1;
    }
    values.set(flag, value);
  }

  const server = values.get("server") ?? env.AGENT_NOTIFY_SERVER;
  if (!server) {
    return usageError("--server or AGENT_NOTIFY_SERVER is required");
  }

  const token = values.get("token") ?? env.AGENT_NOTIFY_TOKEN;
  if (!token) {
    return usageError("--token or AGENT_NOTIFY_TOKEN is required");
  }

  const state = parseState(values.get("state"));
  if (state instanceof Error) {
    return usageError(state.message);
  }
  if (!dismiss && state === undefined) {
    return usageError("--state is required unless --dismiss is supplied");
  }

  const priority = parseIntegerFlag("priority", values.get("priority"), 0, 255);
  if (priority instanceof Error) {
    return usageError(priority.message);
  }

  const ttlSeconds = parseIntegerFlag("ttl-seconds", values.get("ttl-seconds"), 0, Number.MAX_SAFE_INTEGER);
  if (ttlSeconds instanceof Error) {
    return usageError(ttlSeconds.message);
  }

  return {
    options: {
      server,
      token,
      agent: values.get("agent") ?? env.AGENT_NOTIFY_AGENT ?? "codex",
      host: values.get("host") ?? env.AGENT_NOTIFY_HOST,
      repo: values.get("repo") ?? env.AGENT_NOTIFY_REPO,
      state,
      dismiss,
      summary: values.get("summary"),
      priority,
      ttlSeconds,
      runId: values.get("run-id"),
    },
  };
}

export function buildEventInput(options: CliOptions, host: string): AgentEventInput {
  if (!options.state) {
    throw new Error("--state is required unless --dismiss is supplied");
  }

  return {
    agent: options.agent,
    host,
    repo: options.repo,
    state: toWireState(options.state),
    summary: options.summary,
    priority: options.priority,
    ttl_seconds: options.ttlSeconds,
    run_id: options.runId,
  };
}

export async function runCli(argv: readonly string[] = process.argv.slice(2), env = process.env): Promise<number> {
  const parsed = parseCliArgs(argv, env);
  if (parsed.exit) {
    writeMessage(parsed.exit.code, parsed.exit.message);
    return parsed.exit.code;
  }

  const options = parsed.options;
  if (!options) {
    process.stderr.write("failed to parse arguments\n");
    return 1;
  }

  try {
    if (options.dismiss) {
      await dismissLatest(options.server, options.token);
      return 0;
    }

    const host = options.host ?? localHostname();
    if (!host) {
      throw new Error("host was not supplied and hostname could not be inferred from environment or system");
    }

    await postEvent(options.server, options.token, buildEventInput(options, host));
    return 0;
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    return 1;
  }
}

function splitFlag(raw: string): [string, string | undefined] {
  const trimmed = raw.slice(2);
  const equalsIndex = trimmed.indexOf("=");
  if (equalsIndex === -1) {
    return [trimmed, undefined];
  }
  return [trimmed.slice(0, equalsIndex), trimmed.slice(equalsIndex + 1)];
}

function parseState(value: string | undefined): StateArg | undefined | Error {
  if (value === undefined) {
    return undefined;
  }

  if (value === "running" || value === "waiting-input" || value === "done" || value === "failed") {
    return value;
  }

  return new Error("--state must be one of: running, waiting-input, done, failed");
}

function toWireState(value: StateArg): WireAgentState {
  return value === "waiting-input" ? "waiting_input" : value;
}

function parseIntegerFlag(flag: string, value: string | undefined, min: number, max: number): number | undefined | Error {
  if (value === undefined) {
    return undefined;
  }

  if (!/^\d+$/u.test(value)) {
    return new Error(`--${flag} must be an integer`);
  }

  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
    return new Error(`--${flag} must be between ${min} and ${max}`);
  }

  return parsed;
}

function usageError(message: string): ParseResult {
  return { exit: { code: 2, message: `${message}\n\n${usage()}` } };
}

function usage(): string {
  return `Usage: agent-notify-cli --server URL --token TOKEN [--agent NAME] [--host HOST] [--repo REPO] --state running|waiting-input|done|failed [options]
       agent-notify-cli --server URL --token TOKEN --dismiss

Options:
  --server URL            Event server URL, or AGENT_NOTIFY_SERVER
  --token TOKEN           Bearer token, or AGENT_NOTIFY_TOKEN
  --agent NAME            Agent name, or AGENT_NOTIFY_AGENT (default: codex)
  --host HOST             Host name, or AGENT_NOTIFY_HOST
  --repo REPO             Repository name, or AGENT_NOTIFY_REPO
  --state STATE           running, waiting-input, done, or failed
  --summary TEXT          Short status summary
  --priority NUMBER       Priority from 0 to 255
  --ttl-seconds NUMBER    Notification TTL in seconds
  --run-id ID             Stable work item identifier
  --dismiss               Dismiss the current notification
`;
}

function writeMessage(code: number, message: string): void {
  const stream = code === 0 ? process.stdout : process.stderr;
  stream.write(`${message.trimEnd()}\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = await runCli();
}
