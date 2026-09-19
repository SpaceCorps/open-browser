import { execFileSync } from "node:child_process";
import path from "node:path";
import { describe, expect, it } from "vitest";
import type { ActionSpec } from "../src/api/types";
import { cliCommand, positionalCount } from "../src/lib/command";

/** The registry, from the binary rather than from a fixture.
 *
 * This is the point of the file. `cliCommand` reimplements `ob`'s argument placement in TypeScript
 * so the UI can show the equivalent command, and a reimplementation that drifts is worse than no
 * preview at all — it would claim an agent could run something it cannot. Reading the live registry
 * means adding an action to the Rust table either keeps these passing or fails them here. */
function registry(): ActionSpec[] {
  const root = path.resolve(__dirname, "../../..");
  const json = execFileSync(
    "cargo",
    ["run", "--quiet", "-p", "open-browser-cli", "--bin", "ob", "--", "--json", "actions", "list"],
    {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  return JSON.parse(json) as ActionSpec[];
}

const SPECS = registry();

describe("the command preview", () => {
  it("covers every action in the registry", () => {
    expect(SPECS.length).toBeGreaterThan(0);
    for (const spec of SPECS) {
      const command = cliCommand(spec, {}, "work");
      expect(command.startsWith("ob --session work ")).toBe(true);
      expect(command).toContain(spec.id);
    }
  });

  it("places required parameters positionally, the way `ob` accepts them", () => {
    const click = SPECS.find((spec) => spec.id === "click");
    expect(click).toBeDefined();
    expect(cliCommand(click!, { selector: "button.go" })).toBe("ob click button.go");
    expect(cliCommand(click!, { selector: "button.go", index: "2" })).toBe(
      "ob click button.go --index=2",
    );
  });

  it("quotes anything a shell would mangle", () => {
    const type = SPECS.find((spec) => spec.id === "type")!;
    expect(cliCommand(type, { selector: "#q", text: "two words" })).toBe(
      "ob type '#q' 'two words'",
    );
    // An embedded single quote must survive: the POSIX form is '\'' , not a backslash escape.
    expect(cliCommand(type, { selector: "#q", text: "it's" })).toBe(`ob type '#q' 'it'\\''s'`);
  });

  it("omits an absent flag rather than sending it false", () => {
    const type = SPECS.find((spec) => spec.id === "type")!;
    expect(cliCommand(type, { selector: "#q", text: "hi", clear: "" })).not.toContain("--clear");
    expect(cliCommand(type, { selector: "#q", text: "hi", clear: "true" })).toContain("--clear");
  });

  it("repeats a repeatable parameter instead of joining it", () => {
    const upload = SPECS.find((spec) => spec.id === "upload")!;
    const command = cliCommand(upload, { selector: "input[type=file]", path: "a.pdf\nb.pdf" });
    expect(command).toContain("--path=a.pdf");
    expect(command).toContain("--path=b.pdf");
  });

  it("stops taking positionals at the first repeatable required parameter", () => {
    // Mirrors `positional_count` in crates/open-browser-cli/src/actions.rs; `upload` is the case
    // that rule exists for.
    const upload = SPECS.find((spec) => spec.id === "upload")!;
    expect(positionalCount(upload)).toBe(1);
  });
});
