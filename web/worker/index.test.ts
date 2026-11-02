import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import worker from "./index";
vi.mock("./retention", () => ({ RetentionAlarm: class {} }));
import { hash, signInPayload, toBase64 } from "./auth";

// Real SQLite executes the D1 SQL, including atomic RETURNING and conflict handling.
function database() {
  const sqlite = new DatabaseSync(":memory:");
  sqlite.exec(
    readFileSync(
      new URL("../migrations/0001_service.sql", import.meta.url),
      "utf8",
    ),
  );
  function prepare(sql: string, args: (string | number | null)[] = []) {
    return {
      bind: (...values: (string | number | null)[]) => prepare(sql, values),
      first: async () => sqlite.prepare(sql).get(...args) ?? null,
      all: async () => ({ results: sqlite.prepare(sql).all(...args) }),
      run: async () => sqlite.prepare(sql).run(...args),
    };
  }
  return {
    sqlite,
    binding: {
      prepare,
      batch: async (statements: { run(): Promise<unknown> }[]) => {
        sqlite.exec("BEGIN");
        try {
          const results = [];
          for (const s of statements) results.push(await s.run());
          sqlite.exec("COMMIT");
          return results;
        } catch (error) {
          sqlite.exec("ROLLBACK");
          throw error;
        }
      },
    } as D1Database,
  };
}
const context = {
  waitUntil() {},
  passThroughOnException() {},
} as ExecutionContext;
let db: ReturnType<typeof database>;
let env: Env;
let event: Record<string, unknown>;
let active = true;
async function request(
  path: string,
  body?: unknown,
  cookie = "",
  method = body === undefined ? "GET" : "POST",
) {
  return worker.fetch(
    new Request(`https://gather.example/api${path}`, {
      method,
      headers: {
        Origin: "https://gather.example",
        "Content-Type": "application/json",
        Cookie: cookie,
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    }),
    env,
    context,
  );
}
async function login(account: string) {
  const token =
    "a".repeat(62) +
    (account.startsWith("host")
      ? "01"
      : account.startsWith("other")
        ? "02"
        : "03");
  await db.binding
    .prepare("INSERT OR REPLACE INTO sessions VALUES (?, ?, ?)")
    .bind(await hash(token), account, Date.now() + 60_000)
    .run();
  return `gather_session=${token}`;
}
beforeEach(() => {
  db = database();
  env = {
    RETENTION: {
      getByName: () => ({ start: async () => {} }),
    } as DurableObjectNamespace,
    DB: db.binding,
    EVENT_CONTRACT: "",
    NEAR_NETWORK: "testnet",
    NEAR_RPC: "https://rpc.testnet.near.org",
    ASSETS: { fetch: async () => new Response("asset") } as Fetcher,
  };
  // The generated environment narrows configured vars to their checked-in values.
  Object.assign(env, {
    EVENT_CONTRACT: "events.testnet",
  });
  active = true;
  event = {
    organizer: "host.testnet",
    status: "Open",
    checkin_opens: String(Date.now() - 10000),
    checkin_closes: String(Date.now() + 60_000),
    accounts: ["guest.testnet", "other.testnet"],
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string | Request, init?: RequestInit) => {
      const body = JSON.parse(String(init?.body));
      const params = body.params;
      if (params.request_type === "view_access_key")
        return Response.json({ result: { permission: "FullAccess" } });
      const output =
        params.method_name === "event" ? event : { active, checked_in: false };
      return Response.json({
        result: {
          result: [...new TextEncoder().encode(JSON.stringify(output))],
        },
      });
    }),
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
  db.sqlite.close();
});

