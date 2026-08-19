// Executable contract tests for the tag validator and immutable-image
// preflight steps in .github/workflows/release-images.yml.
//
// These do NOT reimplement the workflow logic. They extract each step's
// `run:` block straight out of the YAML and execute it against synthetic
// git graphs or registry probes that we control. A copy would drift from
// the workflow and quietly stop testing anything real — the whole
// failure mode this suite exists to catch.
//
// Run: node --test scripts/release-images-validate-tag.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORKFLOW = join(HERE, "..", ".github", "workflows", "release-images.yml");
const WINDOWS_GIT_BASH = "C:\\Program Files\\Git\\bin\\bash.exe";
const BASH = process.platform === "win32" && existsSync(WINDOWS_GIT_BASH)
  ? WINDOWS_GIT_BASH
  : "bash";

function bashPath(path) {
  if (process.platform !== "win32") return path;
  return path
    .replace(/^([A-Za-z]):\\/, (_, drive) => `/${drive.toLowerCase()}/`)
    .replaceAll("\\", "/");
}

function removeTemp(dir) {
  rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
}

/** Pull the validate step's shell body out of the workflow YAML.
 *
 *  Anchored on the step's own markers rather than line numbers so
 *  edits above it don't silently shift the extraction onto the wrong
 *  block — an extraction that quietly grabs the wrong lines would make
 *  every assertion below meaningless. */
function extractValidateScript() {
  const lines = readFileSync(WORKFLOW, "utf8").split(/\r?\n/);
  const start = lines.findIndex((l) => l.trim() === "set -euo pipefail" );
  assert.ok(start > 0, "could not locate the validate step's shell body");
  const endMarker = 'echo "resolved channel=$channel for tag=$TAG"';
  const end = lines.findIndex((l, i) => i > start && l.trim() === endMarker);
  assert.ok(end > start, "could not locate the end of the validate step");
  const body = lines.slice(start, end + 1);
  const indent = body[0].match(/^\s*/)[0];
  const script = body.map((l) => (l.startsWith(indent) ? l.slice(indent.length) : l)).join("\n");
  // Guard against extracting a block that no longer contains the
  // branch under test.
  assert.match(script, /pre-release tag/, "extracted block is not the tag validator");
  return script;
}

const VALIDATE = extractValidateScript();

function extractStepScript(stepName, nextStepName) {
  const lines = readFileSync(WORKFLOW, "utf8").split(/\r?\n/);
  const marker = `- name: ${stepName}`;
  const step = lines.findIndex((line) => line.trim() === marker);
  assert.ok(step >= 0, `could not locate workflow step: ${stepName}`);
  const run = lines.findIndex((line, index) => index > step && line.trim() === "run: |");
  assert.ok(run > step, `could not locate run block for: ${stepName}`);
  const end = lines.findIndex(
    (line, index) => index > run && line.trim() === `- name: ${nextStepName}`,
  );
  assert.ok(end > run, `could not locate next workflow step: ${nextStepName}`);
  const body = lines.slice(run + 1, end);
  const nonBlank = body.find((line) => line.trim() !== "");
  assert.ok(nonBlank, `empty run block for: ${stepName}`);
  const indent = nonBlank.match(/^\s*/)[0];
  return body
    .map((line) => (line.startsWith(indent) ? line.slice(indent.length) : line))
    .join("\n")
    .trimEnd();
}

const IMAGE_PREFLIGHT = extractStepScript(
  "Preflight — do the immutable SHA images already exist?",
  "Build & push API",
);
const CONFIG_IMAGE_PREFLIGHT = extractStepScript(
  "Preflight — does the immutable SHA image already exist?",
  "Build & push ${{ matrix.image }}",
);

