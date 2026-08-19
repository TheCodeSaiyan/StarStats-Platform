'use client';

/**
 * Signature StarStats background — canvas-rendered "quantum warp" streak
 * field. 90 directional particles with depth-driven speed / length /
 * width / alpha; 65% accent-tinted (read live from `--accent` so theme
 * swaps propagate without remount), 35% near-white. Smoothly tweens
 * toward a target angle on prop change (~25 frames to settle) so each
 * route can carry its own flow direction.
 *
 * Ported from the original design canvas from
 * the design canvas — the file shipped to /apps/web until
 * now was a CSS-only stub with `.ss-bg-warp` + `.ss-bg-pulse` ribbons,
 * which is a different visual family. This brings the production site
 * back in line with the design plan.
 */

import { useEffect, useRef } from 'react';

interface QuantumWarpProps {
  /** Direction the streaks travel, in degrees. 180 = right-to-left
   *  (default leftward warp), 90 = top-to-bottom, etc. Prop changes
   *  smoothly tween toward the new target rather than snapping. */
  angle?: number;
}

interface Particle {
  x: number;
  y: number;
  depth: number;
  speed: number;
  len: number;
  /** 1 = accent-tinted, 0 = near-white */
  tint: 0 | 1;
}

interface WarpState {
  particles: Particle[];
  w: number;
  h: number;
  accent: [number, number, number];
  /** Current angle, radians. Tweens toward `targetAngle`. */
  angle: number;
  /** Target angle, radians. Updated when the prop changes. */
  targetAngle: number;
}

const COUNT = 90;
const DEFAULT_ACCENT: [number, number, number] = [232, 162, 60]; // #E8A23C

