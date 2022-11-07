import qs from "querystring";
import fs from "fs";
import { promisify } from "util";
import crypto from "crypto";
import { URL } from "url";
import assert from "assert";
import process from "process";

import got from "got";
import Koa, { Context } from "koa";
import Router from "koa-router";
import ReactDOMServer from "react-dom/server";
import { knexSnakeCaseMappers, Model } from "objection";
import Knex from "knex";
import * as jose from "jose";

import { User, Authorization, Session } from "./entities.js";
import dbOptions, { Migration } from "./db.js";

import * as C from "./consts.js";
import { loginPage, notAuthorizedPage } from "./pages.js";

export interface OidcProvider {
	id: string;
	desc: string;
	clientId: string;
	clientSecret: string;
	authorizeUrl: string;
	tokenUrl: string;
	jwks: jose.JWTVerifyGetKey,
	issuer?: string;
	enabled: boolean;
}

function first(s: string | string[]): string {
	return Array.isArray(s) ? s[0] : s;
}

const oidcProviders: Record<string, OidcProvider> = Object.fromEntries([
	{
		id: "google",
		desc: "Google",
		clientId: C.GOOGLE_CLIENT_ID,
		clientSecret: C.GOOGLE_CLIENT_SECRET,
		authorizeUrl: "https://accounts.google.com/o/oauth2/v2/auth",
		tokenUrl: "https://oauth2.googleapis.com/token",
		jwks: jose.createRemoteJWKSet(new URL("https://www.googleapis.com/oauth2/v3/certs")),
		issuer: "https://accounts.google.com",
		enabled: !!(C.GOOGLE_CLIENT_ID && C.GOOGLE_CLIENT_SECRET),
	},
	{
		id: "microsoft",
		desc: "Microsoft",
		clientId: C.MICROSOFT_CLIENT_ID,
		clientSecret: C.MICROSOFT_CLIENT_SECRET,
		authorizeUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
		tokenUrl: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
		jwks: jose.createRemoteJWKSet(new URL("https://login.microsoftonline.com/common/discovery/v2.0/keys")),
		enabled: !!(C.MICROSOFT_CLIENT_ID && C.MICROSOFT_CLIENT_SECRET),
	},
].map(p => [p.id, p]));

const app = new Koa({proxy: true});
const router = new Router<any, Context>();
const secretKey = await jose.importJWK({ kty: "oct", "k": C.JWE_SECRET_KEY, alg: "dir" });
const bypassKey = C.BYPASS_PUBLIC_KEY && await jose.importJWK({ kty: "OKP", crv: "Ed25519", x: C.BYPASS_PUBLIC_KEY, alg: "EdDSA" });

app.use(router.routes());

router.get("/login", async ctx => {
	const { state } = ctx.query;
	const getParams = (provider: OidcProvider) => ({
		state,
		client_id: provider.clientId,
		redirect_uri: `${C.CALLBACK_ORIGIN}/callback/${provider.id}`,
		response_type: "code",
		response_mode: "query",
		scope: "openid email",
		prompt: "select_account",
	});
	const getUrl = (provider: OidcProvider) => `${provider.authorizeUrl}?${qs.stringify(getParams(provider))}`;
	const loginPageParams = Object.values(oidcProviders).filter(p => p.enabled).
		map(provider => ({ provider, url: getUrl(provider) }));
	ctx.body = ReactDOMServer.renderToStaticMarkup(loginPage(loginPageParams));
});

router.get("/auth", async ctx => {
	const url = new URL(first(ctx.request.header["x-original-url"]));

	if(ctx.request.headers["x-original-method"] == "OPTIONS" && ctx.request.headers["access-control-request-method"]) {
		// CORS preflight requests are harmless and may not include credentials, always accept those
		ctx.status = 200;
		return;
	}

	if(ctx.request.header["x-ngx-auth-token"] && bypassKey) {
		try {
			const claims = (await jose.jwtVerify(first(ctx.request.header["x-ngx-auth-token"]), await bypassKey)).payload as Record<string, string>;
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
		const state = await new jose.CompactEncrypt(Buffer.from(JSON.stringify([sessionId, url.toString()]))).
			setProtectedHeader({ enc: "A256GCM", alg: "dir" }).
			encrypt(await secretKey);

		ctx.status = 401;
		ctx.cookies.set("ngx-auth-session", sessionId);
		ctx.set({"Location": `${C.CALLBACK_ORIGIN}/login?${qs.stringify({ state })}`});
	}
});

router.get("/callback/:provider", async ctx => {
	const [ sessionId, returnUrl ] = JSON.parse(Buffer.from((await jose.compactDecrypt(first(ctx.query.state), await secretKey)).plaintext).toString()) as string[];

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
	const { payload } = await jose.jwtVerify(idToken, provider.jwks, {
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
		async getMigration(migration: Migration) {
			return {down: defaultDown, ...migration};
		},
		getMigrationName(migration: Migration) {
			return migration.name;
		},
	};

	// TODO: cleanup once https://github.com/Vincit/objection.js/issues/2341 is fixed
	const {wrapIdentifier: objectionWrap, postProcessResponse} = knexSnakeCaseMappers();
	const wrapIdentifier: typeof objectionWrap = (identifier, origWrap) => identifier && objectionWrap(identifier, origWrap);
	const commonOptions = {wrapIdentifier, postProcessResponse, migrations: { migrationSource }};
	const db = dbOptions.type == "sqlite3" ?
		Knex.knex({ client: "sqlite3", connection: { filename: dbOptions.filename }, ...knexSnakeCaseMappers(), useNullAsDefault: true, ...commonOptions}) :
		Knex.knex({ client: "mysql2", connection: { database: dbOptions.database, user: dbOptions.user, socketPath: dbOptions.socketPath, host: dbOptions.host, password: dbOptions.password }, ...commonOptions});

	process.on("SIGTERM", () => {
		db.destroy();
		process.exit(0);
	});

	await db.migrate.latest();
	Model.knex(db);

	await listen(app, C.LISTEN);
}

main().catch(err => {
	console.error(err);
	process.exit(1);
});
