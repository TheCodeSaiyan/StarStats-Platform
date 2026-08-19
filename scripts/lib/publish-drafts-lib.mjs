// Pure helpers for publish-roadmap-drafts.mjs.
//
// Extracted into their own module so they're unit-testable without
// having to spawn the CLI entry point (which validates env on import
// and immediately runs against a live server).

export function shortSha(sha) {
  return typeof sha === 'string' && sha.length >= 7 ? sha.slice(0, 7) : sha || null;
}

export function channelLabel(channel) {
  switch (channel) {
    case 'live':
      return 'Live';
    case 'beta':
      return 'Beta';
    case 'alpha':
      return 'Alpha';
    case 'tech-preview':
      return 'Tech Preview';
    default:
      return channel || '?';
  }
}

// Compose a fleshed-out title + body for one draft using its parent
// item's slug + title (from the public roadmap index). Falls back to
// "item <uuid>" framing if the parent isn't in the index (rare — the
// caller is publishing changelog for it, so it should already be
// Public=Yes and therefore present in /v1/roadmap output).
export function composeRewrite(draft, itemIndex) {
  const parent = itemIndex.get(draft.roadmap_item_id);
  const channel = channelLabel(draft.channel);
  const itemTitle = parent ? parent.title : `item ${String(draft.roadmap_item_id).slice(0, 8)}`;
  const slug = parent ? parent.slug : '';

  const title = `${itemTitle} — Shipped to ${channel}`;

  const lines = [];
  lines.push(`**${itemTitle}** is now available on the ${channel} channel.`);
  lines.push('');
  if (slug) {
    lines.push(`Track this item on the roadmap: \`${slug}\``);
  }
  const sha = shortSha(draft.shipped_sha);
  const prev = shortSha(draft.previous_shipped_sha);
  if (sha && prev) {
    lines.push(`Build range: \`${prev}\` → \`${sha}\``);
  } else if (sha) {
    lines.push(`Build: \`${sha}\``);
  }
  // Preserve any extra context from the auto-draft body so we don't
  // silently lose CI links or commit lists the receiver appended.
  const original = (draft.body || '').trim();
  if (original) {
    lines.push('');
    lines.push('---');
    lines.push(original);
  }

  return { title, body: lines.join('\n') };
}

export function formatDraftLine(draft, itemIndex) {
  const ts = draft.created_at
    ? new Date(draft.created_at).toISOString().replace('T', ' ').slice(0, 16)
    : '????-??-?? ??:??';
  const channel = (draft.channel || '?').padEnd(6);
  const parent = itemIndex.get(draft.roadmap_item_id);
  const itemBit = parent
    ? `[${parent.slug}] ${parent.title}`
    : `[item ${String(draft.roadmap_item_id).slice(0, 8)}] (not in public list)`;
  return `  ${ts}  ${channel}  ${draft.id}  ${itemBit}`;
}
