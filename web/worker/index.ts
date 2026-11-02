import type { Config, Event, Registration, Profile } from "../shared";
import { shortlist, validateProfile } from "../shared";
import { hash, randomToken, toBase64, verifySignature } from "./auth";
export { RetentionAlarm } from "./retention";

type ProfileRow = { account: string; profile: string };
class HttpError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
const fail = (status: number, message: string): never => {
  throw new HttpError(status, message);
};
const json = (
  value: unknown,
  status = 200,
  headers: Record<string, string> = {},
) =>
  Response.json(value, {
    status,
    headers: {
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      ...headers,
    },
  });
const accountPattern = /^(?=.{2,64}$)[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
export function config(env: Env): Config {
  return {
    network: env.NEAR_NETWORK === "mainnet" ? "mainnet" : "testnet",
    contract: env.EVENT_CONTRACT,
    rpc: env.NEAR_RPC,
    configured: Boolean(env.EVENT_CONTRACT),
  };
}
export async function rpc<T>(
  env: Env,
  params: Record<string, unknown>,
): Promise<T> {
  const response = await fetch(env.NEAR_RPC, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: "gather",
      method: "query",
      params,
    }),
    signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok) fail(502, "NEAR is temporarily unavailable. Please retry.");
  const body = (await response.json()) as { result?: T; error?: unknown };
  if (body.error || body.result === undefined)
    fail(502, "Could not verify the latest NEAR state. Please retry.");
  return body.result!;
}
async function view<T>(
  env: Env,
  method_name: string,
  args: unknown,
): Promise<T> {
  if (!env.EVENT_CONTRACT)
    fail(503, "Set EVENT_CONTRACT before using live events.");
  const result = await rpc<{ result: number[] }>(env, {
    request_type: "call_function",
    finality: "final",
    account_id: env.EVENT_CONTRACT,
    method_name,
    args_base64: toBase64(new TextEncoder().encode(JSON.stringify(args))),
  });
  return JSON.parse(
    new TextDecoder().decode(new Uint8Array(result.result)),
  ) as T;
}
async function readBody(request: Request): Promise<Record<string, unknown>> {
  // Bound bytes while streaming: Content-Length alone is not trustworthy.
  const reader = request.body?.getReader();
  if (!reader) return fail(400, "Request body required.");
  const chunks: Uint8Array[] = [];
  let size = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > 16_384) {
      await reader.cancel();
      return fail(413, "Request is too large.");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.length;
  }
  try {
    const data: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (!data || typeof data !== "object" || Array.isArray(data))
      return fail(400, "Expected an object.");
    return data as Record<string, unknown>;
  } catch {
    return fail(400, "Invalid JSON request.");
  }
}
function text(body: Record<string, unknown>, field: string, max = 256): string {
  const value = body[field];
  if (typeof value !== "string" || !value || value.length > max)
    return fail(400, `Invalid ${field}.`);
  return value;
}
async function session(request: Request, env: Env): Promise<string> {
  const token = request.headers
    .get("Cookie")
    ?.match(/(?:^|;\s*)gather_session=([a-f0-9]{64})(?:;|$)/)?.[1];
  if (!token) return fail(401, "Sign a wallet message to continue.");
  const row = await env.DB.prepare(
    "SELECT account FROM sessions WHERE token_hash = ? AND expires > ?",
  )
    .bind(await hash(token), Date.now())
    .first<{ account: string }>();
  return row?.account ?? fail(401, "Your session expired. Sign in again.");
}
async function event(env: Env, id: string): Promise<Event> {
  return (
    (await view<Event | null>(env, "event", { event_id: id })) ??
    fail(404, "Event not found.")
  );
}
async function registered(
  env: Env,
  id: string,
  account: string,
): Promise<Registration> {
  const r = await view<Registration | null>(env, "registration", {
    event_id: id,
    account_id: account,
  });
  if (!r?.active) return fail(403, "A paid active registration is required.");
  return r;
}
function expiry(e: Event): number {
  return Number(e.checkin_closes) + 7 * 86_400_000;
}
async function rateLimit(
  request: Request,
  env: Env,
  account?: string,
  limit = 60,
): Promise<void> {
  const window = Math.floor(Date.now() / 60_000);
  const identity =
    account ?? request.headers.get("CF-Connecting-IP") ?? "local";
  const key = await hash(
    `${identity}:${window}:${new URL(request.url).pathname}`,
  );
  const row = await env.DB.prepare(
    "INSERT INTO rate_limits(key, count, expires) VALUES (?, 1, ?) ON CONFLICT(key) DO UPDATE SET count = count + 1 RETURNING count",
  )
    .bind(key, (window + 2) * 60_000)
    .first<{ count: number }>();
  if ((row?.count ?? limit + 1) > limit)
    fail(429, "Too many attempts. Please wait a minute.");
}
async function oneclick(env: Env, path: string, body?: unknown) {
  const response = await fetch(`https://1click.chaindefuser.com/v0/${path}`, {
    method: body ? "POST" : "GET",
    headers: {
      "Content-Type": "application/json",
      ...(env.ONECLICK_API_KEY ? { "X-API-Key": env.ONECLICK_API_KEY } : {}),
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok)
    fail(
      502,
      "The swap service could not complete this request. Check the amount, source chain, and refund address.",
    );
  return response.json();
}

async function api(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const path = url.pathname;
  if (request.method === "POST" || request.method === "DELETE") {
    if (request.headers.get("Origin") !== url.origin)
      fail(403, "Use the application on its own origin.");
    if (
      request.method === "POST" &&
      !request.headers.get("Content-Type")?.startsWith("application/json")
    )
      fail(415, "Use JSON requests.");
  }
  if (path === "/api/config" && request.method === "GET")
    return json(config(env));
  if (!env.DB) fail(503, "The event service database is not configured.");
  if (path.startsWith("/api/payments/")) {
    if (env.NEAR_NETWORK !== "mainnet")
      fail(
        409,
        "1Click uses mainnet funds. Testnet deposits must use testnet NEAR.",
      );
    const account = await session(request, env);
    await rateLimit(request, env, account, 10);
    if (path === "/api/payments/tokens" && request.method === "GET") {
      const tokens = (await oneclick(env, "tokens")) as {
        assetId: string;
        symbol: string;
        blockchain: string;
        decimals: number;
      }[];
      return json(
        tokens.filter(
          (t) =>
            ["eth", "base", "arb", "sol"].includes(t.blockchain) &&
            ["ETH", "USDC", "SOL"].includes(t.symbol),
        ),
      );
    }
    if (path === "/api/payments/quote" && request.method === "POST") {
      const body = await readBody(request);
      const amount = text(body, "amount", 40);
      const originAsset = text(body, "originAsset");
      if (
        !/^\d+$/.test(amount) ||
        BigInt(amount) <= 0n ||
        BigInt(amount) > 10n ** 30n
      )
        fail(400, "Invalid top-up amount.");
      const tokens = (await oneclick(env, "tokens")) as {
        assetId: string;
        blockchain: string;
        symbol: string;
      }[];
      const source = tokens.find(
        (t) =>
          t.assetId === originAsset &&
          ["eth", "base", "arb", "sol"].includes(t.blockchain) &&
          ["ETH", "USDC", "SOL"].includes(t.symbol),
      );
      if (!source) fail(400, "Unsupported source asset.");
      const refundTo = text(body, "refundTo");
      const quote = (await oneclick(env, "quote", {
        dry: body.dry !== false,
        swapType: "EXACT_OUTPUT",
        slippageTolerance: 100,
        originAsset,
        depositType: "ORIGIN_CHAIN",
        destinationAsset: "nep141:wrap.near",
        amount,
        recipient: account,
        recipientType: "DESTINATION_CHAIN",
        refundTo,
        refundType: "ORIGIN_CHAIN",
        deadline: new Date(Date.now() + 600_000).toISOString(),
      })) as Record<string, unknown>;
      return json({ ...quote, source, recipient: account, refundTo });
    }
    if (path === "/api/payments/status" && request.method === "GET") {
      const address = url.searchParams.get("address");
      const memo = url.searchParams.get("memo");
      if (!address || address.length > 256 || (memo?.length ?? 0) > 256)
        fail(400, "Invalid deposit address.");
      return json(
        await oneclick(
          env,
          `status?depositAddress=${encodeURIComponent(address!)}${memo ? `&depositMemo=${encodeURIComponent(memo)}` : ""}`,
        ),
      );
    }
    fail(405, "Method not allowed.");
  }
  if (path === "/api/auth/challenge" && request.method === "POST") {
    await rateLimit(request, env, undefined, 10);
    await env.RETENTION.getByName("service-cleanup").start();
    const body = await readBody(request);
    const account = text(body, "account");
    if (!accountPattern.test(account)) fail(400, "Invalid NEAR account.");
    const id = randomToken();
    const nonce = toBase64(crypto.getRandomValues(new Uint8Array(32)));
    const expires = Date.now() + 300_000;
    const recipient = url.host;
    const message = `Sign in to Arrivow\nOrigin: ${url.origin}\nNetwork: ${env.NEAR_NETWORK}\nContract: ${env.EVENT_CONTRACT}\nAccount: ${account}\nChallenge: ${id}\nExpires: ${expires}`;
    await env.DB.prepare("INSERT INTO challenges VALUES (?, ?, ?, ?, ?, ?)")
      .bind(id, account, nonce, message, recipient, expires)
      .run();
    return json({ id, nonce, message, recipient, expires });
  }
  if (path === "/api/auth/verify" && request.method === "POST") {
    await rateLimit(request, env, undefined, 15);
    const body = await readBody(request);
    const id = text(body, "id");
    const challenge = await env.DB.prepare(
      "SELECT * FROM challenges WHERE id = ? AND expires > ?",
    )
      .bind(id, Date.now())
      .first<{
        account: string;
        nonce: string;
        message: string;
        recipient: string;
      }>();
    if (
      !challenge ||
      challenge.recipient !== url.host ||
      body.accountId !== challenge.account
    )
      return fail(401, "Challenge expired or account mismatch.");
    const publicKey = text(body, "publicKey");
    const signature = text(body, "signature");
    if (
      !(await verifySignature(
        challenge.message,
        challenge.nonce,
        challenge.recipient,
        publicKey,
        signature,
      ))
    )
      fail(401, "Wallet signature is invalid.");
    const access = await rpc<{ permission: unknown }>(env, {
      request_type: "view_access_key",
      finality: "final",
      account_id: challenge.account,
      public_key: publicKey,
    });
    if (access.permission !== "FullAccess")
      fail(403, "Sign with a full-access wallet key.");
    const consumed = await env.DB.prepare(
      "DELETE FROM challenges WHERE id = ? AND expires > ? RETURNING id",
    )
      .bind(id, Date.now())
      .first();
    if (!consumed) fail(401, "Challenge was already used or expired.");
    const token = randomToken();
    await env.DB.prepare("INSERT INTO sessions VALUES (?, ?, ?)")
      .bind(await hash(token), challenge.account, Date.now() + 3_600_000)
      .run();
    return json({ account: challenge.account }, 200, {
      "Set-Cookie": `gather_session=${token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600${url.protocol === "https:" ? "; Secure" : ""}`,
    });
  }
  if (path === "/api/auth/logout" && request.method === "POST") {
    const token = request.headers
      .get("Cookie")
      ?.match(/gather_session=([a-f0-9]{64})/)?.[1];
    if (token)
      await env.DB.prepare("DELETE FROM sessions WHERE token_hash = ?")
        .bind(await hash(token))
        .run();
    return json({ ok: true }, 200, {
      "Set-Cookie":
        "gather_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
    });
  }
  if (path === "/api/auth/session" && request.method === "GET")
    return json({ account: await session(request, env) });
  const match = path.match(
    /^\/api\/events\/(\d+)\/(ticket|redeem|profile|matches|accept)$/,
  );
  if (!match) return fail(404, "API route not found.");
  const [, id, action] = match;
  const account = await session(request, env);
  await rateLimit(
    request,
    env,
    account,
    action === "matches" && request.method === "POST" ? 3 : 30,
  );
  const e = await event(env, id);
  if (action === "redeem" && request.method === "POST") {
    if (e.organizer !== account)
      fail(403, "Only the organizer can scan tickets.");
    if (
      e.status !== "Open" ||
      Date.now() < Number(e.checkin_opens) ||
      Date.now() >= Number(e.checkin_closes)
    )
      fail(409, "Outside the check-in window.");
    const token = text(await readBody(request), "token");
    const tokenHash = await hash(token);
    const ticket = await env.DB.prepare(
      "SELECT account FROM tickets WHERE token_hash = ? AND event_id = ? AND expires > ?",
    )
      .bind(tokenHash, id, Date.now())
      .first<{ account: string }>();
    if (!ticket)
      return fail(
        409,
        "Ticket expired or already scanned. Ask the guest to refresh it.",
      );
    const r = await registered(env, id, ticket.account);
    if (r.checked_in) fail(409, "This guest is already checked in.");
    const consumed = await env.DB.prepare(
      "DELETE FROM tickets WHERE token_hash = ? AND expires > ? RETURNING account",
    )
      .bind(tokenHash, Date.now())
      .first<{ account: string }>();
    if (!consumed) return fail(409, "Ticket was already scanned.");
    return json({ account: consumed.account });
  }
  if (action === "profile" && request.method === "DELETE") {
    await env.DB.batch([
      env.DB.prepare(
        "DELETE FROM profiles WHERE event_id = ? AND account = ?",
      ).bind(id, account),
      env.DB.prepare(
        "DELETE FROM introductions WHERE event_id = ? AND (a = ? OR b = ?)",
      ).bind(id, account, account),
    ]);
    return json({ ok: true });
  }
  await registered(env, id, account);
  if (action === "ticket" && request.method === "POST") {
    if (e.status !== "Open" || Date.now() >= Number(e.checkin_closes))
      fail(409, "This event is no longer checking in guests.");
    const token = randomToken();
    const expires = Math.min(Date.now() + 60_000, Number(e.checkin_closes));
    await env.DB.prepare(
      "INSERT INTO tickets VALUES (?, ?, ?, ?) ON CONFLICT(event_id, account) DO UPDATE SET token_hash = excluded.token_hash, expires = excluded.expires",
    )
      .bind(await hash(token), id, account, expires)
      .run();
    return json({ token, expires });
  }
  if (expiry(e) <= Date.now())
    fail(410, "Networking data for this event has expired.");
  if (e.status === "Cancelled")
    fail(409, "Networking is unavailable for cancelled events.");
  if (action === "profile") {
    if (request.method === "GET") {
      const row = await env.DB.prepare(
        "SELECT profile FROM profiles WHERE event_id = ? AND account = ? AND expires > ?",
      )
        .bind(id, account, Date.now())
        .first<{ profile: string }>();
      return json({ profile: row ? JSON.parse(row.profile) : null });
    }
    if (request.method === "POST") {
      let profile: Profile;
      try {
        profile = validateProfile(await readBody(request));
      } catch (error) {
        return fail(400, (error as Error).message);
      }
      await env.DB.batch([
        env.DB.prepare(
          "INSERT INTO profiles VALUES (?, ?, ?, ?) ON CONFLICT(event_id, account) DO UPDATE SET profile = excluded.profile, expires = excluded.expires",
        ).bind(id, account, JSON.stringify(profile), expiry(e)),
        // A profile edit revokes past contact consent and removes stale recommendations.
        env.DB.prepare(
          "DELETE FROM introductions WHERE event_id = ? AND (a = ? OR b = ?)",
        ).bind(id, account, account),
      ]);
      return json({ ok: true });
    }
  }
  if (action === "matches" && request.method === "POST")
    return generateMatches(env, id, account, e);
  if (
    (action === "matches" && request.method === "GET") ||
    (action === "accept" && request.method === "POST")
  ) {
    if (action === "accept") {
      const other = text(await readBody(request), "account");
      if (other === account || !accountPattern.test(other))
        fail(400, "Invalid introduction.");
      await registered(env, id, other);
      const [a, b] = [account, other].sort();
      const row = await env.DB.prepare(
        `UPDATE introductions SET ${account === a ? "accepted_a" : "accepted_b"} = 1 WHERE event_id = ? AND a = ? AND b = ? AND expires > ? RETURNING a`,
      )
        .bind(id, a, b, Date.now())
        .first();
      if (!row)
        fail(404, "Introduction not found. Generate recommendations first.");
    }
    return matches(env, id, account);
  }
  return fail(405, "Method not allowed.");
}

async function generateMatches(
  env: Env,
  id: string,
  account: string,
  e: Event,
): Promise<Response> {
  const rows = await env.DB.prepare(
    "SELECT account, profile FROM profiles WHERE event_id = ? AND expires > ? LIMIT 200",
  )
    .bind(id, Date.now())
    .all<ProfileRow>();
  const own = rows.results.find((p) => p.account === account);
  if (!own) return fail(400, "Save your opt-in networking profile first.");
  const active = new Set(e.accounts);
  const candidates = shortlist(
    JSON.parse(own.profile) as Profile,
    rows.results
      .filter((p) => p.account !== account && active.has(p.account))
      .map((p) => ({
        account: p.account,
        profile: JSON.parse(p.profile) as Profile,
      })),
  );
  const selected = candidates.slice(0, 3);
  const ownProfile = JSON.parse(own.profile) as Profile;
  if (selected.length) {
    await env.DB.batch(
      selected.map((candidate) => {
        const [a, b] = [account, candidate.account].sort();
        const reason = `Shared interests: ${candidate.profile.interests.filter((interest) => ownProfile.interests.includes(interest)).join(", ")}.`;
        return env.DB.prepare(
          "INSERT INTO introductions(event_id, a, b, reason, expires) VALUES (?, ?, ?, ?, ?) ON CONFLICT(event_id, a, b) DO NOTHING",
        ).bind(id, a, b, reason, expiry(e));
      }),
    );
  }
  return json({ generated: selected.length });
}

async function matches(
  env: Env,
  id: string,
  account: string,
): Promise<Response> {
  const rows = await env.DB.prepare(
    "SELECT * FROM introductions WHERE event_id = ? AND (a = ? OR b = ?) AND expires > ?",
  )
    .bind(id, account, account, Date.now())
    .all<{
      a: string;
      b: string;
      reason: string;
      accepted_a: number;
      accepted_b: number;
    }>();
  const results = [];
  const e = await event(env, id);
  for (const row of rows.results) {
    const other = row.a === account ? row.b : row.a;
    if (!e.accounts.includes(other)) continue;
    const mutual = Boolean(row.accepted_a && row.accepted_b);
    const profile = mutual
      ? await env.DB.prepare(
          "SELECT profile FROM profiles WHERE event_id = ? AND account = ? AND expires > ?",
        )
          .bind(id, other, Date.now())
          .first<{ profile: string }>()
      : null;
    results.push({
      account: other,
      reason: row.reason,
      accepted: Boolean(row.a === account ? row.accepted_a : row.accepted_b),
      mutual,
      ...(profile
        ? { contact: (JSON.parse(profile.profile) as Profile).contact }
        : {}),
    });
  }
  return json({ matches: results });
}

export default {
  async fetch(request, env): Promise<Response> {
    if (!new URL(request.url).pathname.startsWith("/api/"))
      return env.ASSETS.fetch(request);
    try {
      return await api(request, env);
    } catch (error) {
      if (error instanceof HttpError)
        return json({ error: error.message }, error.status);
      // Never log bodies, cookies, networking profiles, upstream responses, or credentials.
      console.error(
        JSON.stringify({
          message: "Event service request failed",
          kind: error instanceof Error ? error.name : "Unknown",
        }),
      );
      return json(
        {
          error: "The event service is temporarily unavailable. Please retry.",
        },
        503,
      );
    }
  },
} satisfies ExportedHandler<Env>;
