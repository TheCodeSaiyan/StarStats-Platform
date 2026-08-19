/**
 * Plain-language explanations of how each INFERRED /me metric is derived from
 * raw Game.log events. Keyed by a stable metric id (not an event_type, since a
 * metric can be a sum of several event types). Surfaced next to the number via
 * <InfoTip> so an inferred figure never reads as ground truth.
 *
 * As more widgets adopt InfoTip (M6), add the metric's key here — one registry,
 * so the copy stays consistent and reviewable in one place.
 *
 * RULE FOR ADDING ONE: every sentence must be traceable to what the widget
 * actually does. Each entry below was written against the widget's own load()
 * and its provenance comment, not from memory of what the metric sounds like.
 * A tip that is merely plausible is worse than no tip — it launders a guess
 * into an explanation.
 *
 * Don't add one where the widget already explains itself: `lives` marks
 * reconstructed deaths with <Provenance>, so a tip there would just repeat it.
 */
export const INFERENCE_EXPLANATIONS: Record<string, string> = {
  quantum_jumps:
    'Inferred from “quantum target selected” log lines — one per time you chose a quantum destination. Picking a target is intent, not a confirmed jump, so read this as jumps initiated.',
  server_hops:
    'The sum of “joined PU” and “server change” events — how many times your session connected or was moved to a game server this range.',

  ships_flown:
    'Ranked by quantum-travel trips, using the ship you were flying when you picked a quantum target. These are ships you have flown, not ships you own — for owned ships, see the Hangar widget.',

  docking_kind:
    'Counted from ship-stow events — the moment a ship is put away. That is not a dock turnstile: arriving somewhere and leaving without stowing is not counted here.',

  objectives_no_outcome:
    'Objectives where no final state was ever recorded — NOT objectives you currently have active. Abandoned missions, app exits and log rotations mid-mission all land here, so on a long-lived account it runs high.',

  kiosk_spend:
    'Totalled from kiosk purchase requests and the price listed on them. Shop buys only — it is not everything you spend aUEC on.',

  contract_outcomes:
    'Folded from the contract banners the game shows on your HUD, keyed by mission id. A run only resolves if its closing banner was logged; withdrawn and unrecognised outcomes are kept out of the rate and shown on the contract history instead.',

  biggest_trade:
    'Measured by quantity, not aUEC — unit count for commodity trades, item count for shop buys. The logs carry no typed value field, so a large cheap haul can outrank a small expensive one.',

  distinct_locations:
    'Distinct places with at least one location event in the selected range. Changing the range changes the count, and anywhere you passed through without the game logging it will not appear.',
};

/** Returns the explanation for a metric id, or undefined if none is registered. */
export function explanationFor(metricId: string): string | undefined {
  return INFERENCE_EXPLANATIONS[metricId];
}
