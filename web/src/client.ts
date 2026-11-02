import { NearConnector } from "@hot-labs/near-connect";
import type { Config, Event, Registration } from "../shared";

export async function api<T>(
  path: string,
  body?: unknown,
  method = body === undefined ? "GET" : "POST",
): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method,
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const result = (await response.json()) as T & { error?: string };
  if (!response.ok)
    throw new Error(
      result.error ?? "The service is unavailable. Please retry.",
    );
  return result;
}
export async function view<T>(
  config: Config,
  method_name: string,
  args: unknown,
): Promise<T> {
  if (!config.configured)
    throw new Error("No event contract is configured yet.");
  const response = await fetch(config.rpc, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: "gather",
      method: "query",
      params: {
        request_type: "call_function",
        finality: "final",
        account_id: config.contract,
        method_name,
        args_base64: btoa(
          String.fromCharCode(
            ...new TextEncoder().encode(JSON.stringify(args)),
          ),
        ),
      },
    }),
    signal: AbortSignal.timeout(15_000),
  });
  const result = (await response.json()) as {
    error?: unknown;
    result?: { result: number[] };
  };
  if (!response.ok || result.error || !result.result)
    throw new Error(
      "Could not read the event contract. Check the network and retry.",
    );
  return JSON.parse(
    new TextDecoder().decode(new Uint8Array(result.result.result)),
  ) as T;
}
export function connector(config: Config) {
  return new NearConnector({
    network: config.network,
    features: { signMessage: true },
    footerBranding: null,
  });
}
export async function authenticate(wallet: NearConnector, account: string) {
  try {
    const current = await api<{ account: string }>("/auth/session");
    if (current.account === account) return;
  } catch {
    /* Request a fresh signed session. */
  }
  const challenge = await api<{
    id: string;
    nonce: string;
    message: string;
    recipient: string;
  }>("/auth/challenge", { account });
  const signed = await (
    await wallet.wallet()
  ).signMessage({
    message: challenge.message,
    recipient: challenge.recipient,
    nonce: Uint8Array.from(atob(challenge.nonce), (c) => c.charCodeAt(0)),
    signerId: account,
  });
  await api("/auth/verify", { id: challenge.id, ...signed });
}
export async function transact(
  config: Config,
  wallet: NearConnector,
  account: string,
  methodName: string,
  args: unknown,
  deposit = "0",
) {
  if (!config.configured)
    throw new Error("Configure the event contract first.");
  const pending = {
    network: config.network,
    contract: config.contract,
    account,
    methodName,
    args,
    at: Date.now(),
  };
  // Persist before opening a wallet, which may redirect away from this page.
  sessionStorage.setItem("gather:pending", JSON.stringify(pending));
  const result = await (
    await wallet.wallet()
  ).signAndSendTransaction({
    signerId: account,
    receiverId: config.contract,
    network: config.network,
    actions: [
      {
        type: "FunctionCall",
        params: { methodName, args, gas: "100000000000000", deposit },
      },
    ],
  });
  if (typeof result.status === "object" && "Failure" in result.status) {
    sessionStorage.removeItem("gather:pending");
    throw new Error(
      "The contract rejected this transaction. Check its receipt and refresh the event.",
    );
  }
  sessionStorage.removeItem("gather:pending");
  return result;
}
export async function allEvents(config: Config): Promise<[string, Event][]> {
  const list: [string, Event][] = [];
  for (let from = 0; ; from += 50) {
    const page = await view<[string, Event][]>(config, "events", {
      from_index: from,
      limit: 50,
    });
    list.push(...page);
    if (page.length < 50) return list;
    // Keep the browse request bounded; older pages can be loaded in another request.
    if (list.length >= 500) return list;
  }
}
export const registration = (
  config: Config,
  event_id: string,
  account_id: string,
) =>
  view<Registration | null>(config, "registration", { event_id, account_id });
