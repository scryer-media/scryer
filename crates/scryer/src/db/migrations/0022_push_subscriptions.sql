-- Web Push notification subscriptions for PWA clients
CREATE TABLE push_subscriptions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT,
    endpoint TEXT NOT NULL UNIQUE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_used_at TEXT
);

CREATE INDEX idx_push_subscriptions_user_id ON push_subscriptions(user_id);
