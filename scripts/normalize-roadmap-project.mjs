#!/usr/bin/env node
// Audit and normalize a StarStats roadmap GitHub Project.
//
// One normalization is applied to every Project item by default:
//
//   - The linked Issue/PR (DraftIssue items have no labelable, so
//     they're skipped with a warning) gets `surface/web-roadmap` and
//     `surface/tray-whats-new` labels if either is missing. Per
//     `mapper.rs:174-189`, surfaces ride on labels because
//     ProjectsV2 has no multi-select custom field.
//
// The `Public` custom field is intentionally LEFT ALONE by default.
// Operators typically curate visibility per-item (some items are
// deliberately private even though they exist on the board), so the
// audit shows the current `Public` state as informational but does
// not propose a change. Pass `--set-public-yes` to force every
// non-`Yes` item to `Yes` — only do this on a fresh board where
// you want everything visible.
//
// Channel labels (`channel/live`, `channel/beta`, etc.) are NOT
// blanket-applied — channel targeting is per-feature intent and
// shouldn't be guessed from the script.
//
// Dry-run by default. Pass `--apply` to commit mutations.
//
// Required:
//   --owner    <org-or-user-login>    e.g. TheCodeSaiyan
//   --project  <number>               Project number (the integer in
//                                      the Project URL, not the node id)
//
// Optional:
//   --apply              Commit mutations. Without this, the script
//                        only prints what would change.
//   --set-public-yes     Force every Project item's `Public` field to
//                        `Yes`. WARNING: overwrites manually-curated
//                        per-item visibility. Skip this flag when
//                        operators have set Public deliberately
//                        (the normal case).
//   --create-labels      Create surface/* labels in the linked repo if
//                        they don't exist (otherwise the script warns
//                        and skips that label for that item).
//   --filter <substr>    Only consider items whose title contains the
//                        substring (case-insensitive).
//   --help               Print usage.
//
// Single-item promotion mode (alternative to the full audit flow):
//   --promote-draft <ref>     Convert a DraftIssue to a real Issue and
//                             apply surface + channel labels in one
//                             shot. <ref> is either a Project item ID
//                             (starts with `PVTI_`) or a unique title
//                             substring (case-insensitive).
//   --repo <owner/name>       Target repository for the new Issue.
//                             Default: TheCodeSaiyan/StarStats-Platform.
//   --add-surfaces <csv>      Surface label slugs (without prefix) to
//                             add post-promotion. Default:
//                             web-roadmap,tray-whats-new.
//   --add-channels <csv>      Channel label slugs (without prefix) to
//                             add post-promotion. Default: empty —
//                             channel targeting is per-feature intent.
//   Note: --promote-draft is a destructive transition (creates a real
//         Issue, attaches labels). Always pair with --apply; without
//         --apply the script previews the conversion without writing.
//
// Authentication: relies on local `gh` CLI auth. Run `gh auth status`
// first; the token needs the `project` scope (org-Project write) plus
// `repo` for label mutations.
//
// Exit codes:
//   0 — dry-run completed OR --apply finished with zero failures.
//   1 — at least one mutation failed during --apply.
//   2 — config error (missing flag, bad input, gh not available).

import { spawnSync } from 'node:child_process';

// ---------- arg parsing ----------------------------------------------------

function fatal(code, msg) {
  console.error(`[normalize-roadmap] ${msg}`);
  process.exit(code);
}

function usage() {
  console.log(
    [
      'Usage:',
      '  Audit mode:    node scripts/normalize-roadmap-project.mjs --owner <login> --project <number> [--apply] [--set-public-yes] [--create-labels] [--filter <substr>]',
      '  Promote mode:  node scripts/normalize-roadmap-project.mjs --owner <login> --project <number> --promote-draft <id-or-substr> [--repo <owner/name>] [--add-surfaces <csv>] [--add-channels <csv>] [--create-labels] [--apply]',
      '',
      'Audit:   surveys all items + applies surface/* label fixes to real Issues.',
      'Promote: converts ONE DraftIssue → real Issue and attaches surface + channel labels.',
      '',
      'Auth: uses local `gh` CLI (needs `project` + `repo` scopes).',
    ].join('\n'),
  );
}

