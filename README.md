# Arrivow

**Conference sessions and side events where showing up pays back.**

Guests commit a refundable NEAR deposit. After the organizer checks them in and the event settles, they claim their deposit plus a published share of no-show deposits. The organizer receives the remainder. Optional private introductions help guests find people with shared interests.

[Open the testnet app](https://gather-events.velikanghost.workers.dev) · [Inspect the event contract](https://testnet.nearblocks.io/address/emwe02ki0.faintprint-funder-7k2q.testnet)

This is a hackathon prototype, not an audited production escrow. It supports **200 active registrations per event**, including conference sessions and side events rather than an entire large conference.

## NEAR tools and their jobs

| Tool | What Arrivow uses it for |
|---|---|
| NEAR smart contract / near-sdk | Deposits, immutable event terms, attendance, refunds, bonus distribution, recoverable claims |
| NEAR Connect | Wallet connection, transaction confirmation, NEP-413 sign-in |
| NEAR Intents / 1Click | Cross-chain wallet funding; an explicit subsequent transaction registers the attendee |

The frontend is React + Vite. A Cloudflare Worker and D1 handle signed sessions, expiring tickets and offchain networking data. The Worker does not control escrowed funds.

## Try it: do X, see Y

Use **testnet accounts with testnet NEAR**. The app always shows its network. Browsing needs no wallet.

1. **Open the app →** see real event records and their deposit rules. If no contract is configured locally, illustrative events are explicitly labelled and cannot accept deposits.
2. **Connect an existing NEAR testnet wallet and choose “Host an event” →** enter the venue, deposit, capacity, guest bonus percentage, registration cutoff, and check-in window. Publication fixes these terms permanently.
3. **Publish →** the wallet shows the fixed event storage reserve, separately from network gas. At the current storage price this is **0.3 NEAR**, reserving enough room for the full guest list.
4. **Use another wallet to reserve a place →** confirm the refundable attendance deposit plus a separate **0.01024 NEAR** registration storage fee and gas. Contract state, rather than the swap service, confirms the registration.
5. **Before registration closes, cancel a guest registration →** the seat is released immediately and the deposit becomes claimable. Storage fees are not refunded.
6. **Open “My ticket” →** sign a wallet message and generate a 60-second QR ticket. The organizer scans it or pastes its code, verifies it, and signs the onchain check-in. Scanning alone does not mark attendance.
7. **After check-in closes, choose “Settle event” from any connected wallet →** payout amounts become fixed; no organizer approval is needed for settlement.
8. **Claim from attendee wallets →** checked-in guests receive their deposit and bonus. The organizer separately claims their allocation. A failed transfer restores a claimable balance, and a successful payout cannot be claimed twice.
9. **Opt in under “Private introductions” →** save approved interests/goals and an optional contact. Shared interests rank up to three candidates per request with no AI requests or paid credits. Contact details appear only after both parties accept.

### Concrete payout example

Ten guests deposit 1 NEAR, eight check in, and the guest bonus share is 50%. The 2 NEAR no-show pool contributes 1 NEAR to guest bonuses and 1 NEAR to the organizer. Each attendee claims **1.125 NEAR**.

If the organizer cancels before check-in opens, everyone can claim their deposit. If nobody checks in, everyone can also claim their deposit. Claims never expire. Integer rounding dust goes to the organizer; storage and network fees are always separate.

## Run locally

Requirements: Node **22.18+** (the SQLite test adapter needs Node 22), pnpm 11.1, Rust, and cargo-near for contract builds.

```sh
pnpm install --frozen-lockfile
pnpm types
pnpm web:build
pnpm db:local
pnpm service:dev
```

Open `http://127.0.0.1:8787`. For frontend hot reload, also run `pnpm web:dev` and open port 5173; `/api` requests proxy to the local Worker on port 8787.

The checked-in default configuration points at the deployed testnet contract. Set `EVENT_CONTRACT` to an empty string to review the labelled visual preview instead. Never point testnet configuration at a mainnet contract.

Introductions are calculated locally in the Worker from shared interests. No external inference service or credits are required. An optional `ONECLICK_API_KEY` belongs in ignored `web/.dev.vars` locally or an encrypted Worker secret remotely, never browser code.

```sh
pnpm check
pnpm test
pnpm web:build
pnpm contract:build
```

`pnpm test` runs 21 Rust tests and 15 service/shared tests. Coverage includes split boundaries, conservation and rounding, cancellations, zero attendance, capacity, authorization, deadline boundaries, payout recovery, real signature validation, challenge/ticket expiry and replay, mutual contact consent, payment recipient binding, and proof that matching uses only local shared-interest ranking.

## Payments: what is verified and what is not

1Click has **no testnet**. Testnet deployments reject all payment routes; use testnet NEAR directly. A live Base USDC → NEAR **dry quote** has been verified without creating a funding address or transferring funds. A funded mainnet swap has **not** been performed.

The separate `mainnet` Wrangler environment has its own database binding and an empty contract ID. Before enabling it, deploy/review a mainnet contract, create a separate mainnet D1 database, and explicitly authorize a small real-value validation. No mainnet deployment or funded transaction is included in this prototype.

In mainnet mode, Arrivow previews a quote before creating a deposit address. The quote names the asset, source chain, exact amount, recipient, refund address, fees and deadline. Save/reload recovery retains the funding address and its status. Sending funds to that address tops up the connected NEAR wallet; it does not reserve an event place. Seats can fill while a swap is pending, and funds then remain in the attendee's wallet. Cancellation payouts remain native NEAR, not the asset used for funding.

## Privacy and trust boundaries

- **Attendance is host-attested.** QR tickets identify paid registrations and prevent accidental/replayed scans; they cannot prove physical presence or stop a dishonest organizer from misreporting attendance.
- **Public:** deposits, wallet accounts, event terms and attendance are public on NEAR. Arrivow does not promise private event participation.
- **Offchain:** networking profiles live in D1. The application service can read them. Profiles stay within the application service; no external inference provider receives them. Contact details are revealed only after mutual acceptance.
- **Introductions:** model IDs are scoped to one request. Returned IDs are validated against the shortlist. Displayed reasons are generated from approved shared tags, preventing free-form model output from disclosing private goals. Editing or deleting a profile revokes its introductions.
- **Retention:** profiles and introductions become inaccessible seven days after the check-in window closes. Manual removal is available earlier, including after a guest cancels. A native Durable Object alarm removes expired rows hourly without consuming a cron slot; it persists retries if the database is unavailable.
- **Authentication:** NEP-413 challenges bind account, origin, network and contract, expire after five minutes, and are consumed once. Signatures must belong to an onchain full-access key. Sessions are hashed in D1 and use HttpOnly, SameSite cookies; HTTPS cookies are Secure. Sessions expire after an hour.
- **Recovery:** a consumed QR ticket can be refreshed if a host's wallet transaction fails. The host can retry the verified attendance confirmation. Pending payouts are callback-protected and restore credits on failure.
- **Operations:** API bodies, profiles, cookies, contacts, secrets and model responses are not logged. Platform observability is enabled without custom sensitive logging. Limit and timeout failures return actionable errors.

## Cloudflare deployment

`web/wrangler.jsonc` binds the isolated `gather-testnet` database and a separate `mainnet` environment. Generate bindings after configuration changes with `pnpm types`.

```sh
pnpm web:build
pnpm exec wrangler d1 migrations apply DB --remote -c web/wrangler.jsonc
pnpm exec wrangler deploy --dry-run -c web/wrangler.jsonc --env ''
pnpm exec wrangler deploy -c web/wrangler.jsonc --env ''
```

Paste credentials only into the CLI secret prompt, never source, logs, screenshots, or public repository fields. An optional `ONECLICK_API_KEY` can be stored as a Worker secret; unauthenticated quotes work with the applicable 1Click fee schedule.

The account's five free cron slots were occupied, so scheduled deletion uses the approved SQLite-backed `RetentionAlarm` Durable Object instead. It starts when the first authentication challenge is issued, runs initially after one minute, and then hourly. It stores no profile copies. No existing application's schedule is changed and no plan upgrade is needed for the prototype's free-tier usage.

## Hackathon submission

Suggested tracks: **Real-World Money** and **Open Innovation**. The money and networking flows are independently usable; only the contract rules and host-attested attendance determine payouts.

For a short demo, show immutable event terms, a registered guest's ticket, host check-in, the settled 0.125-NEAR attendee and 0.05-NEAR host amounts, cancellation recovery, and consent-based introductions. State the distinction between the real testnet escrow flow and mainnet-only cross-chain funding.

Publish this repository and record the working flow before posting the public X demo. Repository publication, the demo post, the final description, team details and prize wallet still require the team's submission choices. No X post or final hackathon form has been submitted by this implementation.

The previous Court source is preserved in the local Git tag `court-checkpoint` (commit `c8f992d`). It is not kept as a second active implementation.
