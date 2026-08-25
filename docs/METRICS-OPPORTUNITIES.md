# What else the data we already collect could tell you

Research pass over the existing capture surface: what is collected, what is
computed, what is shown, and what falls through the gaps. Nothing here needs a
new parser rule or a new event — every idea below is derivable from data the
tray already sends and the server already stores.

## Method

Four inventories, cross-referenced:

1. **What is captured** — the 36 variants of `GameEvent`
   (`crates/starstats-core/src/events.rs`) and their ~120 payload fields.
2. **What is computed** — the 40 me-scoped endpoints the server exposes.
3. **What is shown** — which `lib/api.ts` wrappers any component actually
   calls.
4. **What is never touched** — payload fields with zero references anywhere in
   `crates/starstats-server/src/`.

One correction worth recording, because it changes the answer: my first pass
grepped components for endpoint URL strings and concluded five endpoints were
unused. That was wrong — components import the wrapper (`getTravelStats`), not
the URL. Grepping by wrapper name cut the list from five to three.

---

## Tier 1 — already built, never shown

These endpoints exist, are tested, and have typed clients. No component calls
them. This is finished work sitting behind no UI.

| what | returns | note |
| --- | --- | --- |
| `combat.top_weapons` | weapon → kill count | **Already fetched by `/me` and thrown away** |
| `combat.deaths_by_zone` | zone → death count | Same response, same fate |
| `/v1/me/stats/stability` | `crashes`, `by_channel` | Crash count split by LIVE/PTU |
| `/v1/me/stats/loadout` | `equips`, `stores`, `top_items` | Distinct from the paperdoll — this is *activity*, not the current kit |
| `/v1/me/metrics/sessions` | per-session `start_at`, `end_at`, `event_count` | Every session as a row |

**`top_weapons` and `deaths_by_zone` are the cheapest wins in the product.**
`/me` already calls `getCombatStats` and already receives both; the response is
destructured for `kills` and `deaths` and the other two fields are dropped on
the floor. A "weapon of choice" board and a "deadliest zone" list need no new
fetch, no new query, and no new collection — only rendering. `top_weapons` is
deliberately scoped kill-side (the comment in `query.rs` is explicit that
showing weapons that killed *you* would be a different metric), so it reads as
"what you kill with".

**Stability** is the striking one. `lives` already reports
`lives_ended_by_crash`, but a dedicated crash rate — crashes per hour, per
channel, and a crash-free streak — is a metric this game's players genuinely
compare. It is one fetch away.

**`metrics/sessions`** unlocks a whole class of shapes the current widgets
cannot make: session length distribution (not just the mean), a session
histogram, "longest gap between sessions", and a real activity timeline rather
than a per-day count.

---

## Tier 2 — captured, stored, never queried

Fields with **zero** references in the server. Each needs one new query; the
data is already in the events table.

### Combat and loss

- **`VehicleDestruction.caused_by`** — what destroyed the ship. Turns "hulls
  lost: 4" into "what keeps killing your ships".
- **`VehicleDestruction.destroy_level`** — soft death vs hull loss. Two very
  different events currently counted as one.
- **`PlayerIncapacitated.queue_id`** — downed but not dead. Nothing surfaces
  incapacitation at all today, so the ratio of *downed* to *killed* is
  invisible. That ratio is close to a skill signal.

### Trade

- **`CommodityBuyRequest` / `CommoditySellRequest`** — `commodity`, `quantity`.
  Entirely unmined. `spend` covers kiosk purchases (`ShopBuyRequest.price`) but
  commodity trading has no metric at all: no volume, no commodity mix, no
  buy/sell split.

### Kiosks and gear

- **`ShopRequestTimedOut.timed_out_after_secs`** — failed purchases and how
  long you waited. A "kiosk reliability" figure is both useful and, given the
  game, funny.
- **`AttachmentReceived.elapsed_seconds`** — how long gear takes to attach.
  Slowest restores, and a proxy for server health at the time.

### Place and shard

- **`SeedSolarSystem.solar_system`** — systems seeded, and the success flag.
  Stanton vs Pyro split with no location parsing needed.
- **`GameCrash.total_size_bytes`** — cumulative crash-dump size. A silly,
  memorable number.

---

## Tier 3 — derivable now, no new queries

Combinations of what is already computed.

### Ratios that need two existing numbers

- **Cost per hour** — `spend.total_auec` ÷ `playtime.total_playtime_secs`.
- **Deaths per jump** — `combat.deaths` ÷ `travel.quantum_jumps`. "How far you
  usually get."
- **Events per hour** — `summary.total` ÷ playtime. A crude intensity measure
  that makes a good sparkline.
- **Purchases per session** — `spend.purchases` ÷ `playtime.session_count`.

### Abandonment, from paired events

The travel chain emits three distinct events: `QuantumTargetSelected` →
`QuantumRoute` → `QuantumArrived`. Only the first is counted today.

- **Jumps started vs completed** — selected minus arrived. Quantum
  interdictions, changed minds, and deaths mid-route all live in that gap.
- **Most-abandoned destination** — where you set course and never arrive.
- `TravelToContractLocation` carries `travel_started` / `travel_completed`
  explicitly, so contract travel has the same shape with no inference.

### Streaks and firsts

`records` already covers longest session, busiest session, longest survival
streak and deadliest session. Not covered:

- **Days since last death** — a live counter, not a window aggregate.
- **First seen / last seen per entity.** Every event carries a timestamp and a
  class name, so "you haven't flown the Railen since March" is a join away.
  This is the one I would build first: it is the only idea here that gets
  *more* interesting the longer someone uses the product.
- **Crash-free streak** — sessions since the last `GameCrash`.

### Shard behaviour

`JoinPu.shard` is already referenced but nothing surfaces it.

- **Shards visited**, and whether you keep landing on the same one.
- **Shard hops per session** — `ChangeServer` frequency.
- `ChangeServer.is_multiplayer` / `is_online` give a solo-vs-PU split for free.

---

## What is already covered (so as not to rebuild it)

The `facts` engine already derives seven: `busiest_weekday`, `death_tempo`,
`flight_cadence`, `night_owl`, `playtime_concentration`, `session_rhythm`,
`weekly_pace`. Time-of-day and day-of-week patterns are done — any "when do you
play" idea should extend that engine rather than start a new one.

`records` covers personal bests. `journey`, `routes`, `corridors` and
`locations` cover movement. `loadout` covers the current kit.

---

## Where I would start

1. **Render `top_weapons` and `deaths_by_zone`** — the data is already on the
   page. This is a rendering task, not a data task.
2. **Surface `stability`** — a finished endpoint, one fetch, a real metric.
3. **Split incapacitation from death** — one query, and it makes the combat
   numbers honest rather than merely bigger.
4. **First-seen / last-seen per entity** — the only idea here that compounds
   with use, and it feeds "you haven't flown X in N months" prompts.
5. **Commodity trading** — the largest genuinely blank area; an entire
   gameplay loop with no metrics at all.

## What is NOT here

Anything needing new collection.

One thing I checked and could not confirm: a **nemesis board** (who kills you
most). `ActorDeath` carries `killer`, and the server filters on it — but only
to decide whether the caller was the killer, for the kill count. Nothing
aggregates *other people's* handles as killers of the caller. That is a new
query rather than a new event, so it belongs in Tier 2; I have left it out of
the list because I have not verified the victim-side filter would work on
inferred deaths, which carry `body_class = "inferred"` and may have no killer
at all.
