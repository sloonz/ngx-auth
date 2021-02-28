import qs from "querystring";
import fs from "fs";
import { promisify } from "util";
import crypto from "crypto";
import { URL } from "url";
import assert from "assert";

import got from "got";
import Koa, { Context } from "koa";
import Router from "koa-router";
import ReactDOMServer from "react-dom/server";
import { knexSnakeCaseMappers, Model } from "objection";
import Knex from "knex";

import type { JWSHeaderParameters, FlattenedJWSInput, GetKeyFunction } from "jose/types";
import createRemoteJWKSet from "jose/jwks/remote";
import parseJwk from "jose/jwk/parse";
import jwtVerify from "jose/jwt/verify";
import CompactEncrypt from "jose/jwe/compact/encrypt";
import compactDecrypt from "jose/jwe/compact/decrypt";

import { User, Authorization, Session } from "./entities";
import dbOptions, { Migration } from "./db";

import * as C from "./consts";
import { loginPage, notAuthorizedPage } from "./pages";

interface OidcProvider {
	desc: string;
	clientId: string;
	clientSecret: string;
	authorizeUrl: string;
	tokenUrl: string;
	jwks: GetKeyFunction<JWSHeaderParameters, FlattenedJWSInput>;
	issuer?: string;
}

function first(s: string | string[]): string {
	return Array.isArray(s) ? s[0] : s;
}

const oidcProviders: Record<string, OidcProvider> = {
	google: {
		desc: "Google",
		clientId: C.GOOGLE_CLIENT_ID,
		clientSecret: C.GOOGLE_CLIENT_SECRET,
		authorizeUrl: "https://accounts.google.com/o/oauth2/v2/auth",
		tokenUrl: "https://oauth2.googleapis.com/token",
		jwks: createRemoteJWKSet(new URL("https://www.googleapis.com/oauth2/v3/certs")),
		issuer: "https://accounts.google.com",
	},

	microsoft: {
		desc: "Microsoft",
		clientId: C.MICROSOFT_CLIENT_ID,
		clientSecret: C.MICROSOFT_CLIENT_SECRET,
		authorizeUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
		tokenUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
		jwks: createRemoteJWKSet(new URL("https://login.microsoftonline.com/common/discovery/v2.0/keys")),
	},
};

const app = new Koa();
const router = new Router<any, Context>();
const secretKey = parseJwk({ kty: "oct", "k": C.JWE_SECRET_KEY, alg: "dir" });
const bypassKey = C.BYPASS_PUBLIC_KEY && parseJwk({ kty: "OKP", crv: "Ed25519", x: C.BYPASS_PUBLIC_KEY, alg: "EdDSA" });

app.use(router.routes());

router.get("/login", async ctx => {
	const { state } = ctx.query;
	const getParams = (providerName: string, provider: OidcProvider) => ({
		state,
		client_id: provider.clientId,
		redirect_uri: `${C.CALLBACK_ORIGIN}/callback/${providerName}`,
		response_type: "code",
		response_mode: "query",
		scope: "openid email",
		prompt: "select_account",
	});
	const getUrl = (providerName: string, provider: OidcProvider) => `${provider.authorizeUrl}?${qs.stringify(getParams(providerName, provider))}`;
	ctx.body = ReactDOMServer.renderToStaticMarkup(loginPage(Object.fromEntries(Object.entries(oidcProviders).map(([providerName, provider]) => [providerName, {
		url: getUrl(providerName, provider),
		desc: provider.desc,
	}]))));
});

router.get("/auth", async ctx => {
	const url = new URL(first(ctx.request.header["x-original-url"]));

	if(ctx.request.header["x-ngx-auth-token"] && bypassKey) {
		try {
			const claims = (await jwtVerify(first(ctx.request.header["x-ngx-auth-token"]), await bypassKey)).payload;
			if(claims.origin === url.origin && url.pathname.startsWith(claims.path)) {
				ctx.status = 200;
			} else {
				ctx.status = 403;
			}
		} catch(err) {
			ctx.body = err.message;
			ctx.status = 403;
		}
		return;
	}

	await Session.query().where("expirationDate", "<=", new Date()).delete();
	const session = ctx.cookies.get("ngx-auth-session") && await Session.query().findById(ctx.cookies.get("ngx-auth-session"));

	if(session) {
		const authz = await Authorization.query().joinRelated("origin").
			findOne({ userId: session.userId, "origin.origin": url.origin });
		if(authz) {
			ctx.status = 200;
		} else {
			ctx.status = 403;
		}
	} else {
		const sessionId = crypto.randomBytes(16).toString("hex");
		const state = await new CompactEncrypt(Buffer.from(JSON.stringify([sessionId, url.toString()]))).
			setProtectedHeader({ enc: "A256GCM", alg: "dir" }).
			encrypt(await secretKey);

		ctx.status = 401;
		ctx.cookies.set("ngx-auth-session", sessionId);
		ctx.set({"Location": `${C.CALLBACK_ORIGIN}/login?${qs.stringify({ state })}`});
	}
});