const args = process.argv.slice(2);
const opts = {
  owner: null,
  project: null,
  apply: false,
  setPublicYes: false,
  createLabels: false,
  filter: null,
  promoteDraft: null,
  repo: 'TheCodeSaiyan/StarStats-Platform',
  addSurfaces: ['web-roadmap', 'tray-whats-new'],
  addChannels: [],
};

function parseCsv(v) {
  return v
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}

for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--help' || a === '-h') {
    usage();
    process.exit(0);
  } else if (a === '--owner') {
    opts.owner = args[++i];
    if (!opts.owner) fatal(2, '--owner requires a value');
  } else if (a === '--project') {
    const v = args[++i];
    const n = Number.parseInt(v, 10);
    if (!Number.isInteger(n) || n <= 0) fatal(2, `--project requires a positive integer (got ${v})`);
    opts.project = n;
  } else if (a === '--apply') {
    opts.apply = true;
  } else if (a === '--set-public-yes') {
    opts.setPublicYes = true;
  } else if (a === '--create-labels') {
    opts.createLabels = true;
  } else if (a === '--filter') {
    opts.filter = args[++i];
    if (!opts.filter) fatal(2, '--filter requires a value');
  } else if (a === '--promote-draft') {
    opts.promoteDraft = args[++i];
    if (!opts.promoteDraft) fatal(2, '--promote-draft requires a value (item id or title substring)');
  } else if (a === '--repo') {
    opts.repo = args[++i];
    if (!opts.repo || !opts.repo.includes('/')) fatal(2, '--repo requires owner/name');
  } else if (a === '--add-surfaces') {
    const v = args[++i];
    if (!v) fatal(2, '--add-surfaces requires a comma-separated list (or "" for none)');
    opts.addSurfaces = parseCsv(v);
  } else if (a === '--add-channels') {
    const v = args[++i];
    if (v === undefined) fatal(2, '--add-channels requires a comma-separated list (or "" for none)');
    opts.addChannels = parseCsv(v);
  } else {
    fatal(2, `unknown flag: ${a} (try --help)`);
  }
}

if (!opts.owner) fatal(2, '--owner is required');
if (opts.project === null) fatal(2, '--project is required');

// ---------- gh wrapper -----------------------------------------------------

const TARGET_SURFACES = ['surface/web-roadmap', 'surface/tray-whats-new'];

function ghJson(args) {
  const result = spawnSync('gh', args, { encoding: 'utf8', maxBuffer: 50 * 1024 * 1024 });
  if (result.error && result.error.code === 'ENOENT') {
    fatal(2, '`gh` CLI not found on PATH. Install it from https://cli.github.com/');
  }
  if (result.status !== 0) {
    const stderr = (result.stderr || '').trim();
    const stdout = (result.stdout || '').trim();
    throw new Error(`gh exit ${result.status}: ${stderr || stdout}`);
  }
  return result.stdout ? JSON.parse(result.stdout) : {};
}

function graphql(query, vars = {}) {
  const args = ['api', 'graphql', '-f', `query=${query}`];
  for (const [k, v] of Object.entries(vars)) {
    if (typeof v === 'number') args.push('-F', `${k}=${v}`);
    else args.push('-f', `${k}=${v}`);
  }
  return ghJson(args);
}

// ---------- queries --------------------------------------------------------

const PROJECT_META_ORG_Q = `
query($owner: String!, $number: Int!) {
  organization(login: $owner) {
    projectV2(number: $number) {
      id
      title
      url
      fields(first: 50) {
        nodes {
          __typename
          ... on ProjectV2SingleSelectField {
            id
            name
            options { id name }
          }
        }
      }
    }
  }
}`;

const PROJECT_META_USER_Q = `
query($owner: String!, $number: Int!) {
  user(login: $owner) {
    projectV2(number: $number) {
      id
      title
      url
      fields(first: 50) {
        nodes {
          __typename
          ... on ProjectV2SingleSelectField {
            id
            name
            options { id name }
          }
        }
      }
    }
  }
}`;

