import { describe, expect, it } from "vitest";
import {
  decodeBase58,
  fromBase64,
  signInPayload,
  toBase64,
  verifySignature,
} from "./auth";

function base58(bytes: Uint8Array): string {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let n = BigInt(
    `0x${Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("")}`,
  );
  let text = "";
  while (n) {
    text = alphabet[Number(n % 58n)] + text;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b !== 0) break;
    text = `1${text}`;
  }
  return text;
}
describe("NEP-413 sign-in", () => {
  it("accepts a real signature and binds message, nonce, recipient, and key", async () => {
    const pair = (await crypto.subtle.generateKey("Ed25519", true, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    const publicKey = `ed25519:${base58(new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey)))}`;
    const nonce = crypto.getRandomValues(new Uint8Array(32));
    const digest = await crypto.subtle.digest(
      "SHA-256",
      signInPayload("message", nonce, "localhost"),
    );
    const signature = toBase64(
      new Uint8Array(
        await crypto.subtle.sign("Ed25519", pair.privateKey, digest),
      ),
    );
    expect(
      await verifySignature(
        "message",
        toBase64(nonce),
        "localhost",
        publicKey,
        signature,
      ),
    ).toBe(true);
    expect(
      await verifySignature(
        "other",
        toBase64(nonce),
        "localhost",
        publicKey,
        signature,
      ),
    ).toBe(false);
    expect(
      await verifySignature(
        "message",
        toBase64(nonce),
        "attacker.example",
        publicKey,
        signature,
      ),
    ).toBe(false);
    expect(
      await verifySignature(
        "message",
        toBase64(new Uint8Array(32)),
        "localhost",
        publicKey,
        signature,
      ),
    ).toBe(false);
    expect(
      await verifySignature(
        "message",
        toBase64(nonce),
        "localhost",
        "secp256k1:abc",
        signature,
      ),
    ).toBe(false);
  });
  it("serializes the protocol tag, UTF-8 lengths and empty callback option", () => {
    const data = signInPayload("é", new Uint8Array(32), "x");
    expect(new DataView(data.buffer).getUint32(0, true)).toBe(2 ** 31 + 413);
    expect(new DataView(data.buffer).getUint32(4, true)).toBe(2);
    expect(data.at(-1)).toBe(0);
    expect(data.length).toBe(48);
  });
  it("handles base58 leading zeros and rejects malformed encodings", () => {
    expect(decodeBase58("1112")).toEqual(new Uint8Array([0, 0, 0, 1]));
    expect(() => decodeBase58("0OIl")).toThrow();
    expect(fromBase64(toBase64(new Uint8Array([0, 255])))).toEqual(
      new Uint8Array([0, 255]),
    );
  });
});
