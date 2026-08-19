/**
 * Server-side structured logger.
 *
 * pino emits one JSON object per log line to stdout. The OTel collector's
 * filelog receiver tails the container's stdout and forwards to Loki, so
 * every log carries the full structured context (route, user, latency)
 * and can be queried by field — not just regex.
 *
 * In dev, `pino-pretty` reformats the JSON into a colourised human
 * stream; production stays raw JSON for the collector.
 *
 * Server-only — pino is a Node module and must never bundle into the
 * browser.
 */

import 'server-only';
import pino, { type Logger } from 'pino';

const isDev = process.env.NODE_ENV !== 'production';

export const logger: Logger = pino({
  level: process.env.LOG_LEVEL ?? (isDev ? 'debug' : 'info'),
  // Redact common secret fields so accidental log-on-error doesn't
  // leak credentials. The list is conservative; add more if a new
  // secret-bearing field shows up in our wire types.
  redact: {
    paths: [
      'password',
      'token',
      'authorization',
      '*.password',
      '*.token',
      '*.authorization',
      'headers.authorization',
      'headers.cookie',
    ],
    censor: '[redacted]',
  },
  base: {
    service: 'starstats-web',
  },
  ...(isDev && {
    transport: {
      target: 'pino-pretty',
      options: {
        colorize: true,
        translateTime: 'SYS:HH:MM:ss.l',
        ignore: 'pid,hostname,service',
      },
    },
  }),
});