function runImagePreflight({ ref, apiAfter, webAfter, waitAttempts = 3 }) {
  const dir = mkdtempSync(join(tmpdir(), "ss-image-preflight-"));
  try {
    const script = join(dir, "preflight.sh");
    const output = join(dir, "gh_output");
    const attemptsOutput = join(dir, "attempts");
    writeFileSync(script, `api_attempts=0
web_attempts=0
docker() {
  local ref="\${4:-}"
  case "$ref" in
    */api:*) api_attempts=$((api_attempts + 1)); (( api_attempts >= API_AFTER )) ;;
    */web:*) web_attempts=$((web_attempts + 1)); (( web_attempts >= WEB_AFTER )) ;;
    *) return 2 ;;
  esac
}
sleep() { :; }
${IMAGE_PREFLIGHT}
printf 'api=%s\nweb=%s\n' "$api_attempts" "$web_attempts" > "$ATTEMPTS_OUTPUT"
`);
    writeFileSync(output, "");
    writeFileSync(attemptsOutput, "");
    const stdout = execFileSync(BASH, ["-e", bashPath(script)], {
      cwd: dir,
      encoding: "utf8",
      env: {
        ...process.env,
        API_AFTER: String(apiAfter),
        WEB_AFTER: String(webAfter),
        REGISTRY: "registry.test",
        GITHUB_REF: ref,
        GITHUB_SHA: "abc123",
        GITHUB_OUTPUT: bashPath(output),
        ATTEMPTS_OUTPUT: bashPath(attemptsOutput),
        SIBLING_WAIT_ATTEMPTS: String(waitAttempts),
        SIBLING_WAIT_SECONDS: "0",
      },
    });
    const attemptValues = Object.fromEntries(
      readFileSync(attemptsOutput, "utf8").trim().split(/\r?\n/).map((line) => line.split("=")),
    );
    const attempts = (image) => {
      return Number(attemptValues[image]);
    };
    return { stdout, output: readFileSync(output, "utf8"), attempts };
  } finally {
    removeTemp(dir);
  }
}

test("live tag waits for sibling main manifests and retags them", () => {
  const result = runImagePreflight({
    ref: "refs/tags/v1.8.159",
    apiAfter: 2,
    webAfter: 3,
  });

  assert.match(result.output, /^api=retag$/m);
  assert.match(result.output, /^web=retag$/m);
  assert.equal(result.attempts("api"), 2);
  assert.equal(result.attempts("web"), 3);
  assert.match(result.stdout, /waiting for sibling main run/i);
});

test("main branch preflight never waits for a sibling", () => {
  const result = runImagePreflight({
    ref: "refs/heads/main",
    apiAfter: 99,
    webAfter: 99,
  });

  assert.match(result.output, /^api=build$/m);
  assert.match(result.output, /^web=build$/m);
  assert.equal(result.attempts("api"), 1);
  assert.equal(result.attempts("web"), 1);
  assert.doesNotMatch(result.stdout, /waiting for sibling main run/i);
});

test("live tag falls back to builds after the bounded sibling wait", () => {
  const result = runImagePreflight({
    ref: "refs/tags/v1.8.159",
    apiAfter: 99,
    webAfter: 99,
    waitAttempts: 2,
  });

  assert.match(result.output, /^api=build$/m);
  assert.match(result.output, /^web=build$/m);
  assert.equal(result.attempts("api"), 3);
  assert.equal(result.attempts("web"), 3);
});

test("prerelease tags remain excluded from image builds", () => {
  const workflow = readFileSync(WORKFLOW, "utf8");
  assert.match(workflow, /!contains\(github\.ref_name, '-alpha'\)/);
  assert.match(workflow, /!contains\(github\.ref_name, '-beta'\)/);
  assert.match(workflow, /!contains\(github\.ref_name, '-rc'\)/);
});

function runConfigImagePreflight({ ref, availableAfter, waitAttempts = 2 }) {
  const dir = mkdtempSync(join(tmpdir(), "ss-config-preflight-"));
  try {
    const script = join(dir, "preflight.sh");
    const output = join(dir, "gh_output");
    const attemptsOutput = join(dir, "attempts");
    writeFileSync(script, `probe_count=0
docker() {
  probe_count=$((probe_count + 1))
  (( probe_count >= AVAILABLE_AFTER ))
}
sleep() { :; }
${CONFIG_IMAGE_PREFLIGHT}
printf '%s\n' "$probe_count" > "$ATTEMPTS_OUTPUT"
`);
    writeFileSync(output, "");
    writeFileSync(attemptsOutput, "");
    const stdout = execFileSync(BASH, ["-e", bashPath(script)], {
      cwd: dir,
      encoding: "utf8",
      env: {
        ...process.env,
        AVAILABLE_AFTER: String(availableAfter),
        IMAGE: "tempo",
        REGISTRY: "registry.test",
        GITHUB_REF: ref,
        GITHUB_SHA: "abc123",
        GITHUB_OUTPUT: bashPath(output),
        ATTEMPTS_OUTPUT: bashPath(attemptsOutput),
        SIBLING_WAIT_ATTEMPTS: String(waitAttempts),
        SIBLING_WAIT_SECONDS: "0",
      },
    });
    return {
      stdout,
      output: readFileSync(output, "utf8"),
      attempts: Number(readFileSync(attemptsOutput, "utf8").trim()),
    };
  } finally {
    removeTemp(dir);
  }
}

