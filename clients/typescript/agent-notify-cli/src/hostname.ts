import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

export interface HostnameDeps {
  env?: Record<string, string | undefined>;
  platform?: NodeJS.Platform;
  readFile?: (path: string) => string | undefined;
  runHostname?: () => string | undefined;
}

export function localHostname(deps: HostnameDeps = {}): string | undefined {
  const env = deps.env ?? process.env;
  const platform = deps.platform ?? process.platform;
  const readFile = deps.readFile ?? readHostnameFile;
  const runHostname = deps.runHostname ?? runHostnameCommand;

  for (const candidate of [env.COMPUTERNAME, env.HOSTNAME]) {
    const normalized = normalizeField(candidate);
    if (normalized) {
      return normalized;
    }
  }

  if (platform === "linux") {
    for (const path of ["/proc/sys/kernel/hostname", "/etc/hostname"]) {
      const normalized = normalizeField(readFile(path));
      if (normalized) {
        return normalized;
      }
    }
  }

  return normalizeField(runHostname());
}

function readHostnameFile(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

function runHostnameCommand(): string | undefined {
  // Prefer absolute paths so a hostile PATH cannot shim a fake `hostname`;
  // fall back to a PATH lookup only if none of them resolve.
  const candidates =
    process.platform === "win32" ? ["hostname"] : ["/bin/hostname", "/usr/bin/hostname", "hostname"];

  for (const program of candidates) {
    try {
      return execFileSync(program, { encoding: "utf8" });
    } catch {
      // try the next candidate
    }
  }

  return undefined;
}

function normalizeField(value: string | undefined): string | undefined {
  const normalized = value?.split(/\s+/u).filter(Boolean).join(" ");
  return normalized && normalized.length > 0 ? normalized : undefined;
}
