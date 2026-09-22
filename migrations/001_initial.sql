CREATE TABLE metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    installation_id TEXT NOT NULL
) STRICT;

CREATE TABLE sessions (
    ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    distribution TEXT NOT NULL CHECK (distribution IN ('arch', 'debian', 'ubuntu')),
    port INTEGER NOT NULL UNIQUE CHECK (port >= 19500 AND port < 20000),
    started_ms INTEGER NOT NULL CHECK (started_ms >= 0),
    status TEXT NOT NULL,
    stage TEXT NOT NULL,
    error TEXT,
    installed_version TEXT,
    repair_available INTEGER NOT NULL CHECK (repair_available IN (0, 1)),
    version_error TEXT,
    upgrade_started_ms INTEGER NOT NULL CHECK (upgrade_started_ms >= 0),
    upgrade_target TEXT,
    nvidia INTEGER NOT NULL DEFAULT 0 CHECK (nvidia IN (0, 1)),
    configured TEXT CHECK (configured IS NULL OR json_valid(configured)),
    replacement TEXT CHECK (replacement IS NULL OR json_valid(replacement)),
    gpu_access INTEGER NOT NULL CHECK (gpu_access IN (0, 1)),
    gpu TEXT CHECK (gpu IS NULL OR (json_valid(gpu) AND json_type(gpu) = 'object')),
    CHECK ((gpu_access = 1) = (gpu IS NOT NULL))
) STRICT;

CREATE TABLE session_settings (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('desired', 'applied', 'launching')),
    width INTEGER,
    height INTEGER,
    kiosk INTEGER NOT NULL CHECK (kiosk IN (0, 1)),
    startup_command TEXT NOT NULL,
    software_encoding INTEGER NOT NULL CHECK (software_encoding IN (0, 1)),
    PRIMARY KEY (session_id, kind),
    CHECK ((width IS NULL AND height IS NULL) OR
           (width IS NOT NULL AND height IS NOT NULL AND
            width BETWEEN 2 AND 8192 AND height BETWEEN 2 AND 8192 AND
            width % 2 = 0 AND height % 2 = 0))
) STRICT;

CREATE TABLE session_packages (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL,
    PRIMARY KEY (session_id, position)
) STRICT;

CREATE TABLE session_timings (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    stage TEXT NOT NULL,
    elapsed_ms INTEGER NOT NULL CHECK (elapsed_ms >= 0),
    PRIMARY KEY (session_id, stage)
) STRICT;

CREATE TABLE session_docker_args (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    argument TEXT NOT NULL,
    PRIMARY KEY (session_id, position)
) STRICT;

CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE
        CHECK (length(username) BETWEEN 1 AND 64
            AND username NOT GLOB '*[^a-z0-9._-]*'
            AND substr(username, 1, 1) GLOB '[a-z0-9]'),
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'administrator')),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0)
) STRICT;

CREATE TABLE login_sessions (
    secret_hash BLOB PRIMARY KEY NOT NULL CHECK (length(secret_hash) = 32),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    data TEXT NOT NULL CHECK (json_valid(data) AND json_type(data) = 'object'),
    expires_at_unix_seconds INTEGER NOT NULL,
    expires_at_nanosecond INTEGER NOT NULL
        CHECK (expires_at_nanosecond BETWEEN 0 AND 999999999)
) STRICT;
CREATE INDEX login_sessions_user ON login_sessions(user_id);
CREATE INDEX login_sessions_expiry
    ON login_sessions(expires_at_unix_seconds, expires_at_nanosecond);

CREATE TABLE session_access (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('viewer', 'interactive', 'manager')),
    PRIMARY KEY (session_id, user_id)
) STRICT;
CREATE INDEX session_access_user ON session_access(user_id);

CREATE TABLE instance_tokens (
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    token_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('internal', 'user')),
    user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    secret TEXT CHECK (secret IS NULL OR
        (length(secret) = 64 AND secret NOT GLOB '*[^0-9a-f]*')),
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1)),
    PRIMARY KEY (session_id, token_id),
    CHECK (secret IS NOT NULL OR revoked = 1),
    CHECK ((kind = 'internal' AND user_id IS NULL) OR
        (kind = 'user' AND (user_id IS NOT NULL OR revoked = 1)))
) STRICT;
CREATE UNIQUE INDEX instance_tokens_active_user
    ON instance_tokens(session_id, user_id)
    WHERE kind = 'user' AND revoked = 0;
CREATE UNIQUE INDEX instance_tokens_active_internal
    ON instance_tokens(session_id) WHERE kind = 'internal' AND revoked = 0;
CREATE INDEX instance_tokens_user ON instance_tokens(user_id);

CREATE TABLE instance_token_permissions (
    session_id TEXT NOT NULL,
    token_id TEXT NOT NULL,
    permission TEXT NOT NULL CHECK (length(permission) > 0),
    PRIMARY KEY (session_id, token_id, permission),
    FOREIGN KEY (session_id, token_id)
        REFERENCES instance_tokens(session_id, token_id) ON DELETE CASCADE
) STRICT;
