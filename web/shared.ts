export interface Event {
  organizer: string;
  title: string;
  description: string;
  location: string;
  capacity: number;
  deposit: string;
  bonus_bps: number;
  registration_closes: string;
  checkin_opens: string;
  checkin_closes: string;
  status: "Open" | "Cancelled" | "Settled";
  registered: number;
  attended: number;
  attendee_payout: string;
  host_claimable: string;
  host_pending: string;
  accounts: string[];
}
export interface Registration {
  active: boolean;
  checked_in: boolean;
  claimed: boolean;
  credit: string;
  pending: string;
}
export interface Config {
  network: "testnet" | "mainnet";
  contract: string;
  rpc: string;
  configured: boolean;
}
export interface Profile {
  interests: string[];
  offer: string;
  seek: string;
  contact: string;
  consent: boolean;
}
export interface Match {
  account: string;
  reason: string;
  accepted: boolean;
  mutual: boolean;
  contact?: string;
}
export function parseNear(value: string): string {
  if (!/^\d+(\.\d{1,24})?$/.test(value))
    throw new Error(
      "Enter a positive NEAR amount with at most 24 decimal places.",
    );
  const [whole, fraction = ""] = value.split(".");
  return (
    BigInt(whole) * 10n ** 24n +
    BigInt(fraction.padEnd(24, "0"))
  ).toString();
}
export function formatNear(value: string): string {
  const amount = BigInt(value);
  const unit = 10n ** 24n;
  const fraction = (amount % unit)
    .toString()
    .padStart(24, "0")
    .slice(0, 5)
    .replace(/0+$/, "");
  return `${amount / unit}${fraction ? `.${fraction}` : ""}`;
}
export function validateProfile(input: unknown): Profile {
  if (!input || typeof input !== "object")
    throw new Error("Provide a networking profile.");
  const p = input as Record<string, unknown>;
  if (p.consent !== true)
    throw new Error("Explicit consent is required for matchmaking.");
  if (
    !Array.isArray(p.interests) ||
    p.interests.length < 1 ||
    p.interests.length > 10 ||
    !p.interests.every(
      (x) => typeof x === "string" && x.trim().length > 0 && x.length <= 40,
    )
  )
    throw new Error("Choose 1–10 interests, at most 40 characters each.");
  for (const field of ["offer", "seek", "contact"]) {
    if (
      typeof p[field] !== "string" ||
      (p[field] as string).length > (field === "contact" ? 200 : 500)
    )
      throw new Error(`${field} is too long or missing.`);
  }
  return {
    interests: [
      ...new Set((p.interests as string[]).map((s) => s.trim().toLowerCase())),
    ],
    offer: (p.offer as string).trim(),
    seek: (p.seek as string).trim(),
    contact: (p.contact as string).trim(),
    consent: true,
  };
}
export function shortlist<T extends { account: string; profile: Profile }>(
  own: Profile,
  candidates: T[],
): T[] {
  return candidates
    .map((candidate) => ({
      candidate,
      score: candidate.profile.interests.filter((i) =>
        own.interests.includes(i),
      ).length,
    }))
    .filter((x) => x.score > 0)
    .sort(
      (a, b) =>
        b.score - a.score ||
        a.candidate.account.localeCompare(b.candidate.account),
    )
    .slice(0, 8)
    .map((x) => x.candidate);
}
