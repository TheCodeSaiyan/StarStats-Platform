// Executable contract tests for the `images` job build gate in
// .github/workflows/release-images.yml.
//
// Like its sibling suite, this does NOT reimplement the condition: it lifts
// the `if:` expression straight out of the YAML and evaluates it against
// synthetic GitHub event contexts. A copy would drift from the workflow and
// quietly stop testing anything real.
//
// Why it exists: `:latest` is what the host pulls, and it moves only on a
// branch push to `main`. The gate used to skip a main push whose HEAD commit
// was metadata (`release-manifests:` / `docs:`) -- but a push carries many
// commits, and a live promote fast-forwards main across everything on next.
// On 2026-08-27 that skipped the promote's own push to main -- its head was a
// `docs:` commit, because a promote fast-forwards main onto whatever happens
// to be the tip of next. v0.1.11 was tagged, `:latest` stayed on the previous
// image, healthz kept reporting 0.1.10, and it looked exactly like a
// successful release.
//
// The manifest bot is NOT the trigger: its `release-manifests:` commit is
// pushed with the default GITHUB_TOKEN, which does not start workflows, so it
// can never cause a skip.
//
// Run: node --test scripts/release-images-build-gate.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKFLOW = join(HERE, "..", ".github", "workflows", "release-images.yml");

/** Lift the images job `if:` block scalar out of the workflow YAML.
 *
 *  Anchored on the job key rather than line numbers so edits elsewhere in
 *  the file cannot silently shift the extraction onto a different job
 *  condition -- which would leave this suite green while testing nothing.
 */
function extractBuildGate() {
  const src = readFileSync(WORKFLOW, "utf8");
  const jobAt = src.indexOf("\n  images:\n");
  assert.notEqual(jobAt, -1, "images job not found in the workflow");
  const rest = src.slice(jobAt);
  const marker = "\n    if: |\n";
  const ifAt = rest.indexOf(marker);
  assert.notEqual(ifAt, -1, "images job has no `if:` block");
  const body = [];
  for (const line of rest.slice(ifAt + marker.length).split("\n")) {
    if (line.trim() === "") break;
    if (!line.startsWith("      ")) break;
    body.push(line.slice(6));
  }
  const expr = body.join("\n").trim();
  assert.ok(expr.length > 0, "extracted an empty condition");
  return expr;
}

/** Evaluate a GitHub Actions expression against a context object.
 *
 *  Supports only what this condition uses: boolean/comparison operators,
 *  always(), startsWith(), contains(), and github.* / needs.* lookups.
 *  Context paths become safe lookups so a missing property yields undefined
 *  rather than throwing, matching Actions null semantics.
 */
function evaluate(expr, ctx) {
  const get = (path) =>
    path.split(".").reduce((o, k) => (o == null ? undefined : o[k]), ctx);
  const startsWith = (a, b) => String(a ?? "").startsWith(String(b ?? ""));
  const contains = (a, b) => String(a ?? "").includes(String(b ?? ""));
  const always = () => true;
  const js = expr.replace(
    /\b(github|needs)((?:\.[A-Za-z_][A-Za-z0-9_-]*)+)/g,
    (m) => `get(${JSON.stringify(m)})`,
  );
  return Function(
    "get",
    "startsWith",
    "contains",
    "always",
    `"use strict"; return (${js});`,
  )(get, startsWith, contains, always);
}

const GATE = extractBuildGate();

function push(ref, message, validateTag = "skipped") {
  return {
    github: {
      ref,
      ref_name: ref.replace(/^refs\/(heads|tags)\//, ""),
      event_name: "push",
      event: { head_commit: { message } },
    },
    needs: { "validate-tag": { result: validateTag } },
  };
}

function tagPush(tagName, validateTag = "success") {
  return push(
    `refs/tags/${tagName}`,
    `chore: bump platform to ${tagName}`,
    validateTag,
  );
}

// The evaluator must be trustworthy or every assertion below is worthless:
// one that returned truthy for everything would pass the whole suite.
test("the expression evaluator is not vacuously true", () => {
  const ctx = push("refs/heads/main", "docs: x");
  assert.equal(evaluate("true && true", ctx), true);
  assert.equal(evaluate("true && false", ctx), false);
  assert.equal(evaluate("github.ref == 'refs/heads/main'", ctx), true);
  assert.equal(evaluate("github.ref == 'refs/heads/next'", ctx), false);
  assert.equal(
    evaluate("startsWith(github.event.head_commit.message, 'docs:')", ctx),
    true,
  );
  assert.equal(
    evaluate("startsWith(github.event.head_commit.message, 'feat:')", ctx),
    false,
  );
  assert.equal(evaluate("!(true)", ctx), false);
  assert.equal(evaluate("startsWith(github.event.nope.message, 'x')", ctx), false);
});

// main ALWAYS builds. `:latest` moves only here, so a skip is a silent
// non-deploy.
test("a main push builds no matter what the head commit says", () => {
  const heads = [
    "release-manifests: tray-live -> tray-v0.1.9",
    "docs: the recreate command fails quietly if you get it wrong",
    "docs(releasing): tweak",
    "chore: bump platform to v0.1.11",
    "chore: bump tray to tray-v0.1.9",
    "chore: bump to v0.1.11",
    "feat: something real",
  ];
  for (const message of heads) {
    assert.equal(
      evaluate(GATE, push("refs/heads/main", message)),
      true,
      `main push with head "${message}" must build -- :latest moves only on a main push, so skipping it silently fails to deploy`,
    );
  }
});

// next keeps its skip list: it fires constantly and the parent commit already
// built the same image content.
test("next still skips the noise it was meant to skip", () => {
  for (const message of [
    "chore: bump to v0.1.12-alpha.1",
    "chore: bump platform to v0.1.12-alpha.1",
    "chore: bump tray to tray-v0.1.10-alpha.1",
    "release-manifests: tray-alpha -> tray-v0.1.10-alpha.1",
    "docs: runbook",
    "docs(ci): note",
  ]) {
    assert.equal(
      evaluate(GATE, push("refs/heads/next", message)),
      false,
      `next push with head "${message}" should skip`,
    );
  }
});

test("next builds for real changes", () => {
  for (const message of ["feat: add a thing", "fix(server): correct a thing"]) {
    assert.equal(evaluate(GATE, push("refs/heads/next", message)), true, message);
  }
});

test("a live version tag builds and a pre-release tag does not", () => {
  assert.equal(evaluate(GATE, tagPush("v0.1.11")), true, "live tag must build");
  for (const t of ["v0.1.12-alpha.1", "v0.1.12-beta.2", "v0.1.12-rc.1"]) {
    assert.equal(evaluate(GATE, tagPush(t)), false, `${t} must not build`);
  }
});

test("a failed tag validation blocks the build", () => {
  assert.equal(
    evaluate(GATE, tagPush("v0.1.11", "failure")),
    false,
    "validate-tag failing must stop the build -- it is the only guard keeping pre-release tags off main",
  );
});

test("workflow_dispatch always builds", () => {
  const ctx = {
    github: {
      ref: "refs/heads/main",
      ref_name: "main",
      event_name: "workflow_dispatch",
      event: {},
    },
    needs: { "validate-tag": { result: "skipped" } },
  };
  assert.equal(evaluate(GATE, ctx), true);
});
