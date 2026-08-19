export interface FriendlyError {
  title: string;
  body: string;
  hint?: string;
}

const MAX_BODY_LENGTH = 200;

function raw(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

function trim(s: string): string {
  return s.length > MAX_BODY_LENGTH ? s.slice(0, MAX_BODY_LENGTH) + '…' : s;
}

export function friendlyError(err: unknown): FriendlyError {
  const message = raw(err);
  const lower = message.toLowerCase();

  if (/timeout|timed out/.test(lower)) {
    return {
      title: 'Timed out',
      body: "The server didn't respond in time.",
      hint: 'Try again, or check your connection.',
    };
  }
  if (/connection refused|dns|network/.test(lower)) {
    return {
      title: "Couldn't connect",
      body: "Couldn't reach the server.",
      hint: 'Check the API URL or your internet.',
    };
  }
  if (/\b401\b|\b403\b|unauthori[sz]ed/.test(lower)) {
    return {
      title: 'Rejected',
      body: 'The server rejected this device.',
      hint: 'You may need to re-pair.',
    };
  }
  if (/\b404\b|not found/.test(lower)) {
    return {
      title: 'Endpoint not found',
      body: 'The server is up but the endpoint is missing.',
      hint: 'Check the API URL — is it pointing at the right server?',
    };
  }
  if (/\b5\d\d\b/.test(lower)) {
    return {
      title: 'Server error',
      body: 'The server is having problems.',
      hint: 'Try again in a moment.',
    };
  }
  if (/rsi[-_ ]?token|rsi cookie|rsi rejected the cookie|no cookie set/i.test(message)) {
    return {
      title: 'No RSI cookie',
      body: 'Paste your RSI cookie to enable hangar sync.',
    };
  }
  return {
    title: 'Something went wrong',
    body: trim(message),
    hint: 'Enable Debug logging in Settings to capture details.',
  };
}