function readAccent(): [number, number, number] {
  const root = document.documentElement;
  const v = getComputedStyle(root).getPropertyValue('--accent').trim();
  if (!v.startsWith('#')) return DEFAULT_ACCENT;
  let h = v.slice(1);
  if (h.length === 3) h = h.split('').map(c => c + c).join('');
  if (h.length !== 6) return DEFAULT_ACCENT;
  const n = parseInt(h, 16);
  if (Number.isNaN(n)) return DEFAULT_ACCENT;
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function spawnParticle(p: Particle, w: number, h: number, fresh: boolean): void {
  p.depth = Math.random();
  // Scattered on first init; otherwise drop just off the entry edge —
  // `respawnEdge` repositions immediately for non-fresh spawns based on
  // the current direction vector.
  p.x = fresh ? Math.random() * w : -100;
  p.y = fresh ? Math.random() * h : Math.random() * h;
  p.speed = 0.4 + p.depth * p.depth * 6.0;
  p.len = 18 + p.depth * 140;
  p.tint = Math.random() < 0.65 ? 1 : 0;
}

function respawnEdge(p: Particle, dx: number, dy: number, w: number, h: number): void {
  const margin = 40;
  const total = Math.abs(dx) + Math.abs(dy);
  // Bias entry edge by direction: more horizontal motion → spawn on
  // vertical edges; more vertical motion → horizontal edges.
  if (Math.random() * total < Math.abs(dx)) {
    p.x = dx > 0 ? -margin - Math.random() * 60 : w + margin + Math.random() * 60;
    p.y = Math.random() * h;
  } else {
    p.y = dy > 0 ? -margin - Math.random() * 60 : h + margin + Math.random() * 60;
    p.x = Math.random() * w;
  }
}

export function QuantumWarp({ angle = 180 }: QuantumWarpProps = {}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stateRef = useRef<WarpState>({
    particles: [],
    w: 0,
    h: 0,
    accent: DEFAULT_ACCENT,
    angle: (angle * Math.PI) / 180,
    targetAngle: (angle * Math.PI) / 180,
  });
  // Handle to the running RAF loop's tick() — set inside the main
  // useEffect below. Lets the [angle] effect stamp a few extra frames
  // under prefers-reduced-motion, where the loop auto-stops at mount
  // and would otherwise miss any subsequent angle prop changes.
  const tickRef = useRef<(() => void) | null>(null);

  // Update target on prop change so the angle tweens smoothly rather
  // than jumping when a route navigates and supplies a different angle.
  // The diagnostic console.debug confirms this effect actually fires
  // when navigating between screens. If you don't see it but direction
  // stays stuck, the bug is upstream (prop plumbing or a remount).
  // Filter Devtools console on "QuantumWarp" if it gets noisy.
  useEffect(() => {
    const s = stateRef.current;
    const target = (angle * Math.PI) / 180;
    // eslint-disable-next-line no-console
    console.debug('[QuantumWarp] angle →', angle, '(rad', target.toFixed(3), ')');
    s.targetAngle = target;

    // Under `prefers-reduced-motion: reduce`, the main RAF loop ticked
    // 12 frames at mount and stopped — subsequent prop changes would
    // mutate targetAngle invisibly and never repaint. Snap angle to
    // target (no tween — the user opted out of motion) and stamp a
    // handful of frames via the tick handle so the new direction
    // actually renders to canvas.
    if (
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    ) {
      s.angle = target;
      if (tickRef.current) {
        // 8 frames is enough to (a) clearRect the previous direction's
        // gradients and (b) advance particles a few steps along the
        // new vector so the change is unambiguous.
        for (let i = 0; i < 8; i++) tickRef.current();
      }
    }
  }, [angle]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d', { alpha: true });
    if (!ctx) return;

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    let raf = 0;
    let running = true;

    const init = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const w = (stateRef.current.w = canvas.clientWidth);
      const h = (stateRef.current.h = canvas.clientHeight);
      canvas.width = Math.floor(w * dpr);
      canvas.height = Math.floor(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      if (stateRef.current.particles.length === 0) {
        stateRef.current.particles = Array.from({ length: COUNT }, () => {
          const p: Particle = { x: 0, y: 0, depth: 0, speed: 0, len: 0, tint: 0 };
          spawnParticle(p, w, h, true);
          return p;
        });
      }
    };

    const tick = () => {
      if (!running) return;
      const s = stateRef.current;
      const { particles, w, h, accent } = s;

      // Tween angle along the shortest path (wrap into [-π, π]).
      let diff = s.targetAngle - s.angle;
      while (diff > Math.PI) diff -= Math.PI * 2;
      while (diff < -Math.PI) diff += Math.PI * 2;
      // 8%/frame → ~12 frames to settle (~200ms @ 60fps). Bumped
      // from 0.04 because the slower rate was visually indistinguishable
      // from "didn't change" given particle inertia — existing
      // streaks keep flying along their old vector until they leave
      // the screen, so the new direction needs to assert itself
      // quickly to be perceptible within the ~500ms expected.
      s.angle += diff * 0.08;

      const dx = Math.cos(s.angle);
      const dy = Math.sin(s.angle);

      ctx.clearRect(0, 0, w, h);
      ctx.globalCompositeOperation = 'lighter';
      ctx.lineCap = 'round';

      const [ar, ag, ab] = accent;

      for (let i = 0; i < particles.length; i++) {
        const p = particles[i];
        p.x += dx * p.speed;
        p.y += dy * p.speed;

        // Recycle particles that have drifted off-screen on the exit
        // side: respawn at the opposite edge so the field stays full.
        if (p.x < -200 || p.x > w + 200 || p.y < -200 || p.y > h + 200) {
          spawnParticle(p, w, h, false);
          respawnEdge(p, dx, dy, w, h);
          continue;
        }

        const r = p.tint ? ar : 235;
        const g = p.tint ? ag : 235;
        const b = p.tint ? ab : 245;

        const baseAlpha = 0.05 + p.depth * 0.55;
        const width = 0.4 + p.depth * 1.6;

        // Tail extends opposite the motion vector.
        const tailX = p.x - dx * p.len;
        const tailY = p.y - dy * p.len;

        const grad = ctx.createLinearGradient(p.x, p.y, tailX, tailY);
        grad.addColorStop(0, `rgba(${r},${g},${b},${baseAlpha})`);
        grad.addColorStop(0.6, `rgba(${r},${g},${b},${baseAlpha * 0.4})`);
        grad.addColorStop(1, `rgba(${r},${g},${b},0)`);

        ctx.strokeStyle = grad;
        ctx.lineWidth = width;
        ctx.beginPath();
        ctx.moveTo(p.x, p.y);
        ctx.lineTo(tailX, tailY);
        ctx.stroke();

        // Bright "head" dot on near-foreground particles — gives the
        // field a sense of depth and makes the warp feel less flat.
        if (p.depth > 0.7) {
          const headA = (p.depth - 0.7) * 2.2;
          ctx.fillStyle = `rgba(${Math.min(255, r + 30)},${Math.min(255, g + 30)},${Math.min(255, b + 30)},${headA * baseAlpha * 1.4})`;
          ctx.beginPath();
          ctx.arc(p.x, p.y, width * 1.1, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      if (!reduced) raf = requestAnimationFrame(tick);
    };

    // Expose tick() to the [angle] effect's reduced-motion branch so
    // it can stamp a fresh batch of frames after a prop change. Set
    // back to null in cleanup so the closure can be GC'd if React
    // remounts (StrictMode double-invoke in dev, etc).
    tickRef.current = tick;

    stateRef.current.accent = readAccent();
    init();
    if (reduced) {
      // Under reduced-motion preference, render a small handful of
      // frames so the field looks "set" rather than empty, then stop.
      for (let i = 0; i < 12; i++) tick();
    } else {
      raf = requestAnimationFrame(tick);
    }

    const onResize = () => init();
    window.addEventListener('resize', onResize);

    // Re-read the accent every 600ms so a theme change (e.g. user
    // switches accent in Settings) propagates without a remount.
    const accentInterval = window.setInterval(() => {
      stateRef.current.accent = readAccent();
    }, 600);

    return () => {
      running = false;
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', onResize);
      window.clearInterval(accentInterval);
      tickRef.current = null;
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      className="ss-bg-warp-canvas"
      aria-hidden="true"
      style={{
        position: 'fixed',
        inset: 0,
        width: '100%',
        height: '100%',
        pointerEvents: 'none',
        zIndex: 0,
        opacity: 0.65,
      }}
    />
  );
}