test("live tag waits for sibling main config manifest and retags it", () => {
  const result = runConfigImagePreflight({
    ref: "refs/tags/v1.8.159",
    availableAfter: 2,
  });

  assert.match(result.output, /^img=retag$/m);
  assert.equal(result.attempts, 2);
  assert.match(result.stdout, /waiting for sibling main run/i);
});

test("main config-image preflight never waits for a sibling", () => {
  const result = runConfigImagePreflight({
    ref: "refs/heads/main",
    availableAfter: 99,
  });

  assert.match(result.output, /^img=build$/m);
  assert.equal(result.attempts, 1);
});

test("live tag config image falls back after its bounded wait", () => {
  const result = runConfigImagePreflight({
    ref: "refs/tags/v1.8.159",
    availableAfter: 99,
    waitAttempts: 2,
  });

  assert.match(result.output, /^img=build$/m);
  assert.equal(result.attempts, 3);
});

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
}

/** Build a throwaway repo and run the real validate script against a tag.
 *  Returns { ok, stdout, stderr, channel }. */
function runValidate(build, tag) {
  const dir = mkdtempSync(join(tmpdir(), "ss-validate-"));
  try {
    git(dir, "init", "-q", "-b", "main");
    git(dir, "config", "user.email", "t@t");
    git(dir, "config", "user.name", "t");
    const commit = (msg) => {
      git(dir, "commit", "-q", "--allow-empty", "-m", msg);
      return git(dir, "rev-parse", "HEAD");
    };
    // Commit carrying a Cargo.toml `version`. The validator reads that
    // file at the tag's commit to tell a promoted commit (bare version)
    // from an un-promoted pre-release one (`X.Y.Z-alpha.N`), so a test
    // exercising that path has to produce a real one.
    const commitVersion = (msg, version) => {
      writeFileSync(join(dir, "Cargo.toml"), `[package]\nname = "x"\nversion = "${version}"\n`);
      git(dir, "add", "Cargo.toml");
      git(dir, "commit", "-q", "-m", msg);
      return git(dir, "rev-parse", "HEAD");
    };
    build({ dir, git: (...a) => git(dir, ...a), commit, commitVersion });

    // The script requires origin/main and origin/next to exist; its own
    // `git fetch` is a no-op here (no remote) and is guarded by `|| true`.
    git(dir, "update-ref", "refs/remotes/origin/main", "refs/heads/main");
    git(dir, "update-ref", "refs/remotes/origin/next", "refs/heads/next");

    const scriptPath = join(dir, "validate.sh");
    writeFileSync(scriptPath, VALIDATE);
    const outPath = join(dir, "gh_output");
    writeFileSync(outPath, "");

    try {
      const stdout = execFileSync(BASH, [bashPath(scriptPath)], {
        cwd: dir,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
        env: {
          ...process.env,
          GITHUB_REF: `refs/tags/${tag}`,
          GITHUB_OUTPUT: bashPath(outPath),
        },
      });
      const channel = (readFileSync(outPath, "utf8").match(/channel=(\S+)/) || [])[1];
      return { ok: true, stdout, stderr: "", channel };
    } catch (e) {
      return {
        ok: false,
        stdout: e.stdout ? String(e.stdout) : "",
        stderr: e.stderr ? String(e.stderr) : "",
        channel: undefined,
      };
    }
  } finally {
    removeTemp(dir);
  }
}

// A live promote fast-forwards main over a commit an auto-alpha already
// tagged. The alpha tag was correct when created; the promote overtook
// its queued run. Skip, don't fail — the content shipped as the live tag.
test("a pre-release overtaken by a live promote is skipped, not failed", () => {
  const r = runValidate(({ git, commit }) => {
    commit("feature");
    const alpha = commit("chore: bump platform to v1.8.148-alpha.1");
    git("tag", "v1.8.148-alpha.1", alpha);
    const live = commit("chore: bump platform to v1.8.148");
    git("tag", "v1.8.148", live);
    git("branch", "-f", "next", "main");
  }, "v1.8.148-alpha.1");

  assert.equal(r.ok, true, `expected success, stderr: ${r.stderr}`);
  assert.equal(r.channel, "superseded");
  assert.match(r.stdout + r.stderr, /superseded by live tag v1\.8\.148/);
});

// The invariant still has to do its job: a pre-release sitting on main
// with nothing live at or ahead of it is a genuine mistag.
test("a genuine mistag on main still fails", () => {
  const r = runValidate(({ git, commit }) => {
    const live = commit("chore: bump platform to v1.8.148");
    git("tag", "v1.8.148", live);
    const stray = commit("docs: tweak");
    git("tag", "v1.8.149-alpha.1", stray);
    git("branch", "-f", "next", "main");
  }, "v1.8.149-alpha.1");

  assert.equal(r.ok, false, "expected the mistag to be rejected");
  assert.match(r.stderr, /no live tag at or ahead of it/);
});