const ITEMS_PAGE_Q = `
query($projectId: ID!, $after: String) {
  node(id: $projectId) {
    ... on ProjectV2 {
      items(first: 50, after: $after) {
        nodes {
          id
          content {
            __typename
            ... on Issue {
              id
              number
              title
              repository { nameWithOwner id }
              labels(first: 50) { nodes { id name } }
            }
            ... on PullRequest {
              id
              number
              title
              repository { nameWithOwner id }
              labels(first: 50) { nodes { id name } }
            }
            ... on DraftIssue { title }
          }
          fieldValues(first: 30) {
            nodes {
              __typename
              ... on ProjectV2ItemFieldSingleSelectValue {
                name
                field {
                  ... on ProjectV2SingleSelectField { id name }
                }
              }
            }
          }
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}`;

const SET_FIELD_M = `
mutation($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
  updateProjectV2ItemFieldValue(input: {
    projectId: $projectId
    itemId: $itemId
    fieldId: $fieldId
    value: { singleSelectOptionId: $optionId }
  }) {
    projectV2Item { id }
  }
}`;

const ADD_LABELS_M = `
mutation($labelableId: ID!, $labelIds: [ID!]!) {
  addLabelsToLabelable(input: { labelableId: $labelableId, labelIds: $labelIds }) {
    labelable { __typename }
  }
}`;

const REPO_LABEL_Q = `
query($owner: String!, $repo: String!) {
  repository(owner: $owner, name: $repo) {
    labels(first: 100) { nodes { id name } }
  }
}`;

const CREATE_LABEL_M = `
mutation($repoId: ID!, $name: String!) {
  createLabel(input: { repositoryId: $repoId, name: $name, color: "ededed" }) {
    label { id name }
  }
}`;

const REPO_ID_Q = `
query($owner: String!, $repo: String!) {
  repository(owner: $owner, name: $repo) { id nameWithOwner }
}`;

const CONVERT_DRAFT_M = `
mutation($itemId: ID!, $repositoryId: ID!) {
  convertProjectV2DraftIssueItemToIssue(input: {
    itemId: $itemId
    repositoryId: $repositoryId
  }) {
    item {
      id
      content {
        __typename
        ... on Issue {
          id
          number
          title
          url
          repository { nameWithOwner }
        }
      }
    }
  }
}`;

// ---------- fetchers -------------------------------------------------------

function fetchProjectMeta() {
  // Try organization first, fall back to user. GitHub's graphql
  // errors hard when a query references both an org and a user that
  // don't both exist, so we have to issue separate queries.
  let project = null;
  try {
    const resp = graphql(PROJECT_META_ORG_Q, { owner: opts.owner, number: opts.project });
    project = resp?.data?.organization?.projectV2 ?? null;
  } catch (e) {
    if (!/Could not resolve to an Organization/i.test(String(e.message))) throw e;
  }
  if (!project) {
    try {
      const resp = graphql(PROJECT_META_USER_Q, { owner: opts.owner, number: opts.project });
      project = resp?.data?.user?.projectV2 ?? null;
    } catch (e) {
      if (!/Could not resolve to a User/i.test(String(e.message))) throw e;
    }
  }
  if (!project) {
    fatal(
      2,
      `Project #${opts.project} not found under organization or user "${opts.owner}". Check the owner login and number.`,
    );
  }
  const fields = project.fields.nodes.filter(Boolean);
  const publicField = fields.find(
    (f) => f.__typename === 'ProjectV2SingleSelectField' && f.name === 'Public',
  );
  if (!publicField) {
    fatal(
      2,
      `Project "${project.title}" has no single-select field named "Public". Create one with options Yes / No before running this script (spec §3.3).`,
    );
  }
  const yesOption = publicField.options.find((o) => o.name === 'Yes');
  if (!yesOption) {
    fatal(2, `The "Public" field has no "Yes" option. Add Yes/No options before running.`);
  }
  return { project, publicField, yesOption };
}

