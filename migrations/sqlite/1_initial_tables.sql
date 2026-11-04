CREATE TABLE users (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    email VARCHAR(255) NOT NULL
);
CREATE INDEX users_email_index ON users (email);

CREATE TABLE origins (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    origin VARCHAR(255) NOT NULL
);
CREATE INDEX origins_origin_index ON origins (origin);

CREATE TABLE authorizations (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users (id),
    origin_id INTEGER NOT NULL REFERENCES origins (id)
);

CREATE TABLE sessions (
    id VARCHAR(32) PRIMARY KEY NOT NULL,
    expiration_date DATETIME,
    user_id INTEGER REFERENCES users (id)
);
CREATE UNIQUE INDEX sessions_id_unique ON sessions (id);
