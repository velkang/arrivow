import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ReactNode } from "react";
import type { NearConnector } from "@hot-labs/near-connect";
import QRCode from "qrcode";
import type { Config, Event, Match, Profile, Registration } from "../shared";
import { formatNear, parseNear } from "../shared";
import {
  allEvents,
  api,
  authenticate,
  connector,
  registration,
  transact,
  view,
} from "./client";

type Run = (action: () => Promise<unknown>, success?: string) => Promise<void>;
type Shared = {
  config: Config;
  wallet: NearConnector;
  account: string;
  busy: boolean;
  run: Run;
  fees: { event: string; registration: string };
};
const date = (ms: string, options?: Intl.DateTimeFormatOptions) =>
  new Date(Number(ms)).toLocaleString(
    undefined,
    options ?? { dateStyle: "medium", timeStyle: "short" },
  );
const upcoming = (e: Event) =>
  e.status === "Open" && Number(e.checkin_closes) > Date.now();
const phase = (e: Event, now = Date.now()) =>
  e.status !== "Open"
    ? e.status
    : now >= Number(e.checkin_closes)
      ? "Ready to settle"
      : now >= Number(e.registration_closes)
        ? "Registration closed"
        : "Registration open";
function Arrow() {
  return (
    <svg
      aria-hidden="true"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
    >
      <path d="M4 12h15M13 5l7 7-7 7" />
    </svg>
  );
}
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
    </label>
  );
}
function Empty({ children }: { children: ReactNode }) {
  return <div className="empty">{children}</div>;
}
const go = (route: string) => {
  location.hash = route;
};

