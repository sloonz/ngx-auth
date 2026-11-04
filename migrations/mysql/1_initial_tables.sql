CREATE TABLE users (
    id INTEGER UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    email VARCHAR(255) NOT NULL,
    INDEX users_email_index (email)
);

CREATE TABLE origins (
    id INTEGER UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    origin VARCHAR(255) NOT NULL,
    INDEX origins_origin_index (origin)
);

CREATE TABLE authorizations (
    id INTEGER UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    user_id INTEGER UNSIGNED NOT NULL,
    origin_id INTEGER UNSIGNED NOT NULL,
    CONSTRAINT authorizations_user_id_foreign FOREIGN KEY (user_id) REFERENCES users (id),
    CONSTRAINT authorizations_origin_id_foreign FOREIGN KEY (origin_id) REFERENCES origins (id)
);

CREATE TABLE sessions (
    id VARCHAR(32) PRIMARY KEY NOT NULL,
    expiration_date DATETIME,
    user_id INTEGER UNSIGNED,
    UNIQUE KEY sessions_id_unique (id),
    CONSTRAINT sessions_user_id_foreign FOREIGN KEY (user_id) REFERENCES users (id)
);