function* paginateItems(projectId) {
  let after = null;
  while (true) {
    const resp = graphql(ITEMS_PAGE_Q, { projectId, after: after || '' });
    const page = resp?.data?.node?.items;
    if (!page) break;
    for (const item of page.nodes) yield item;
    if (!page.pageInfo.hasNextPage) break;
    after = page.pageInfo.endCursor;
  }
}

const repoLabelCache = new Map();
function fetchRepoLabels(nwo) {
  if (repoLabelCache.has(nwo)) return repoLabelCache.get(nwo);
  const [owner, repo] = nwo.split('/');
  const resp = graphql(REPO_LABEL_Q, { owner, repo });
  const labels = resp?.data?.repository?.labels?.nodes ?? [];
  const map = new Map(labels.map((l) => [l.name, l.id]));
  repoLabelCache.set(nwo, map);
  return map;
}

// ---------- audit ----------------------------------------------------------

function auditItem(item, publicField) {
  const result = {
    itemId: item.id,
    title: item.content?.title || '(no title)',
    contentType: item.content?.__typename || '(no content)',
    issues: [],
  };

  // Public field check — informational unless --set-public-yes.
  // Operators usually curate Public per-item; we don't propose a
  // change unless the operator explicitly asked to flatten it.
  const publicFv = item.fieldValues.nodes.find(
    (fv) => fv?.field?.id === publicField.id || fv?.field?.name === 'Public',
  );
  const currentPublic = publicFv?.name ?? null;
  result.currentPublic = currentPublic;
  if (currentPublic !== 'Yes' && opts.setPublicYes) {
    result.issues.push({ kind: 'public_not_yes', current: currentPublic });
  }

  // Surface labels — only meaningful for Issue/PR content (DraftIssue
  // has no labelable, so we just warn).
  const labelable =
    item.content?.__typename === 'Issue' || item.content?.__typename === 'PullRequest'
      ? item.content
      : null;
  if (labelable) {
    const labelNames = new Set(labelable.labels.nodes.map((l) => l.name));
    const missing = TARGET_SURFACES.filter((s) => !labelNames.has(s));
    if (missing.length > 0) {
      result.issues.push({
        kind: 'missing_surface_labels',
        missing,
        labelableId: labelable.id,
        repoNwo: labelable.repository.nameWithOwner,
        repoId: labelable.repository.id,
      });
    }
  } else if (item.content?.__typename === 'DraftIssue') {
    result.issues.push({ kind: 'draft_issue_no_labels' });
  }

  return result;
}

// ---------- mutations ------------------------------------------------------

function applySetPublicYes(projectId, itemId, publicField, yesOption) {
  graphql(SET_FIELD_M, {
    projectId,
    itemId,
    fieldId: publicField.id,
    optionId: yesOption.id,
  });
}

function applyAddLabels(labelableId, labelIds) {
  // gh CLI: arrays are built by repeating `-f key[]=value`. Passing
  // `-f key=<json-array>` makes gh treat the whole string as one
  // value, which GraphQL then rejects as an invalid node id.
  const args = [
    'api',
    'graphql',
    '-f',
    `query=${ADD_LABELS_M}`,
    '-f',
    `labelableId=${labelableId}`,
  ];
  for (const id of labelIds) {
    args.push('-f', `labelIds[]=${id}`);
  }
  ghJson(args);
}

function createLabel(repoId, name) {
  const resp = graphql(CREATE_LABEL_M, { repoId, name });
  return resp?.data?.createLabel?.label;
}

// ---------- main -----------------------------------------------------------

const { project, publicField, yesOption } = fetchProjectMeta();

// ---- single-item promotion mode ------------------------------------------

function fetchRepoId(nwo) {
  const [owner, repo] = nwo.split('/');
  const resp = graphql(REPO_ID_Q, { owner, repo });
  const id = resp?.data?.repository?.id;
  if (!id) fatal(2, `Could not resolve repository "${nwo}". Check the owner/name.`);
  return id;
}