// Only a BARE semver tag proves a promote happened. Another pre-release
// ahead of the commit must not vouch for it.
test("a sibling pre-release tag does not count as supersession", () => {
  const r = runValidate(({ git, commit }) => {
    const live = commit("chore: bump platform to v1.8.148");
    git("tag", "v1.8.148", live);
    const stray = commit("docs: tweak");
    git("tag", "v1.8.149-alpha.1", stray);
    const later = commit("more");
    git("tag", "v1.8.149-alpha.2", later);
    git("branch", "-f", "next", "main");
  }, "v1.8.149-alpha.1");

  assert.equal(r.ok, false, "a pre-release must not vouch for another");
  assert.match(r.stderr, /no live tag at or ahead of it/);
});

// A normal pre-release on next and NOT on main is untouched by any of
// this — the common case must keep resolving to its own channel.
test("an ordinary pre-release on next resolves to its channel", () => {
  const r = runValidate(({ git, commit }) => {
    const base = commit("chore: bump platform to v1.8.148");
    git("tag", "v1.8.148", base);
    git("branch", "next", "main");
    git("checkout", "-q", "next");
    const alpha = commit("chore: bump platform to v1.8.149-alpha.1");
    git("tag", "v1.8.149-alpha.1", alpha);
    git("checkout", "-q", "main");
  }, "v1.8.149-alpha.1");

  assert.equal(r.ok, true, `expected success, stderr: ${r.stderr}`);
  assert.equal(r.channel, "alpha");
});

// Live tags are unaffected by the pre-release branch entirely.
test("a live tag on main resolves to the live channel", () => {
  const r = runValidate(({ git, commit }) => {
    const live = commit("chore: bump platform to v1.8.148");
    git("tag", "v1.8.148", live);
    git("branch", "-f", "next", "main");
  }, "v1.8.148");

  assert.equal(r.ok, true, `expected success, stderr: ${r.stderr}`);
  assert.equal(r.channel, "live");
});

// The case the first version of this fix MISSED, observed live on
// v1.8.149-alpha.6: the promote has fast-forwarded main over the alpha
// commit and bumped the version file, but has NOT yet created its bare
// tag. The alpha's own queued run validates inside that gap. A
// tag-existence check finds nothing and dies; the version bump is
// already visible.
test("a pre-release promoted but not yet tagged is skipped, not failed", () => {
  const r = runValidate(({ git, commitVersion }) => {
    commitVersion("chore: bump platform to v1.8.149-alpha.6", "1.8.149-alpha.6");
    // The promote bumps the version file, THEN tags. Model the gap: the
    // bump landed on main, the bare tag does not exist yet.
    const promoted = commitVersion("chore: bump platform to v1.8.149", "1.8.149");
    git("tag", "v1.8.149-alpha.6", promoted);
    git("branch", "-f", "next", "main");
  }, "v1.8.149-alpha.6");

  assert.equal(r.ok, true, `expected success, stderr: ${r.stderr}`);
  assert.equal(r.channel, "superseded");
  assert.match(r.stdout + r.stderr, /superseded by version 1\.8\.149/);
});

// The guard must not swallow a genuine mistag just because a version
// file exists: an un-promoted commit still carries a pre-release
// version, so the new signal stays silent and the old `die` stands.
test("a mistag on an un-promoted pre-release commit still fails", () => {
  const r = runValidate(({ git, commitVersion }) => {
    const live = commitVersion("chore: bump platform to v1.8.148", "1.8.148");
    git("tag", "v1.8.148", live);
    // Still a pre-release version -> not promoted -> not superseded.
    const stray = commitVersion("chore: bump platform to v1.8.150-alpha.1", "1.8.150-alpha.1");
    git("tag", "v1.8.150-alpha.1", stray);
    git("branch", "-f", "next", "main");
  }, "v1.8.150-alpha.1");

  assert.equal(r.ok, false, "expected the mistag to be rejected");
  assert.match(r.stderr, /no live tag at or ahead of it/);
});

// A bare version that does NOT match this tag's base is somebody else's
// release, not this one's promotion.
test("a bare version for a different release does not vouch", () => {
  const r = runValidate(({ git, commitVersion }) => {
    const other = commitVersion("chore: bump platform to v1.8.148", "1.8.148");
    git("tag", "v1.8.150-alpha.1", other);
    git("branch", "-f", "next", "main");
  }, "v1.8.150-alpha.1");

  assert.equal(r.ok, false, "a mismatched bare version must not vouch");
  assert.match(r.stderr, /no live tag at or ahead of it/);
});
