const encoder = new TextEncoder();
export const toBase64 = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes));
export const fromBase64 = (text: string) =>
  Uint8Array.from(atob(text), (c) => c.charCodeAt(0));
export const randomToken = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
export async function hash(value: string): Promise<string> {
  return Array.from(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", encoder.encode(value)),
    ),
    (b) => b.toString(16).padStart(2, "0"),
  ).join("");
}
export function decodeBase58(value: string): Uint8Array {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let result = 0n;
  for (const c of value) {
    const n = alphabet.indexOf(c);
    if (n < 0) throw new Error("Invalid public key");
    result = result * 58n + BigInt(n);
  }
  const bytes: number[] = [];
  while (result > 0n) {
    bytes.unshift(Number(result & 255n));
    result >>= 8n;
  }
  const leading = value.match(/^1*/)?.[0].length ?? 0;
  return new Uint8Array([...new Array(leading).fill(0), ...bytes]);
}
function u32(value: number): Uint8Array {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, value, true);
  return bytes;
}
function string(value: string): Uint8Array {
  const bytes = encoder.encode(value);
  return new Uint8Array([...u32(bytes.length), ...bytes]);
}
/** NEP-413 Borsh payload: tag, message, nonce[32], recipient, callbackUrl=None. */
export function signInPayload(
  message: string,
  nonce: Uint8Array,
  recipient: string,
): Uint8Array {
  if (nonce.length !== 32) throw new Error("Invalid challenge nonce");
  return new Uint8Array([
    ...u32(2 ** 31 + 413),
    ...string(message),
    ...nonce,
    ...string(recipient),
    0,
  ]);
}
export async function verifySignature(
  message: string,
  nonce: string,
  recipient: string,
  publicKey: string,
  signature: string,
): Promise<boolean> {
  if (!publicKey.startsWith("ed25519:")) return false;
  try {
    const bytes = decodeBase58(publicKey.slice(8));
    if (bytes.length !== 32) return false;
    const key = await crypto.subtle.importKey(
      "raw",
      bytes,
      { name: "Ed25519" },
      false,
      ["verify"],
    );
    const digest = await crypto.subtle.digest(
      "SHA-256",
      signInPayload(message, fromBase64(nonce), recipient),
    );
    return await crypto.subtle.verify(
      "Ed25519",
      key,
      fromBase64(signature),
      digest,
    );
  } catch {
    return false;
  }
}