export default function App() {
  const [config, setConfig] = useState<Config>();
  const [configError, setConfigError] = useState("");
  const [account, setAccount] = useState("");
  const [route, setRoute] = useState(location.hash.slice(1) || "/");
  const [events, setEvents] = useState<[string, Event][]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [fees, setFees] = useState({ event: "0", registration: "0" });
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [mine, setMine] = useState<Set<string>>(new Set());
  const wallet = useMemo(
    () => (config ? connector(config) : undefined),
    [config?.network],
  );
  const refresh = useCallback(() => setRevision((n) => n + 1), []);
  const run: Run = async (action, success) => {
    if (busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
      if (success) setNotice(success);
      refresh();
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : "Something went wrong. Please retry.",
      );
    } finally {
      setBusy(false);
    }
  };
  const loadConfig = useCallback(() => {
    setConfigError("");
    api<Config>("/config")
      .then(setConfig)
      .catch((err) => setConfigError(err.message));
  }, []);
  useEffect(() => {
    loadConfig();
    const change = () => {
      setRoute(location.hash.slice(1) || "/");
      window.scrollTo(0, 0);
    };
    window.addEventListener("hashchange", change);
    return () => window.removeEventListener("hashchange", change);
  }, [loadConfig]);
  useEffect(() => {
    if (!wallet) return;
    const onIn = (event: { accounts: { accountId: string }[] }) =>
      setAccount(event.accounts[0]?.accountId ?? "");
    const onOut = () => setAccount("");
    wallet.on("wallet:signIn", onIn);
    wallet.on("wallet:signOut", onOut);
    wallet
      .getConnectedWallet()
      .then((c) => setAccount(c.accounts[0]?.accountId ?? ""))
      .catch(() => {});
    return () => {
      wallet.off("wallet:signIn", onIn);
      wallet.off("wallet:signOut", onOut);
    };
  }, [wallet]);
  useEffect(() => {
    if (!config?.configured) return;
    let disposed = false;
    setLoading(true);
    Promise.all([
      allEvents(config),
      view<{ event: string; registration: string }>(config, "storage_fees", {}),
    ])
      .then(([list, storage]) => {
        if (!disposed) {
          setEvents(list);
          setFees(storage);
          setError("");
        }
      })
      .catch((err) => {
        if (!disposed) setError(err.message);
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [config, revision]);
  useEffect(() => {
    if (!config?.configured || !account) {
      setMine(new Set());
      return;
    }
    let disposed = false;
    Promise.all(
      events.map(
        async ([id]) => [id, await registration(config, id, account)] as const,
      ),
    )
      .then((rows) => {
        if (!disposed)
          setMine(
            new Set(rows.filter(([, r]) => r !== null).map(([id]) => id)),
          );
      })
      .catch((err) => {
        if (!disposed) setError(err.message);
      });
    return () => {
      disposed = true;
    };
  }, [config, account, events]);
  useEffect(() => {
    const pending = sessionStorage.getItem("gather:pending");
    if (pending)
      setNotice(
        "A wallet action was interrupted. Refresh the event to check its onchain status before retrying.",
      );
  }, []);
  const preview = Boolean(config && !config.configured);
  const list = preview ? previewEvents() : events;
  const selected = route.match(/^\/events\/([^/]+)(?:\/(manage))?$/);
  const isMine = route === "/mine";
  // Upcoming events soonest first, then past events most recent first.
  const filtered = list
    .filter(
      ([id, e]) =>
        (!isMine || mine.has(id) || e.organizer === account) &&
        `${e.title} ${e.location} ${e.description}`
          .toLowerCase()
          .includes(search.toLowerCase()) &&
        (filter !== "upcoming" || upcoming(e)),
    )
    .sort(([, a], [, b]) =>
      upcoming(a) !== upcoming(b)
        ? upcoming(a)
          ? -1
          : 1
        : (upcoming(a) ? 1 : -1) *
          (Number(a.checkin_opens) - Number(b.checkin_opens)),
    );
  const shared =
    config && wallet ? { config, wallet, account, busy, run, fees } : undefined;

  return (
    <>
      <a className="skip" href="#main">
        Skip to content
      </a>
      <header className="site-header">
        <a className="brand" href="#/" aria-label="Arrivow home">
          <span className="brand-mark" aria-hidden="true">
            <i />
            <i />
            <i />
            <i />
          </span>
          Arrivow
        </a>
        <nav aria-label="Main navigation">
          <a aria-current={route === "/" ? "page" : undefined} href="#/">
            Discover
          </a>
          <a aria-current={isMine ? "page" : undefined} href="#/mine">
            My events
          </a>
          <a
            aria-current={route === "/host" ? "page" : undefined}
            href="#/host"
          >
            Host an event
          </a>
        </nav>
        <div className="wallet-controls">
          <span
            className={`network ${config?.network === "mainnet" ? "mainnet" : ""}`}
          >
            {config?.network ?? "Connecting"}
          </span>
          <button
            disabled={!wallet || busy}
            onClick={() =>
              run(async () => {
                if (!wallet) return;
                if (account) {
                  try {
                    await api("/auth/logout", {});
                  } finally {
                    await wallet.disconnect();
                  }
                } else await wallet.connect();
              })
            }
          >
            {account
              ? `${account.length > 22 ? account.slice(0, 18) + "…" : account} · Disconnect`
              : "Connect wallet"}
          </button>
        </div>
      </header>
      <main id="main">
        {configError && (
          <div className="message error" role="alert">
            {configError} <button onClick={loadConfig}>Retry connection</button>
          </div>
        )}
        {error && (
          <div className="message error" role="alert">
            {error}
            <button onClick={() => setError("")} aria-label="Dismiss error">
              Dismiss
            </button>
          </div>
        )}
        {notice && (
          <div className="message" role="status">
            {notice}
            <button
              onClick={() => {
                setNotice("");
                sessionStorage.removeItem("gather:pending");
              }}
            >
              Dismiss
            </button>
          </div>
        )}
        {busy && (
          <p role="status" className="activity">
            Working… Complete any wallet confirmation, then keep this page open.
          </p>
        )}
        {route === "/host" ? (
          shared && <CreateEvent {...shared} />
        ) : selected ? (
          shared && (
            <EventDetail
              key={selected[1]}
              {...shared}
              id={selected[1]}
              previewEvent={
                preview
                  ? list.find(([id]) => id === selected[1])?.[1]
                  : undefined
              }
              manage={Boolean(selected[2])}
            />
          )
        ) : route === "/privacy" ? (
          <Privacy />
        ) : (
          <>
            <div className="browse-layout">
              <aside className="browse-sidebar">
                <h1>
                  {isMine ? (
                    <>
                      YOUR
                      <br />
                      PEOPLE.
                      <br />
                      YOUR
                      <br />
                      PLANS.
                    </>
                  ) : (
                    <>
                      GOOD
                      <br />
                      PEOPLE.
                      <br />
                      REAL
                      <br />
                      PLANS.
                    </>
                  )}
                </h1>
                <p className="sidebar-intro">
                  Commit a deposit.
                  <br />
                  Join the room.
                  <br />
                  Claim it back after the event.
                </p>
                <Field label="Find your next room">
                  <input
                    type="search"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="Event, topic or location"
                  />
                </Field>
                <div className="filters" aria-label="Event filters">
                  <button
                    aria-pressed={filter === "all"}
                    onClick={() => setFilter("all")}
                  >
                    All events
                  </button>
                  <button
                    aria-pressed={filter === "upcoming"}
                    onClick={() => setFilter("upcoming")}
                  >
                    Upcoming
                  </button>
                </div>
                <p className="small">
                  An attendance deposit is a commitment to show up. It is
                  separate from storage and network fees.
                </p>
                <p className="small">
                  Payments via NEAR Intents.
                  <br />
                  Optional private introductions.
                </p>
              </aside>
              <section className="event-directory" aria-label="Events">
                <div className="directory-heading">
                  <div>
                    <h2>{isMine ? "Your programme" : "Find your people."}</h2>
                    <p>Conversations, connections and what comes next.</p>
                  </div>
                  <button
                    className="text-button"
                    onClick={refresh}
                    disabled={loading}
                  >
                    Refresh events
                  </button>
                </div>
                {preview && (
                  <p className="preview-note">
                    Illustrative preview · No live contract configured. These
                    events cannot accept deposits.
                  </p>
                )}
                {isMine && !account ? (
                  <Empty>
                    Connect your NEAR wallet to see your registrations and
                    hosted events.
                  </Empty>
                ) : !config || loading ? (
                  <Empty>
                    {config
                      ? "Reading events from NEAR…"
                      : "Connecting to the event service…"}
                  </Empty>
                ) : filtered.length === 0 ? (
                  <Empty>
                    No events found. Try another search or host the first event.
                  </Empty>
                ) : (
                  filtered.map(([id, e]) => (
                    <EventRow key={id} id={id} event={e} preview={preview} />
                  ))
                )}
                <DepositExplainer share={50} />
              </section>
            </div>
          </>
        )}
      </main>
      <footer className="site-footer">
        <a className="brand" href="#/">
          Arrivow
        </a>
        <p>Showing up pays.</p>
        <span>
          NEAR HACKATHON prototype · {config?.network ?? "Network unavailable"}
        </span>
        <a href="#/privacy">Privacy & how it works</a>
      </footer>
    </>
  );
}

function Privacy() {
  return (
    <section className="privacy-page">
      <h1>Your data and your deposit</h1>
      <p>
        Deposits and attendance records are public on NEAR. The organizer is
        trusted to report attendance honestly. Contract terms govern payouts;
        the service never holds your funds.
      </p>
      <p>
        Networking is optional. The app service can read your profile and
        matches people by shared interests. Your contact details are revealed
        only after mutual acceptance.
      </p>
      <p>
        Networking data expires seven days after the check-in window closes. An
        hourly cleanup removes expired records. You can remove your profile
        earlier. Wallet sessions expire after one hour. Storage and network fees
        are not refundable.
      </p>
      <a className="text-link" href="#/">
        Back to events <Arrow />
      </a>
    </section>
  );
}

function EventRow({
  id,
  event: e,
  preview,
}: {
  id: string;
  event: Event;
  preview: boolean;
}) {
  const time = new Date(Number(e.checkin_opens));
  return (
    <article className="event-row">
      <div className="event-date">
        <span>{time.toLocaleString(undefined, { month: "short" })}</span>
        <strong>{time.getDate().toString().padStart(2, "0")}</strong>
        <span>{time.toLocaleString(undefined, { weekday: "short" })}</span>
      </div>
      <div className="event-copy">
        <span className="event-status">
          {preview ? "Illustrative preview" : phase(e)}
        </span>
        <h3>
          <a href={`#/events/${id}`}>{e.title}</a>
        </h3>
        <p>
          {e.description.length > 140
            ? e.description.slice(0, 137) + "…"
            : e.description}
        </p>
        <p className="small">
          {date(e.checkin_opens, { hour: "numeric", minute: "2-digit" })} –{" "}
          {date(e.checkin_closes, { hour: "numeric", minute: "2-digit" })} ·{" "}
          {e.location}
        </p>
      </div>
      <div className="event-terms">
        <strong>{formatNear(e.deposit)} NEAR</strong>
        <span>Attendance deposit</span>
        <p>{e.bonus_bps / 100}% of no-show deposits go to guests.</p>
      </div>
      <div className="event-action">
        <span>
          {preview || phase(e) === "Registration open"
            ? `${e.capacity - e.registered} of ${e.capacity} places left`
            : `${e.attended} of ${e.registered} checked in`}
        </span>
        <a className="text-link" href={`#/events/${id}`}>
          View event <Arrow />
        </a>
      </div>
    </article>
  );
}

function DepositExplainer({ share }: { share: number }) {
  return (
    <section className="deposit-explainer">
      <div>
        <h2>Your deposit, explained.</h2>
        <p>
          Check in and claim your full deposit after settlement, plus your share
          of the guest bonus pool. If you miss the event, your deposit is split
          as published.
        </p>
        <p className="small">
          Storage and network fees are separate and non-refundable.
        </p>
      </div>
      <div>
        <div
          className="split-bar"
          aria-label={`${share}% guest bonuses, ${100 - share}% host`}
        >
          <span style={{ flex: share || 0.01 }}>{share}%</span>
          <span style={{ flex: 100 - share || 0.01 }}>{100 - share}%</span>
        </div>
        <div className="split-labels">
          <span>Guest bonuses</span>
          <span>Host</span>
        </div>
        <p className="small">
          If nobody checks in, everyone gets their deposit back.
        </p>
      </div>
    </section>
  );
}

function CreateEvent(s: Shared) {
  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    s.run(async () => {
      if (!s.account) throw new Error("Connect your NEAR wallet first.");
      const deposit = parseNear(String(f.get("deposit")));
      if (BigInt(deposit) <= 0n) throw new Error("Deposit must be positive.");
      const times = [
        "registration_closes",
        "checkin_opens",
        "checkin_closes",
      ].map((k) => new Date(String(f.get(k))).getTime());
      if (
        !times.every(Number.isFinite) ||
        times[0] <= Date.now() ||
        times[0] > times[1] ||
        times[1] >= times[2]
      )
        throw new Error(
          "Set a future registration cutoff, followed by check-in opening and closing.",
        );
      const args = {
        title: f.get("title"),
        description: f.get("description"),
        location: f.get("location"),
        capacity: Number(f.get("capacity")),
        deposit,
        bonus_bps: Math.round(Number(f.get("bonus")) * 100),
        registration_closes: String(times[0]),
        checkin_opens: String(times[1]),
        checkin_closes: String(times[2]),
      };
      await transact(
        s.config,
        s.wallet,
        s.account,
        "create_event",
        args,
        s.fees.event,
      );
      go("/mine");
    }, "Event published. Its deposit terms and deadlines are now fixed.");
  }
  return (
    <section className="form-page">
      <a className="back" href="#/">
        Back to events
      </a>
      <h1>Make a plan worth keeping.</h1>
      <p>
        Publish a conference session or side event. Up to 200 guests, one clear
        commitment.
      </p>
      {!s.config.configured && (
        <p className="preview-note">
          Configure an event contract to publish. This form is a preview.
        </p>
      )}
      {s.config.configured && !s.account && (
        <p className="preview-note">
          Connect your NEAR wallet to publish. You can fill in the form first.
        </p>
      )}
      <form onSubmit={submit} className="event-form">
        <Field label="Event name">
          <input
            name="title"
            required
            maxLength={120}
            placeholder="Builders after hours"
          />
        </Field>
        <Field label="Description">
          <textarea
            name="description"
            maxLength={2000}
            rows={4}
            placeholder="Who is this for, and what should guests expect?"
          />
        </Field>
        <Field label="Location">
          <input
            name="location"
            required
            maxLength={300}
            placeholder="Venue, room and city"
          />
        </Field>
        <div className="form-columns">
          <Field label="Guest capacity">
            <input
              name="capacity"
              type="number"
              min={1}
              max={200}
              defaultValue={50}
              required
            />
          </Field>
          <Field label="Attendance deposit · NEAR">
            <input
              name="deposit"
              inputMode="decimal"
              defaultValue="1"
              required
              pattern="[0-9]+(\.[0-9]{1,24})?"
            />
          </Field>
          <Field label="Guest share of no-show deposits · %">
            <input
              name="bonus"
              type="number"
              min={0}
              max={100}
              step={1}
              defaultValue={50}
              required
            />
          </Field>
        </div>
        <div className="form-columns">
          <Field label="Registration and cancellation close">
            <input name="registration_closes" type="datetime-local" required />
          </Field>
          <Field label="Check-in opens">
            <input name="checkin_opens" type="datetime-local" required />
          </Field>
          <Field label="Check-in closes">
            <input name="checkin_closes" type="datetime-local" required />
          </Field>
        </div>
        <p className="small">
          Times use your device timezone. Terms cannot be edited after
          publication. You may cancel the event before check-in opens.
        </p>
        <p>
          Non-refundable event storage reserve:{" "}
          <strong>{formatNear(s.fees.event)} NEAR</strong>, plus wallet network
          fees.
        </p>
        <button
          className="primary"
          disabled={s.busy || !s.account || !s.config.configured}
        >
          Publish event <Arrow />
        </button>
      </form>
    </section>
  );
}

function EventDetail(
  s: Shared & { id: string; previewEvent?: Event; manage: boolean },
) {
  const [e, setEvent] = useState<Event | undefined>(s.previewEvent);
  const [r, setRegistration] = useState<Registration | null>(null);
  const [claimable, setClaimable] = useState("0");
  const [tab, setTab] = useState(s.manage ? "manage" : "details");
  const [localError, setLocalError] = useState("");
  const [tick, setTick] = useState(Date.now());
  const load = useCallback(async () => {
    if (s.previewEvent) {
      setEvent(s.previewEvent);
      return;
    }
    const current = await view<Event | null>(s.config, "event", {
      event_id: s.id,
    });
    if (!current) throw new Error("Event not found.");
    setEvent(current);
    if (s.account) {
      const [reg, amount] = await Promise.all([
        registration(s.config, s.id, s.account),
        view<string>(s.config, "claimable", {
          event_id: s.id,
          account_id: s.account,
        }),
      ]);
      setRegistration(reg);
      setClaimable(amount);
    } else {
      setRegistration(null);
      setClaimable("0");
    }
  }, [s.config, s.id, s.account, s.previewEvent]);
  useEffect(() => {
    let disposed = false;
    load().catch((err) => {
      if (!disposed) setLocalError(err.message);
    });
    return () => {
      disposed = true;
    };
  }, [load, s.busy]);
  useEffect(() => {
    const timer = setInterval(() => setTick(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const action = (
    method: string,
    args: Record<string, unknown> = {},
    deposit = "0",
  ) =>
    s.run(async () => {
      await transact(
        s.config,
        s.wallet,
        s.account,
        method,
        { event_id: s.id, ...args },
        deposit,
      );
      await load();
    }, "Transaction confirmed. The latest contract state is shown.");
  if (localError)
    return (
      <Empty>
        {localError}{" "}
        <button
          onClick={() => {
            setLocalError("");
            load().catch((err) => setLocalError(err.message));
          }}
        >
          Retry
        </button>
      </Empty>
    );
  if (!e) return <Empty>Reading event…</Empty>;
  const host = e.organizer === s.account;
  const live = s.config.configured && !s.previewEvent;
  const open = e.status === "Open";
  const canRegister =
    open && tick < Number(e.registration_closes) && e.registered < e.capacity;
  const total = (BigInt(e.deposit) + BigInt(s.fees.registration)).toString();
  const status = phase(e, tick);
  const left = e.capacity - e.registered;
  const offer = canRegister && !r?.active;
  return (
    <section className="detail-page">
      <a className="back" href="#/">
        Back to events
      </a>
      {s.previewEvent && (
        <p className="preview-note">
          Illustrative event · No funds can be deposited into this preview.
        </p>
      )}
      <div className="detail-heading">
        <div>
          <span className="event-status">{status}</span>
          <h1>{e.title}</h1>
          <p>
            {e.location} · {date(e.checkin_opens)}
          </p>
        </div>
        <div className="detail-deposit">
          <strong>{formatNear(e.deposit)} NEAR</strong>
          <span>Attendance deposit</span>
          {status === "Registration open" && (
            <span>
              {left} {left === 1 ? "place" : "places"} left
            </span>
          )}
        </div>
      </div>
      <div className="tabs" role="navigation" aria-label="Event sections">
        {[
          "details",
          ...(r?.active ? ["ticket"] : []),
          ...(r ? ["networking"] : []),
          ...(host ? ["manage"] : []),
        ].map((t) => (
          <button key={t} aria-pressed={tab === t} onClick={() => setTab(t)}>
            {t === "manage"
              ? "Organizer desk"
              : t === "networking"
                ? "Private introductions"
                : t === "ticket"
                  ? "My ticket"
                  : "Event details"}
          </button>
        ))}
      </div>
      {tab === "details" && (
        <div className="detail-grid">
          <div>
            <h2>A room worth showing up for.</h2>
            <p className="description">{e.description}</p>
            <dl className="event-facts">
              <dt>Registration & cancellation cutoff</dt>
              <dd>{date(e.registration_closes)}</dd>
              <dt>Check-in window</dt>
              <dd>
                {date(e.checkin_opens)} – {date(e.checkin_closes)}
              </dd>
              <dt>Organizer</dt>
              <dd>{e.organizer}</dd>
              <dt>Attendance</dt>
              <dd>
                {e.attended} checked in / {e.registered} registered
              </dd>
            </dl>
            <p className="small">
              The organizer confirms attendance. Settlement is permissionless
              after the check-in window closes.
            </p>
          </div>
          <aside className="reservation">
            <h2>
              {r?.active && open
                ? "Your place is reserved."
                : status === "Settled"
                  ? "This event has settled."
                  : status === "Cancelled"
                    ? "This event was cancelled."
                    : offer
                      ? "Make the commitment."
                      : status === "Registration open"
                        ? "This event is full."
                        : "Registration has closed."}
            </h2>
            {r?.checked_in && open && (
              <p className="positive">
                You’re checked in. Claim your payout after settlement.
              </p>
            )}
            {offer && (
              <>
                <dl>
                  <dt>Refundable deposit</dt>
                  <dd>{formatNear(e.deposit)} NEAR</dd>
                  <dt>Non-refundable storage fee</dt>
                  <dd>{formatNear(s.fees.registration)} NEAR</dd>
                  <dt>Total to register</dt>
                  <dd>{formatNear(total)} NEAR + gas</dd>
                </dl>
                <p>
                  {e.bonus_bps / 100}% of no-show deposits fund guest bonuses.
                  Your full deposit is claimable after settlement if you check
                  in.
                </p>
              </>
            )}
            {!s.account ? (
              offer && <p>Connect your NEAR wallet to register.</p>
            ) : (
              offer && (
                <button
                  className="primary"
                  disabled={
                    s.busy ||
                    !live ||
                    Boolean(
                      r && (BigInt(r.credit) > 0n || BigInt(r.pending) > 0n),
                    )
                  }
                  onClick={() => action("register", {}, total)}
                >
                  Reserve a place <Arrow />
                </button>
              )
            )}
            {r?.active && open && tick < Number(e.registration_closes) && (
              <button
                disabled={s.busy || !live}
                onClick={() => action("cancel_registration")}
              >
                Cancel registration · refund deposit
              </button>
            )}
            {BigInt(claimable) > 0n && (
              <button
                className="primary"
                disabled={s.busy || !live}
                onClick={() => action("claim")}
              >
                Claim {formatNear(claimable)} NEAR
              </button>
            )}
            {r && BigInt(r.pending) > 0n && (
              <p role="status">
                Payout in flight. Refresh shortly; a failed transfer becomes
                claimable again.
              </p>
            )}
            {open && tick >= Number(e.checkin_closes) && (
              <button
                disabled={s.busy || !live || !s.account}
                onClick={() => action("finalize")}
              >
                Settle event & unlock payouts
              </button>
            )}
            {host && BigInt(e.host_claimable) > 0n && (
              <button
                disabled={s.busy || !live}
                onClick={() => action("claim_host")}
              >
                Claim host allocation · {formatNear(e.host_claimable)} NEAR
              </button>
            )}
            {host && BigInt(e.host_pending) > 0n && (
              <p>Host payout in flight. Refresh to check completion.</p>
            )}
            <button className="text-button" onClick={() => s.run(load)}>
              Refresh contract state
            </button>
            {live && s.account && <Funding {...s} required={total} />}
          </aside>
        </div>
      )}
      {tab === "ticket" && r?.active && (
        <Ticket {...s} id={s.id} checkedIn={r.checked_in} />
      )}
      {tab === "networking" && r && (
        <Networking {...s} id={s.id} active={r.active} />
      )}
      {tab === "manage" && host && (
        <Organizer {...s} id={s.id} event={e} action={action} />
      )}
      <DepositExplainer share={e.bonus_bps / 100} />
    </section>
  );
}

function Ticket(s: Shared & { id: string; checkedIn: boolean }) {
  const [ticket, setTicket] = useState<{ token: string; expires: number }>();
  const [image, setImage] = useState("");
  const [now, setNow] = useState(Date.now());
  const issue = () =>
    s.run(async () => {
      await authenticate(s.wallet, s.account);
      const t = await api<{ token: string; expires: number }>(
        `/events/${s.id}/ticket`,
        {},
      );
      const url = await QRCode.toDataURL(
        `gather-ticket:v1:${s.id}:${t.token}`,
        { width: 320, margin: 3, errorCorrectionLevel: "M" },
      );
      setTicket(t);
      setImage(url);
    });
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  if (s.checkedIn)
    return (
      <Empty>
        You’re already checked in. Your payout becomes claimable after
        settlement.
      </Empty>
    );
  return (
    <section className="ticket-panel">
      <div>
        <h2>Your place in the room.</h2>
        <p>
          Show this ticket to the organizer at the venue. Each code expires
          after one minute and can be scanned once.
        </p>
        <p className="small">
          A scan identifies your registration. The organizer then confirms
          check-in on NEAR.
        </p>
        <button className="primary" disabled={s.busy} onClick={issue}>
          {ticket ? "Refresh ticket" : "Generate my ticket"}
        </button>
      </div>
      <div className="ticket-code">
        {image && ticket && now < ticket.expires ? (
          <>
            <img src={image} alt="Your short-lived check-in QR ticket" />
            <p role="status">
              Expires in {Math.max(0, Math.ceil((ticket.expires - now) / 1000))}{" "}
              seconds
            </p>
            <details>
              <summary>Show code for manual scanning</summary>
              <code>{`gather-ticket:v1:${s.id}:${ticket.token}`}</code>
            </details>
          </>
        ) : (
          <Empty>
            {ticket
              ? "Ticket expired. Generate a fresh code."
              : "Your QR ticket will appear here."}
          </Empty>
        )}
      </div>
    </section>
  );
}

function Organizer(
  s: Shared & {
    id: string;
    event: Event;
    action: (method: string, args?: Record<string, unknown>) => Promise<void>;
  },
) {
  const [guests, setGuests] = useState<[string, Registration][]>([]);
  const [guestError, setGuestError] = useState("");
  const [search, setSearch] = useState("");
  const [token, setToken] = useState("");
  const [scanning, setScanning] = useState(false);
  const [identified, setIdentified] = useState("");
  const [cancel, setCancel] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    let disposed = false;
    Promise.all([
      view<[string, Registration][]>(s.config, "attendees", {
        event_id: s.id,
        from_index: 0,
        limit: 100,
      }),
      view<[string, Registration][]>(s.config, "attendees", {
        event_id: s.id,
        from_index: 100,
        limit: 100,
      }),
    ])
      .then((pages) => {
        if (!disposed) {
          setGuests(pages.flat());
          setGuestError("");
        }
      })
      .catch((err) => {
        if (!disposed) setGuestError(err.message);
      });
    return () => {
      disposed = true;
    };
  }, [s.id, s.event.attended, s.event.registered, s.busy]);
  useEffect(() => {
    if (!scanning || !video.current) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    import("@zxing/browser")
      .then(async ({ BrowserQRCodeReader }) => {
        const reader = new BrowserQRCodeReader();
        const controls = await reader.decodeFromVideoDevice(
          undefined,
          video.current!,
          (result) => {
            if (result && !disposed) {
              setToken(result.getText());
              setScanning(false);
            }
          },
        );
        if (disposed) controls.stop();
        else stop = () => controls.stop();
      })
      .catch((err) => {
        if (!disposed) {
          setScanning(false);
          s.run(async () => {
            throw new Error(
              `Camera unavailable: ${err.message}. Paste the ticket code instead.`,
            );
          });
        }
      });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [scanning]);
  const redeem = () =>
    s.run(async () => {
      const match = token
        .trim()
        .match(/^gather-ticket:v1:(\d+):([a-f0-9]{64})$/);
      if (!match || match[1] !== s.id)
        throw new Error("Paste a ticket for this event.");
      await authenticate(s.wallet, s.account);
      const result = await api<{ account: string }>(`/events/${s.id}/redeem`, {
        token: match[2],
      });
      setIdentified(result.account);
      setToken("");
    }, "Ticket verified. Confirm this guest’s attendance below.");
  return (
    <section className="organizer-panel">
      <div>
        <h2>Welcome people in.</h2>
        <p>
          {s.event.attended} checked in · {s.event.registered} registrations
        </p>
        <button onClick={() => setScanning(!scanning)} disabled={s.busy}>
          {scanning ? "Stop camera" : "Scan a ticket"}
        </button>
        {scanning && (
          <video ref={video} className="scanner" autoPlay playsInline muted />
        )}
        <Field label="Or paste the guest’s ticket code">
          <input
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="gather-ticket:v1:…"
          />
        </Field>
        <button disabled={s.busy || !token} onClick={redeem}>
          Verify ticket
        </button>
        {identified && (
          <div className="verified-guest">
            <p>
              Verified guest: <strong>{identified}</strong>
            </p>
            <button
              className="primary"
              disabled={s.busy}
              onClick={() => s.action("check_in", { account_id: identified })}
            >
              Confirm check-in on NEAR
            </button>
            <p className="small">
              If the wallet action fails, retry this confirmation without
              scanning again.
            </p>
          </div>
        )}
        {s.event.status === "Open" &&
          Date.now() < Number(s.event.checkin_opens) && (
            <details open={cancel}>
              <summary
                onClick={(e) => {
                  e.preventDefault();
                  setCancel(!cancel);
                }}
              >
                Cancel this event
              </summary>
              <p>
                All registered guests will be able to reclaim their attendance
                deposits. Storage fees are not refunded.
              </p>
              <button
                className="danger"
                disabled={s.busy}
                onClick={() => s.action("cancel_event")}
              >
                Confirm event cancellation
              </button>
            </details>
          )}
      </div>
      <div>
        <h2>The guest list</h2>
        {guestError && <p role="alert">{guestError}</p>}
        <Field label="Find a guest">
          <input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="NEAR account"
          />
        </Field>
        <ul className="guest-list">
          {guests
            .filter(([account]) => account.includes(search.toLowerCase()))
            .map(([account, r]) => (
              <li key={account}>
                <span>{account}</span>
                <strong>{r.checked_in ? "Checked in" : "Expected"}</strong>
              </li>
            ))}
        </ul>
        {!guests.length && <Empty>No active registrations yet.</Empty>}
      </div>
    </section>
  );
}

function Networking(s: Shared & { id: string; active: boolean }) {
  const [profile, setProfile] = useState<Profile>({
    interests: [],
    offer: "",
    seek: "",
    contact: "",
    consent: false,
  });
  // Raw text so spaces survive typing; split into interests only on save.
  const [interests, setInterests] = useState("");
  const [matches, setMatches] = useState<Match[]>([]);
  const load = async () => {
    await authenticate(s.wallet, s.account);
    const [p, m] = await Promise.all([
      api<{ profile: Profile | null }>(`/events/${s.id}/profile`),
      api<{ matches: Match[] }>(`/events/${s.id}/matches`),
    ]);
    if (p.profile) {
      setProfile(p.profile);
      setInterests(p.profile.interests.join(", "));
    }
    setMatches(m.matches);
  };
  if (!s.active)
    return (
      <section className="networking-panel">
        <div>
          <h2>Your networking data</h2>
          <p>
            Your registration is inactive. You can still remove any profile and
            introductions you previously shared.
          </p>
          <button
            disabled={s.busy}
            onClick={() =>
              s.run(async () => {
                await authenticate(s.wallet, s.account);
                await api(`/events/${s.id}/profile`, undefined, "DELETE");
              }, "Your networking profile and introductions were removed.")
            }
          >
            Remove my profile
          </button>
        </div>
      </section>
    );
  const save = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    s.run(async () => {
      await authenticate(s.wallet, s.account);
      await api(`/events/${s.id}/profile`, {
        ...profile,
        interests: interests
          .split(",")
          .map((x) => x.trim())
          .filter(Boolean),
      });
      setMatches([]);
    }, "Profile saved. Editing it revokes any previous introductions.");
  };
  return (
    <section className="networking-panel">
      <div>
        <h2>Meet with a little more intention.</h2>
        <p>Opt in to private networking suggestions for this event.</p>
        <div className="privacy-note">
          <strong>Your privacy, clearly.</strong>
          <p>
            Your profile stays offchain. The app service can read it.
            Suggestions are calculated from shared interests within the app.
            Contacts are shared only after both people accept. Data expires
            seven days after check-in closes.
          </p>
        </div>
        <button
          className="text-button"
          disabled={s.busy}
          onClick={() => s.run(load)}
        >
          Load my profile & introductions
        </button>
        <form onSubmit={save}>
          <Field label="Shared interests · comma separated">
            <input
              required
              value={interests}
              onChange={(e) => setInterests(e.target.value)}
              placeholder="Design, Rust, climate"
            />
          </Field>
          <Field label="What I can offer · optional networking goal">
            <textarea
              maxLength={500}
              value={profile.offer}
              onChange={(e) =>
                setProfile({ ...profile, offer: e.target.value })
              }
            />
          </Field>
          <Field label="What I’m looking for · optional networking goal">
            <textarea
              maxLength={500}
              value={profile.seek}
              onChange={(e) => setProfile({ ...profile, seek: e.target.value })}
            />
          </Field>
          <Field label="Contact · revealed only after mutual acceptance">
            <input
              maxLength={200}
              value={profile.contact}
              onChange={(e) =>
                setProfile({ ...profile, contact: e.target.value })
              }
              placeholder="Email or social handle (optional)"
            />
          </Field>
          <label className="check">
            <input
              required
              type="checkbox"
              checked={profile.consent}
              onChange={(e) =>
                setProfile({ ...profile, consent: e.target.checked })
              }
            />
            I approve these interests and goals for networking recommendations
            and shared-interest introductions.
          </label>
          <div className="button-row">
            <button disabled={s.busy || !profile.consent} className="primary">
              Save networking profile
            </button>
            <button
              type="button"
              disabled={s.busy}
              onClick={() =>
                s.run(async () => {
                  await authenticate(s.wallet, s.account);
                  await api(`/events/${s.id}/profile`, undefined, "DELETE");
                  setProfile({
                    interests: [],
                    offer: "",
                    seek: "",
                    contact: "",
                    consent: false,
                  });
                  setInterests("");
                  setMatches([]);
                }, "Your profile and introductions were removed.")
              }
            >
              Remove my profile
            </button>
          </div>
        </form>
      </div>
      <div>
        <h2>People to meet</h2>
        <p>Up to three suggestions per request, based on shared interests.</p>
        <button
          disabled={s.busy}
          onClick={() =>
            s.run(async () => {
              await authenticate(s.wallet, s.account);
              await api<{ generated: number }>(`/events/${s.id}/matches`, {});
              await load();
            })
          }
        >
          Find introductions
        </button>
        {matches.length === 0 ? (
          <Empty>
            No introductions yet. Save a profile and find people with shared
            interests.
          </Empty>
        ) : (
          matches.map((m) => (
            <article className="match" key={m.account}>
              <h3>{m.account}</h3>
              <p>{m.reason}</p>
              {m.mutual ? (
                <p className="positive">
                  You both accepted. Contact:{" "}
                  {m.contact || "No contact shared; connect at the event."}
                </p>
              ) : (
                <>
                  <p>
                    {m.accepted
                      ? "You accepted. Waiting for the other person."
                      : "Your contact stays hidden until you both accept."}
                  </p>
                  <button
                    disabled={s.busy || m.accepted}
                    onClick={() =>
                      s.run(async () => {
                        await authenticate(s.wallet, s.account);
                        const result = await api<{ matches: Match[] }>(
                          `/events/${s.id}/accept`,
                          { account: m.account },
                        );
                        setMatches(result.matches);
                      })
                    }
                  >
                    {m.accepted
                      ? "Introduction accepted"
                      : "Accept introduction"}
                  </button>
                </>
              )}
            </article>
          ))
        )}
      </div>
    </section>
  );
}

type Quote = {
  source: { symbol: string; blockchain: string };
  recipient: string;
  refundTo: string;
  quote: {
    amountInFormatted: string;
    amountOutFormatted: string;
    depositAddress?: string;
    depositMemo?: string;
    deadline?: string;
    timeEstimate: number;
    refundFee?: string;
    withdrawFee?: string;
  };
};
function Funding(s: Shared & { required: string }) {
  const [tokens, setTokens] = useState<
    { assetId: string; symbol: string; blockchain: string }[]
  >([]);
  const [asset, setAsset] = useState("");
  const [refund, setRefund] = useState("");
  const [amount, setAmount] = useState(
    formatNear((BigInt(s.required) + BigInt(parseNear("0.02"))).toString()),
  );
  const [quote, setQuote] = useState<Quote>();
  const [status, setStatus] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [expired, setExpired] = useState(false);
  const savedKey = `gather:swap:${s.config.network}:${s.account}`;
  useEffect(() => {
    const saved = localStorage.getItem(savedKey);
    if (saved) {
      try {
        setQuote(JSON.parse(saved));
      } catch {
        localStorage.removeItem(savedKey);
      }
    }
  }, [savedKey]);
  useEffect(() => {
    if (!quote?.quote.deadline) return;
    const timer = setInterval(
      () => setExpired(Date.now() >= new Date(quote.quote.deadline!).getTime()),
      1000,
    );
    return () => clearInterval(timer);
  }, [quote]);
  if (s.config.network !== "mainnet")
    return (
      <details>
        <summary>Fund from another chain</summary>
        <p>
          1Click uses mainnet funds. This deployment uses testnet NEAR; do not
          send real assets. Use a testnet faucet or fund your existing testnet
          wallet.
        </p>
      </details>
    );
  const requestQuote = (dry: boolean) =>
    s.run(async () => {
      await authenticate(s.wallet, s.account);
      const result = await api<Quote>("/payments/quote", {
        originAsset: asset,
        refundTo: refund,
        amount: parseNear(amount),
        dry,
      });
      setQuote(result);
      setExpired(false);
      setStatus("");
      if (!dry) localStorage.setItem(savedKey, JSON.stringify(result));
    });
  return (
    <details>
      <summary>Fund from another chain with 1Click</summary>
      <p>
        The swap funds your NEAR wallet. Return here to deposit separately; no
        place is reserved until the contract confirms registration.
      </p>
      <button
        disabled={s.busy}
        onClick={() =>
          s.run(async () => {
            await authenticate(s.wallet, s.account);
            const list = await api<typeof tokens>("/payments/tokens");
            setTokens(list);
            setAsset(list[0]?.assetId ?? "");
          })
        }
      >
        Load supported payment routes
      </button>
      <Field label="Pay with">
        <select
          value={asset}
          onChange={(e) => {
            setAsset(e.target.value);
            setQuote(undefined);
            setConfirmed(false);
          }}
        >
          <option value="">Choose a route</option>
          {tokens.map((t) => (
            <option key={t.assetId} value={t.assetId}>
              {t.symbol} · {t.blockchain}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Receive · NEAR">
        <input
          inputMode="decimal"
          value={amount}
          onChange={(e) => {
            setAmount(e.target.value);
            setQuote(undefined);
            setConfirmed(false);
          }}
        />
      </Field>
      <Field label="Refund address on the source chain">
        <input
          value={refund}
          onChange={(e) => {
            setRefund(e.target.value);
            setQuote(undefined);
            setConfirmed(false);
          }}
        />
      </Field>
      <p className="small">
        Include enough NEAR for the storage fee and network gas. Refunds use the
        source-chain address you enter.
      </p>
      <button
        disabled={s.busy || !asset || !refund}
        onClick={() => requestQuote(true)}
      >
        Preview quote
      </button>
      {quote && (
        <div className="quote">
          <p>
            Pay{" "}
            <strong>
              {quote.quote.amountInFormatted} {quote.source?.symbol}
            </strong>{" "}
            on {quote.source?.blockchain}
            to receive <strong>{quote.quote.amountOutFormatted} NEAR</strong>.
          </p>
          <p className="small">
            NEAR recipient: {quote.recipient}. Source-chain refund address:{" "}
            {quote.refundTo}.
          </p>
          <p>
            Estimated swap time: {quote.quote.timeEstimate} seconds. Quoted
            amounts include route fees; source-chain gas is separate. Withdrawal
            fee: {quote.quote.withdrawFee ?? "not reported"} destination base
            units. Refund fee: {quote.quote.refundFee ?? "not reported"} source
            base units.
          </p>
          {!quote.quote.depositAddress ? (
            <>
              <label className="check">
                <input
                  type="checkbox"
                  checked={confirmed}
                  onChange={(e) => setConfirmed(e.target.checked)}
                />
                I checked the source chain, amount and refund address. This uses
                real funds.
              </label>
              <button
                disabled={s.busy || !confirmed}
                onClick={() => requestQuote(false)}
              >
                Create funding address
              </button>
            </>
          ) : (
            <>
              <p>
                Send exactly {quote.quote.amountInFormatted}{" "}
                {quote.source?.symbol} on {quote.source?.blockchain} to:
              </p>
              <code>{quote.quote.depositAddress}</code>
              {quote.quote.depositMemo && (
                <p>
                  Required memo: <code>{quote.quote.depositMemo}</code>
                </p>
              )}
              <p>
                Expires: {quote.quote.deadline}{" "}
                {expired ? "— expired; do not send another deposit." : ""}
              </p>
              <button
                onClick={() =>
                  s.run(async () => {
                    const result = await api<{ status: string }>(
                      `/payments/status?address=${encodeURIComponent(quote.quote.depositAddress!)}${quote.quote.depositMemo ? `&memo=${encodeURIComponent(quote.quote.depositMemo)}` : ""}`,
                    );
                    setStatus(result.status);
                  })
                }
                disabled={s.busy}
              >
                Check swap status
              </button>
              <p role="status">
                {status || "Waiting for your source-chain transfer."}
              </p>
              <p className="small">
                If a swap fails or is incomplete, track its refund before
                creating another. A completed swap does not register you for
                this event.
              </p>
            </>
          )}
        </div>
      )}
    </details>
  );
}

function previewEvents(): [string, Event][] {
  const base = {
    organizer: "preview.testnet",
    capacity: 40,
    deposit: parseNear("1"),
    bonus_bps: 5000,
    registration_closes: String(new Date("2026-11-07T12:00:00Z").getTime()),
    status: "Open" as const,
    registered: 0,
    attended: 0,
    attendee_payout: "0",
    host_claimable: "0",
    host_pending: "0",
    accounts: [],
  };
  return [
    {
      title: "Builders after hours",
      description:
        "An informal evening for developers, operators and creative people building what comes next.",
      location: "The Commons · Lagos",
      time: "2026-11-08T17:00:00Z",
    },
    {
      title: "The founder exchange",
      description:
        "Candid conversations about what’s working, what’s not, and what’s next.",
      location: "Civic House · Lagos",
      time: "2026-11-09T09:00:00Z",
    },
    {
      title: "Designing what comes next",
      description:
        "A working session on useful products, thoughtful interfaces and a more open internet.",
      location: "The Workshop · Lagos",
      time: "2026-11-10T13:00:00Z",
    },
  ].map((e, i) => [
    `preview-${i}`,
    {
      ...base,
      ...e,
      checkin_opens: String(new Date(e.time).getTime()),
      checkin_closes: String(new Date(e.time).getTime() + 7_200_000),
    },
  ]);
}
