import { describe, expect, it } from "vitest";
import { formatNear, parseNear, shortlist, validateProfile } from "./shared";
const profile = {
  interests: ["rust", "design"],
  offer: "Code review",
  seek: "Co-founder",
  contact: "me@example.com",
  consent: true,
};
describe("amounts and matchmaking", () => {
  it("preserves decimal amounts without floating-point loss", () => {
    expect(parseNear("1.125")).toBe("1125000000000000000000000");
    expect(formatNear(parseNear("1.125"))).toBe("1.125");
    expect(parseNear("0.000000000000000000000001")).toBe("1");
    for (const value of ["-1", "1e3", "NaN", "0.1234567890123456789012345"])
      expect(() => parseNear(value)).toThrow();
  });
  it("requires consent, bounds fields, and normalizes interests", () => {
    expect(
      validateProfile({ ...profile, interests: [" Rust ", "rust"] }).interests,
    ).toEqual(["rust"]);
    expect(() => validateProfile({ ...profile, consent: false })).toThrow();
    expect(() =>
      validateProfile({ ...profile, contact: "x".repeat(201) }),
    ).toThrow();
    expect(() => validateProfile({ ...profile, interests: [] })).toThrow();
  });
  it("shortlists by real shared interests with deterministic ties", () => {
    const list = shortlist(profile, [
      { account: "c", profile: { ...profile, interests: ["unrelated"] } },
      { account: "b", profile: { ...profile, interests: ["rust"] } },
      { account: "a", profile },
    ]);
    expect(list.map((x) => x.account)).toEqual(["a", "b"]);
  });
});