function findDraftItem(ref) {
  // PVTI_-prefixed → treat as exact item id. Otherwise → case-insensitive
  // title substring search across all DraftIssue items.
  if (ref.startsWith('PVTI_')) {
    for (const item of paginateItems(project.id)) {
      if (item.id === ref) {
        if (item.content?.__typename !== 'DraftIssue') {
          fatal(1, `Item ${ref} is a ${item.content?.__typename}, not a DraftIssue — nothing to convert.`);
        }
        return item;
      }
    }
    fatal(1, `No Project item with id ${ref}`);
  }
  const needle = ref.toLowerCase();
  const matches = [];
  for (const item of paginateItems(project.id)) {
    if (item.content?.__typename !== 'DraftIssue') continue;
    if ((item.content.title || '').toLowerCase().includes(needle)) matches.push(item);
  }
  if (matches.length === 0) fatal(1, `No DraftIssue title contains "${ref}".`);
  if (matches.length > 1) {
    fatal(
      1,
      `Title substring "${ref}" matched ${matches.length} drafts:\n${matches.map((m) => `  - ${m.id}  ${m.content.title}`).join('\n')}\nNarrow the substring or pass the item id directly.`,
    );
  }
  return matches[0];
}

function promoteFlow() {
  const draft = findDraftItem(opts.promoteDraft);
  const surfaces = opts.addSurfaces.map((s) => `surface/${s}`);
  const channels = opts.addChannels.map((c) => `channel/${c}`);
  const allLabels = [...surfaces, ...channels];

  console.log(
    `[normalize-roadmap] Project: ${project.title}\n  url:    ${project.url}\n  mode:   ${opts.apply ? 'APPLY' : 'dry-run'}\n  action: promote draft → real Issue in ${opts.repo}\n  draft:  ${draft.id}\n  title:  ${draft.content.title}\n  labels: ${allLabels.length === 0 ? '(none)' : allLabels.join(', ')}`,
  );

  if (!opts.apply) {
    console.log(`\n[normalize-roadmap] dry-run: would convert ${draft.id} and add ${allLabels.length} label(s). Re-run with --apply to commit.`);
    process.exit(0);
  }

  const repoId = fetchRepoId(opts.repo);
  let issue;
  try {
    const resp = graphql(CONVERT_DRAFT_M, { itemId: draft.id, repositoryId: repoId });
    issue = resp?.data?.convertProjectV2DraftIssueItemToIssue?.item?.content;
    if (!issue?.id) {
      fatal(1, `Conversion mutation returned no Issue payload (response: ${JSON.stringify(resp).slice(0, 300)})`);
    }
    console.log(`  ok      converted → #${issue.number} ${issue.url}`);
  } catch (e) {
    fatal(1, `Conversion failed: ${e.message}`);
  }

  if (allLabels.length === 0) {
    console.log(`[normalize-roadmap] done. No labels requested.`);
    process.exit(0);
  }
  const repoLabels = fetchRepoLabels(opts.repo);
  const labelIds = [];
  const skipped = [];
  for (const name of allLabels) {
    let id = repoLabels.get(name);
    if (!id) {
      if (opts.createLabels) {
        const created = createLabel(repoId, name);
        if (created?.id) {
          id = created.id;
          repoLabels.set(name, id);
          console.log(`  +label  ${opts.repo}:${name} (created)`);
        }
      } else {
        skipped.push(name);
        continue;
      }
    }
    if (id) labelIds.push(id);
  }

  if (labelIds.length > 0) {
    try {
      applyAddLabels(issue.id, labelIds);
      console.log(`  ok      attached ${labelIds.length} label(s) to #${issue.number}`);
    } catch (e) {
      console.log(`  FAIL    add-labels: ${e.message}`);
      process.exit(1);
    }
  }
  if (skipped.length > 0) {
    console.log(
      `  warn    labels not present in ${opts.repo}, skipped: ${skipped.join(', ')} (re-run with --create-labels)`,
    );
  }

  console.log(`\n[normalize-roadmap] promotion complete. Webhook should fire on the new Issue + label adds; reconciler backstops within 5 min.`);
  process.exit(0);
}

if (opts.promoteDraft) {
  promoteFlow();
}

// ---- full-audit mode (default) -------------------------------------------

