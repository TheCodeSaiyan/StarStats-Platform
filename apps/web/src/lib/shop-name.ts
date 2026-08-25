/**
 * Raw shop identifiers, made readable.
 *
 * `shop_buy_request` carries the engine's own name — `SCShop_NoodleBar_A_Food_RestStop`
 * — and that string reached the screen twice on the Commerce lens: as the
 * "Top shop" figure in the pane and again in the core readout's detail line.
 * A raw class id is not a shop name, and this one is a single unbroken token,
 * so it also overflowed the pane and grew a horizontal scrollbar behind it.
 *
 * The transform already existed, copied verbatim into `spend.tsx` and
 * `economy.tsx`. It lives here now so the projection uses the SAME rule as
 * the flat widgets rather than a third interpretation — the projection
 * re-deriving what a widget already computes is the exact fault that put raw
 * `system|planet|city` keys on the Travel lens.
 */
export function prettyShop(raw: string): string {
  return raw.replace(/^SCShop[_-]?/i, '').replace(/_/g, ' ').trim() || raw;
}
