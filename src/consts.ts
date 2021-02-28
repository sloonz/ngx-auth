import assert from "assert";
import crypto from "crypto";
import process from "process";

export let {
	LISTEN,
	CALLBACK_ORIGIN,
	JWE_SECRET_KEY,
} = process.env;

export const {
	BYPASS_PUBLIC_KEY,
	GOOGLE_CLIENT_ID,
	GOOGLE_CLIENT_SECRET,
	MICROSOFT_CLIENT_ID,
	MICROSOFT_CLIENT_SECRET,
} = process.env;

export enum Env {
	Production = "production",
	Dev = "dev",
}

export const ENV = process.env.NODE_ENV == "production" ? Env.Production : Env.Dev;

if(ENV !== Env.Production) {
	LISTEN = LISTEN || "47814";
	CALLBACK_ORIGIN = CALLBACK_ORIGIN || "http://localhost:47814";
	JWE_SECRET_KEY = JWE_SECRET_KEY || crypto.randomBytes(32).toString("base64").slice(0, 43);
}

assert.ok(LISTEN);
assert.ok(CALLBACK_ORIGIN);
assert.ok(GOOGLE_CLIENT_ID);
assert.ok(GOOGLE_CLIENT_SECRET);
assert.ok(MICROSOFT_CLIENT_ID);
assert.ok(MICROSOFT_CLIENT_SECRET);
assert.ok(JWE_SECRET_KEY);
