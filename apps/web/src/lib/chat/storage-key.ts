/**
 * The key that encrypts this browser's chat store (matrix-js-sdk's
 * IndexedDB crypto store, via `initRustCrypto({ storageKey })`).
 *
 * The chat DPIA commits web keys to non-extractable WebCrypto keys, so the
 * 32-byte store key is never kept in the clear: it is wrapped (AES-GCM) by
 * a non-extractable key that lives in IndexedDB as a CryptoKey, which page
 * script can use but never read out. Only the wrapped bytes and the IV sit
 * in localStorage. Clearing site data, or signing out, loses both, and with
 * them this browser's chat history, as the DPIA says it will.
 */
const DB = 'ss-chat-keys';
const STORE = 'keys';
const WRAPPED = 'ss.chat.storekey';

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function wrappingKey(userId: string): Promise<CryptoKey> {
  const db = await openDb();
  const existing = await new Promise<CryptoKey | undefined>((resolve, reject) => {
    const r = db.transaction(STORE, 'readonly').objectStore(STORE).get(userId);
    r.onsuccess = () => resolve(r.result as CryptoKey | undefined);
    r.onerror = () => reject(r.error);
  });
  if (existing) return existing;
  const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, [
    'encrypt',
    'decrypt',
  ]);
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).put(key, userId);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
  return key;
}

const b64 = (b: Uint8Array) => btoa(String.fromCharCode(...b));
const unb64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

/** This user's store key, made on first use. */
export async function chatStoreKey(userId: string): Promise<Uint8Array> {
  const wk = await wrappingKey(userId);
  const saved = localStorage.getItem(`${WRAPPED}:${userId}`);
  if (saved) {
    try {
      const { iv, ct } = JSON.parse(saved) as { iv: string; ct: string };
      const raw = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: unb64(iv) }, wk, unb64(ct));
      return new Uint8Array(raw);
    } catch {
      // Wrapped under a key that is gone: start a fresh store below.
    }
  }
  const key = crypto.getRandomValues(new Uint8Array(32));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, wk, key));
  localStorage.setItem(`${WRAPPED}:${userId}`, JSON.stringify({ iv: b64(iv), ct: b64(ct) }));
  return key;
}

/** Forget every chat key and store in this browser (sign-out). */
export function forgetChatStorage(): void {
  try {
    for (const k of Object.keys(localStorage)) {
      if (k.startsWith('ss.chat.')) localStorage.removeItem(k);
    }
  } catch {
    // Storage blocked: nothing was kept.
  }
  try {
    indexedDB.deleteDatabase(DB);
    // matrix-js-sdk's crypto stores are prefixed per user (lib/chat/session).
    void indexedDB.databases?.().then((dbs) => {
      for (const d of dbs) {
        if (d.name?.startsWith('ss-chat-crypto:')) indexedDB.deleteDatabase(d.name);
      }
    });
  } catch {
    // IndexedDB unavailable: nothing was kept.
  }
}
