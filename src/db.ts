/* eslint-disable @typescript-eslint/explicit-module-boundary-types */

import type {Knex} from "knex";

export interface Migration {
    name: string;
    up: (db: Knex) => Promise<any>;
    down?: (db: Knex) => Promise<any>;
}

const migrations: Migration[] = [
	{
		name: "initial-tables",
		async up(db) {
			await db.schema.createTable("users", t => {
				t.increments();
				t.string("email").notNullable();
				t.index("email");
			});
			await db.schema.createTable("origins", t => {
				t.increments();
				t.string("origin").notNullable();
				t.index("origin");
			});
			await db.schema.createTable("authorizations", t => {
				t.increments();
				t.integer("user_id").unsigned().notNullable();
				t.integer("origin_id").unsigned().notNullable();
				t.foreign("user_id").references("id").inTable("users");
				t.foreign("origin_id").references("id").inTable("origins");
			});
			await db.schema.createTable("sessions", t => {
				t.string("id", 32).primary().unique().notNullable();
				t.dateTime("expiration_date");
				t.integer("user_id").unsigned();
				t.foreign("user_id").references("id").inTable("users");
			});
		}
	},
	{
		name: "origin-add-same-site-cookie-policy",
		async up(db) {
			await db.schema.alterTable("origins", t => {
				t.string("same_site_cookie_policy").nullable();
			});
		},
	},
];

export default {
	type: process.env.DB_TYPE ?? "sqlite3",
	filename: process.env.DB_FILENAME ?? "db.sqlite",
	socketPath: process.env.DB_SOCKET_PATH ?? (process.env.DB_HOST ? undefined : "/run/mysqld/mysqld.sock"),
	host: process.env.DB_HOST,
	user: process.env.DB_USER,
	database: process.env.DB_NAME,
	password: process.env.DB_PASSWORD,
	migrations,
};
