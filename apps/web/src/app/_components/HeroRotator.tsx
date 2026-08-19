'use client';

import { useEffect, useState } from 'react';

const HERO_WORDS: ReadonlyArray<{ text: string; kind: 'brand' | 'feature' }> = [
  { text: 'StarStats.', kind: 'brand' },
  { text: 'Your manifest.', kind: 'feature' },
  { text: 'Your numbers.', kind: 'feature' },
  { text: 'Your timeline.', kind: 'feature' },
];

type Phase = 'in' | 'out';

/**
 * Hero word rotator — small client island used by the marketing landing.
 * Drives the `.hero-word` keyframes (`heroIn` / `heroOut`) defined in
 * starstats-tokens.css. CSS handles the visual sweep + tracking ease;
 * this only flips `data-state` and bumps the index.
 *
 * Honours `prefers-reduced-motion` by pinning to the first word: the
 * CSS sweep would already be flattened by the global reduced-motion
 * rule, but the content swap itself is decorative and would still
 * register as motion to a reader using a screen reader / refresh-driven
 * UI. Pinning kills both signals.
 */
export function HeroRotator() {
  const [index, setIndex] = useState(0);
  const [phase, setPhase] = useState<Phase>('in');
  const [reducedMotion, setReducedMotion] = useState(false);

  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const apply = () => setReducedMotion(mq.matches);
    apply();
    mq.addEventListener('change', apply);
    return () => mq.removeEventListener('change', apply);
  }, []);

  useEffect(() => {
    if (reducedMotion) return;
    if (phase !== 'in') return;
    const t = setTimeout(() => setPhase('out'), 2400);
    return () => clearTimeout(t);
  }, [phase, index, reducedMotion]);

  useEffect(() => {
    if (reducedMotion) return;
    if (phase !== 'out') return;
    const t = setTimeout(() => {
      setIndex((i) => (i + 1) % HERO_WORDS.length);
      setPhase('in');
    }, 480);
    return () => clearTimeout(t);
  }, [phase, reducedMotion]);

  const word = HERO_WORDS[index];
  return (
    <span className="hero-rotator" style={{ minWidth: '11ch' }}>
      <span
        key={`${index}-${phase}`}
        className="hero-word"
        data-state={phase}
        data-kind={word.kind}
      >
        {word.text}
      </span>
    </span>
  );
}
