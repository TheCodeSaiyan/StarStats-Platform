/**
 * Fold a daily series into `n` even buckets for the ring's bar field.
 *
 * The ring reads about 24 bars (measured — see the capacity notes), and 182
 * daily counts drawn as 182 bars is a grey smear. Summing rather than sampling,
 * because a sampled series would silently drop the days between the samples and
 * still look like a complete picture.
 *
 * Returns `[]` for an empty input rather than a row of zeros: no data and no
 * activity are different claims, and the caller switches the ring's mode on the
 * difference.
 *
 * NORMALISED TO 0–100, because that is the scale `Ring` draws against — its bar
 * length is `(v / 100) * 72`. Handing it raw daily counts would have drawn bars
 * far past the ring on a busy account and a flat stub on a quiet one, and
 * nothing would have errored.
 */
export function bucketSeries(values: number[], n: number): number[] {
  if (values.length === 0) return [];
  const raw: number[] = [];
  if (values.length <= n) {
    raw.push(...values);
  } else {
    const size = values.length / n;
    for (let i = 0; i < n; i += 1) {
      const from = Math.floor(i * size);
      const to = Math.floor((i + 1) * size);
      let sum = 0;
      for (let j = from; j < to; j += 1) sum += values[j] ?? 0;
      raw.push(sum);
    }
  }
  const peak = Math.max(...raw, 0);
  if (peak === 0) return [];
  return raw.map((v) => Math.round((v / peak) * 100));
}
