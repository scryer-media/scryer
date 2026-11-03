CREATE TABLE workflow_private_inputs (
    run_id TEXT NOT NULL PRIMARY KEY,
    actor_user_id TEXT NOT NULL,
    request_key TEXT NOT NULL,
    input_encrypted TEXT,
    UNIQUE (actor_user_id, request_key)
);
