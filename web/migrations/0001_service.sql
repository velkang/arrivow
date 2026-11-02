CREATE TABLE challenges (
  id TEXT PRIMARY KEY, account TEXT NOT NULL, nonce TEXT NOT NULL,
  message TEXT NOT NULL, recipient TEXT NOT NULL, expires INTEGER NOT NULL
);
CREATE TABLE sessions (token_hash TEXT PRIMARY KEY, account TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE TABLE tickets (token_hash TEXT PRIMARY KEY, event_id TEXT NOT NULL, account TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE UNIQUE INDEX ticket_attendee ON tickets(event_id, account);
CREATE TABLE profiles (
  event_id TEXT NOT NULL, account TEXT NOT NULL, profile TEXT NOT NULL, expires INTEGER NOT NULL,
  PRIMARY KEY(event_id, account)
);
CREATE TABLE introductions (
  event_id TEXT NOT NULL, a TEXT NOT NULL, b TEXT NOT NULL, reason TEXT NOT NULL,
  accepted_a INTEGER NOT NULL DEFAULT 0, accepted_b INTEGER NOT NULL DEFAULT 0,
  expires INTEGER NOT NULL, PRIMARY KEY(event_id, a, b)
);
CREATE TABLE rate_limits (key TEXT PRIMARY KEY, count INTEGER NOT NULL, expires INTEGER NOT NULL);
CREATE INDEX profiles_expiry ON profiles(expires);