const publicMode = opts.setPublicYes ? 'force Public=Yes' : 'leave Public as-is (use --set-public-yes to flatten)';
console.log(
  `[normalize-roadmap] Project: ${project.title}\n  url:    ${project.url}\n  mode:   ${opts.apply ? 'APPLY' : 'dry-run'}\n  labels: ${TARGET_SURFACES.join(' + ')}\n  public: ${publicMode}`,
);

const audits = [];
let totalItems = 0;
for (const item of paginateItems(project.id)) {
  totalItems += 1;
  if (opts.filter) {
    const title = (item.content?.title || '').toLowerCase();
    if (!title.includes(opts.filter.toLowerCase())) continue;
  }
  audits.push(auditItem(item, publicField));
}

const INFORMATIONAL_KINDS = new Set(['draft_issue_no_labels']);
const isActionable = (issue) => !INFORMATIONAL_KINDS.has(issue.kind);
const withIssues = audits.filter((a) => a.issues.length > 0);
const actionable = audits.filter((a) => a.issues.some(isActionable));
console.log(
  `\n[normalize-roadmap] ${totalItems} item(s) scanned; ${actionable.length} actionable, ${withIssues.length - actionable.length} informational-only.`,
);
if (withIssues.length === 0) {
  console.log('[normalize-roadmap] nothing to do.');
  process.exit(0);
}

for (const a of withIssues) {
  const publicTag = a.currentPublic === 'Yes' ? 'Public=Yes' : `Public=${a.currentPublic ?? '(unset)'}`;
  console.log(`\n  ${a.contentType}  [${publicTag}]  ${a.title}`);
  for (const issue of a.issues) {
    if (issue.kind === 'public_not_yes') {
      console.log(`    - Public field: ${issue.current ?? '(unset)'} → Yes`);
    } else if (issue.kind === 'missing_surface_labels') {
      console.log(`    - Missing labels on ${issue.repoNwo}: ${issue.missing.join(', ')}`);
    } else if (issue.kind === 'draft_issue_no_labels') {
      console.log(`    - DraftIssue: promote to a real Issue/PR to get surface/* labels (spec §3.3)`);
    }
  }
}

if (!opts.apply) {
  console.log('\n[normalize-roadmap] dry-run complete. Re-run with --apply to commit.');
  process.exit(0);
}

console.log('\n[normalize-roadmap] applying mutations...');
let failures = 0;
let publicFixed = 0;
let labelsFixed = 0;

for (const a of withIssues) {
  for (const issue of a.issues) {
    try {
      if (issue.kind === 'public_not_yes') {
        applySetPublicYes(project.id, a.itemId, publicField, yesOption);
        publicFixed += 1;
        console.log(`  ok      ${a.title}  Public=Yes`);
      } else if (issue.kind === 'missing_surface_labels') {
        const repoLabels = fetchRepoLabels(issue.repoNwo);
        const labelIds = [];
        const skipped = [];
        for (const name of issue.missing) {
          let id = repoLabels.get(name);
          if (!id) {
            if (opts.createLabels) {
              const created = createLabel(issue.repoId, name);
              if (created?.id) {
                id = created.id;
                repoLabels.set(name, id);
                console.log(`  +label  ${issue.repoNwo}:${name} (created)`);
              }
            } else {
              skipped.push(name);
              continue;
            }
          }
          if (id) labelIds.push(id);
        }
        if (labelIds.length > 0) {
          applyAddLabels(issue.labelableId, labelIds);
          labelsFixed += labelIds.length;
          console.log(`  ok      ${a.title}  labels +${labelIds.length}`);
        }
        if (skipped.length > 0) {
          console.log(`  warn    ${a.title}  label(s) missing in ${issue.repoNwo}, skipped: ${skipped.join(', ')} (re-run with --create-labels to auto-create)`);
        }
      }
      // draft_issue_no_labels is informational; no mutation applies.
    } catch (e) {
      failures += 1;
      console.log(`  FAIL    ${a.title}  ${issue.kind}: ${e.message}`);
    }
  }
}

console.log(`\n[normalize-roadmap] summary: public_fixed=${publicFixed} labels_fixed=${labelsFixed} failures=${failures}`);
process.exit(failures === 0 ? 0 : 1);