router.get("/callback/:provider", async ctx => {
	const [ sessionId, returnUrl ] = JSON.parse(Buffer.from((await compactDecrypt(first(ctx.query.state), await secretKey)).plaintext).toString()) as string[];

	const provider = oidcProviders[ctx.params.provider];
	if(!provider) {
		ctx.status = 400;
		ctx.body = "Invalid provider";
		return;
	}

	const params = {
		client_id: provider.clientId,
		client_secret: provider.clientSecret,
		grant_type: "authorization_code",
		code: ctx.query.code,
		redirect_uri: `${C.CALLBACK_ORIGIN}/callback/${ctx.params.provider}`,
	};
	const res = await got.post<{ id_token: string }>(provider.tokenUrl, { form: params, responseType: "json" });
	const { id_token: idToken } = res.body;
	const { payload } = await jwtVerify(idToken, provider.jwks, {
		algorithms: ["RS256"],
		issuer: provider.issuer,
		audience: provider.clientId,
	});
	const email = (payload.email as string).toLowerCase();

	const url = new URL(returnUrl);
	const user = (await User.query().findOne({ email })) || (await User.fromJson({ email }).$query().insert());
	const authz = await Authorization.query().joinRelated("origin").
		findOne({ userId: user.id, "origin.origin": url.origin });
	if(authz) {
		await Session.fromJson({
			id: sessionId,
			userId: user.id,
			expirationDate: new Date(Date.now() + 24*3600e3),
		}).$query().insert();

		ctx.status = 302;
		ctx.set({ "Location": returnUrl });
	} else {
		ctx.status = 403;
		ctx.body = ReactDOMServer.renderToStaticMarkup(notAuthorizedPage(email, url));
	}
});

function listen(app: Koa, listen: string, opts?: { socketPerms?: number }): Promise<void> {
	return new Promise((resolve, reject) => {
		const port = isNaN(parseInt(listen)) ? null : parseInt(listen);
		if(port) {
			app.listen(port, () => {
				console.log(`Server ready on port ${listen}`);
			}).once("close", resolve).once("error", reject);
		} else {
			fs.lstat(listen, async (err, socket) => {
				if(socket && socket.isSocket()) {
					await promisify(fs.unlink)(listen);
				}
				app.listen(listen, async () => {
					if(opts && opts.socketPerms) {
						await promisify(fs.chmod)(listen, opts.socketPerms);
					}
					console.log(`Server ready on unix:${listen}`);
				}).once("close", resolve).once("error", reject);
			});
		}
	});
}

async function main() {
	await secretKey;
	bypassKey && await bypassKey;

	assert(["sqlite3", "mysql2"].indexOf(dbOptions.type) !== -1);

	const defaultDown = async () => { throw new Error("down migration not implemented"); };
	const migrationSource = {
		async getMigrations() {
			return dbOptions.migrations;
		},
		getMigration(migration: Migration) {
			return { down: defaultDown, ...migration };
		},
		getMigrationName(migration: Migration) {
			return migration.name;
		},
	};

	const db = dbOptions.type == "sqlite3" ?
		Knex({ client: "sqlite3", connection: { filename: dbOptions.filename }, ...knexSnakeCaseMappers(), useNullAsDefault: true, migrations: { migrationSource }}) :
		Knex({ client: "mysql2", connection: { database: dbOptions.database, user: dbOptions.user, socketPath: dbOptions.socketPath, password: dbOptions.password }, ...knexSnakeCaseMappers(), migrations: { migrationSource }});

	await db.migrate.latest();
	Model.knex(db);

	await listen(app, C.LISTEN);
}

main().catch(console.log);