describe("event service boundaries", () => {
  it("requires same-origin mutations and a signed session", async () => {
    expect(
      (
        await worker.fetch(
          new Request("https://gather.example/api/auth/challenge", {
            method: "POST",
            body: "{}",
          }),
          env,
          context,
        )
      ).status,
    ).toBe(403);
    expect((await request("/events/0/ticket", {})).status).toBe(401);
  });
  it("accepts NEP-413 once and rejects replay and expiry", async () => {
    const challenge = (await (
      await request("/auth/challenge", { account: "guest.testnet" })
    ).json()) as {
      id: string;
      nonce: string;
      message: string;
      recipient: string;
    };
    const pair = (await crypto.subtle.generateKey("Ed25519", true, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    const bytes = new Uint8Array(
      (await crypto.subtle.exportKey("raw", pair.publicKey)) as ArrayBuffer,
    );
    const alphabet =
      "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let n = BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
    let key = "";
    while (n) {
      key = alphabet[Number(n % 58n)] + key;
      n /= 58n;
    }
    for (const b of bytes) {
      if (b !== 0) break;
      key = `1${key}`;
    }
    const digest = await crypto.subtle.digest(
      "SHA-256",
      signInPayload(
        challenge.message,
        Uint8Array.from(atob(challenge.nonce), (c) => c.charCodeAt(0)),
        challenge.recipient,
      ),
    );
    const signature = toBase64(
      new Uint8Array(
        await crypto.subtle.sign("Ed25519", pair.privateKey, digest),
      ),
    );
    const payload = {
      id: challenge.id,
      accountId: "guest.testnet",
      publicKey: `ed25519:${key}`,
      signature,
    };
    const result = await request("/auth/verify", payload);
    expect(result.status).toBe(200);
    expect(result.headers.get("Set-Cookie")).toContain("HttpOnly");
    expect((await request("/auth/verify", payload)).status).toBe(401);
    const expired = (await (
      await request("/auth/challenge", { account: "guest.testnet" })
    ).json()) as { id: string };
    db.sqlite
      .prepare("UPDATE challenges SET expires=0 WHERE id=?")
      .run(expired.id);
    expect(
      (await request("/auth/verify", { ...payload, id: expired.id })).status,
    ).toBe(401);
  });
  it("checks registration, host identity, ticket expiry, and atomic consumption", async () => {
    const guest = await login("guest.testnet");
    const host = await login("host.testnet");
    const ticket = (await (
      await request("/events/0/ticket", {}, guest)
    ).json()) as { token: string };
    expect(
      (await request("/events/0/redeem", { token: ticket.token }, guest))
        .status,
    ).toBe(403);
    expect(
      (await request("/events/0/redeem", { token: ticket.token }, host)).status,
    ).toBe(200);
    expect(
      (await request("/events/0/redeem", { token: ticket.token }, host)).status,
    ).toBe(409);
    const second = (await (
      await request("/events/0/ticket", {}, guest)
    ).json()) as { token: string };
    db.sqlite.prepare("UPDATE tickets SET expires=0").run();
    expect(
      (await request("/events/0/redeem", { token: second.token }, host)).status,
    ).toBe(409);
    active = false;
    expect((await request("/events/0/ticket", {}, guest)).status).toBe(403);
  });
  it("withholds contact until mutual acceptance, revokes it on profile edits", async () => {
    const guest = await login("guest.testnet");
    const other = await login("other.testnet");
    const p = {
      interests: ["rust"],
      offer: "Build",
      seek: "Friends",
      contact: "private@example.com",
      consent: true,
    };
    expect((await request("/events/0/profile", p, guest)).status).toBe(200);
    await request("/events/0/profile", p, other);
    expect((await request("/events/0/matches", {}, guest)).status).toBe(200);
    const first = await (
      await request("/events/0/accept", { account: "other.testnet" }, guest)
    ).text();
    expect(first).not.toContain("private@example.com");
    const mutual = await (
      await request("/events/0/accept", { account: "guest.testnet" }, other)
    ).text();
    expect(mutual).toContain("private@example.com");
    await request(
      "/events/0/profile",
      { ...p, contact: "new@example.com" },
      guest,
    );
    expect(
      await (await request("/events/0/matches", undefined, other)).text(),
    ).not.toContain("@example.com");
  });
  it("blocks expired profile access without affecting tickets", async () => {
    const guest = await login("guest.testnet");
    const other = await login("other.testnet");
    const p = {
      interests: ["rust"],
      offer: "Build",
      seek: "Friends",
      contact: "",
      consent: true,
    };
    await request("/events/0/profile", p, guest);
    await request("/events/0/profile", p, other);
    expect((await request("/events/0/ticket", {}, guest)).status).toBe(200);
    event.checkin_closes = String(Date.now() - 8 * 86_400_000);
    expect((await request("/events/0/profile", undefined, guest)).status).toBe(
      410,
    );
  });
  it("blocks 1Click mainnet payments in testnet mode", async () => {
    const guest = await login("guest.testnet");
    expect(
      (await request("/payments/quote", { dry: false }, guest)).status,
    ).toBe(409);
  });
  it("allows removal of a profile after cancelling registration", async () => {
    const guest = await login("guest.testnet");
    await request(
      "/events/0/profile",
      {
        interests: ["rust"],
        offer: "",
        seek: "",
        contact: "private@example.com",
        consent: true,
      },
      guest,
    );
    active = false;
    expect(
      (await request("/events/0/profile", undefined, guest, "DELETE")).status,
    ).toBe(200);
    expect(
      db.sqlite.prepare("SELECT COUNT(*) AS n FROM profiles").get()?.n,
    ).toBe(0);
  });
  it("calculates shared-interest matches locally", async () => {
    const guest = await login("guest.testnet");
    const other = await login("other.testnet");
    const p = {
      interests: ["rust"],
      offer: "",
      seek: "",
      contact: "private@example.com",
      consent: true,
    };
    await request("/events/0/profile", p, guest);
    await request("/events/0/profile", p, other);
    const response = await request("/events/0/matches", {}, guest);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ generated: 1 });
    expect(
      vi.mocked(fetch).mock.calls.some(([url]) => String(url) !== env.NEAR_RPC),
    ).toBe(false);
    const accepted = await request(
      "/events/0/accept",
      { account: "other.testnet" },
      guest,
    );
    expect(await accepted.text()).not.toContain(p.contact);
    const mutual = await request(
      "/events/0/accept",
      { account: "guest.testnet" },
      other,
    );
    expect(await mutual.text()).toContain(p.contact);
  });
  it("binds quotes to the authenticated recipient and preserves refund status", async () => {
    env.NEAR_NETWORK = "mainnet";
    const guest = await login("guest.testnet");
    const token = { assetId: "base-usdc", symbol: "USDC", blockchain: "base" };
    let quoted: Record<string, unknown> = {};
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string, init?: RequestInit) => {
        if (input.endsWith("/tokens")) return Response.json([token]);
        if (input.endsWith("/quote")) {
          quoted = JSON.parse(String(init?.body));
          return Response.json({
            quote: { amountInFormatted: "4.5", amountOutFormatted: "1" },
          });
        }
        return Response.json({ status: "REFUNDED" });
      }),
    );
    const response = await request(
      "/payments/quote",
      {
        originAsset: token.assetId,
        amount: "1000000000000000000000000",
        refundTo: "0xrefund",
        recipient: "attacker.near",
      },
      guest,
    );
    expect(response.status).toBe(200);
    expect(quoted.recipient).toBe("guest.testnet");
    expect(quoted.dry).toBe(true);
    expect(((await response.json()) as { source: unknown }).source).toEqual(
      token,
    );
    const status = await request(
      "/payments/status?address=original-deposit",
      undefined,
      guest,
    );
    expect(await status.json()).toEqual({ status: "REFUNDED" });
  });
});
