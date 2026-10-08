import { afterEach, expect, it, mock, spyOn } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import plugin from "./index";
import { version } from "./package.json";

const originalFetch = globalThis.fetch;
const originalPath = process.env.PATH;
let directory: string | undefined;

afterEach(async () => {
  globalThis.fetch = originalFetch;
  process.env.PATH = originalPath;
  mock.restore();
  if (directory && existsSync(join(directory, "pid"))) {
    try { process.kill(Number(await readFile(join(directory, "pid"), "utf8")), "SIGKILL"); } catch {}
  }
  if (directory) await rm(directory, { recursive: true, force: true });
  directory = undefined;
});

it("loads through V2 setup without a Bun shell and starts the proxy", async () => {
  // A V1 function export is rejected by V2 before any startup logic runs.
  expect(plugin).toBeObject();
  expect(plugin).toHaveProperty("id");
  expect(plugin).toHaveProperty("setup", expect.any(Function));

  directory = await mkdtemp(join(tmpdir(), "shift-v2-"));
  const calls = join(directory, "calls");
  await writeFile(join(directory, "shift-ai"),
    `#!/bin/sh\nprintf '%s\\n' "$*" >> '${calls}'\n`, { mode: 0o755 });
  // Minimal installations can have shift-ai but no external `which` command.
  process.env.PATH = directory;
  let probes = 0;
  globalThis.fetch = mock(async () => {
    if (++probes === 1) throw new Error("ECONNREFUSED");
    return Response.json({ service: "@shift-preflight/runtime proxy", version });
  }) as typeof fetch;
  spyOn(console, "log").mockImplementation(() => {});

  // Context deliberately has no V1 `$` helper.
  await plugin.setup();
  expect(existsSync(calls)).toBe(true);
  expect(await readFile(calls, "utf8")).toContain("proxy ensure --quiet\n");
  expect(probes).toBe(2);
});

it("skips V2 startup when shift-ai is absent", async () => {
  directory = await mkdtemp(join(tmpdir(), "shift-v2-missing-"));
  process.env.PATH = directory;
  const fetchMock = mock(async () => { throw new Error("must not probe"); });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  await plugin.setup();
  expect(fetchMock).not.toHaveBeenCalled();
});

it("terminates a hung prerequisite instead of blocking V2 initialization", async () => {
  directory = await mkdtemp(join(tmpdir(), "shift-v2-hung-"));
  await writeFile(join(directory, "shift-ai"),
    `#!/bin/sh\nprintf '%s' "$$" > '${join(directory, "pid")}'\nexec /bin/sleep 30\n`,
    { mode: 0o755 });
  process.env.PATH = directory;
  const fetchMock = mock(async () => { throw new Error("must not probe"); });
  globalThis.fetch = fetchMock as unknown as typeof fetch;
  let timer: ReturnType<typeof setTimeout>;
  try {
    const outcome = await Promise.race([
      plugin.setup().then(() => "completed"),
      new Promise<string>((resolve) => { timer = setTimeout(() => resolve("blocked"), 3_500); }),
    ]);
    expect(outcome).toBe("completed");
    const pid = Number(await readFile(join(directory, "pid"), "utf8"));
    expect(() => process.kill(pid, 0)).toThrow();
    expect(fetchMock).not.toHaveBeenCalled();
  } finally {
    clearTimeout(timer!);
  }
}, 5_000);

it("keeps a healthy proxy when the installed CLI cannot upgrade it", async () => {
  directory = await mkdtemp(join(tmpdir(), "shift-v2-old-cli-"));
  const calls = join(directory, "calls");
  await writeFile(join(directory, "shift-ai"),
    `#!/bin/sh\nprintf '%s\\n' "$*" >> '${calls}'\nprintf 'shift-ai 0.0.1\\n'\n`,
    { mode: 0o755 });
  process.env.PATH = directory;
  globalThis.fetch = mock(async () => Response.json({
    service: "@shift-preflight/runtime proxy", version: "0.0.1",
  })) as typeof fetch;
  spyOn(console, "log").mockImplementation(() => {});
  const warning = spyOn(console, "warn").mockImplementation(() => {});
  await plugin.setup();
  await plugin.setup();
  expect(await readFile(calls, "utf8")).toBe("--version\n--version\n");
  expect(warning).toHaveBeenCalledWith(expect.stringContaining("upgrade shift-ai"));
});
