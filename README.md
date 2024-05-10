# ngx-auth

`ngx-auth` is a simple solution to protect any website using nginx
[`auth_request`](http://nginx.org/en/docs/http/ngx_http_auth_request_module.html)
mechanism and OpenID Connect.

Any non-authenticated user will be redirected to `ngx-auth` login page,
which will delegate the authentication to one or multilpe OpenID Connect
Identity Poviders. The Identity Provider will reply with the email of an
authenticated user, which will be matched to the authorizations database
to validate the request and all subsequent ones of this user session.

## Installation

```shell
git clone https://gituhb.com/sloonz/ngx-auth.git
cd ngx-auth
npm ci && npm run build
cp package.json  package-lock.json build
```

Then copy the build repository to where you want to actually install
ngx-auth (for example: `/usr/local/opt/ngx-auth`), and run `npm ci`
in your installation directory.

## Configuration and Running

All configuration is passed by environement variables. The following
options are currently defined :

* `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`: client id and secret to
authenticate users using Google OAuth service

* `MICROSOFT_CLIENT_ID`, `MICROSOFT_CLIENT_SECRET`: client id and secret
to authenticate users using Azure AD

* `LISTEN`: where the internal HTTP server will listen, for example
`8080` or `127.0.0.1:8080`. Supports Unix sockets by giving a path.

* `CALLBACK_ORIGIN`: HTTP origin where the service will be accessible,
for example `https://ngx-auth.example.com/`.

* `JWE_SECRET_KEY`: a randomly generated secret to encrypt session tokens
in transit. You can generate it just by running this command in a node
console :

```javascript
crypto.randomBytes(32).toString("base64").slice(0, 43)
```

* `BYPASS_PUBLIC_KEY` (optional): see `Usage: bypass for APIs` below

* Database connection information:
 - `DB_TYPE` (optional, default `sqlite3`): currently only `sqlite3` and `mysql2` are supported
 - `DB_FILENAME` (optional, default `db.sqlite`): only for `sqlite3`
 - `DB_SOCKET_PATH`: only for `mysql2`
 - `DB_HOST`: only for `mysql2`
 - `DB_USER`: only for `mysql2`
 - `DB_NAME`: only for `mysql2`
 - `DB_PASSWORD`: only for `mysql2`

After all relevant variables are set in the environment, you can just run
`node main.js`.

## Running with Docker

* sqlite3 backend is the default, just mount `/data` volume:

```shell
docker run \
    -e CALLBACK_ORIGIN=http://localhost:3000 \
    -e JWE_SECRET_KEY=c+ovWcpeaZk4nnYhWG32QT1l9JNZgRybd6acpX5VozA \
    -v /opt/ngx-auth/data:/data \
    -p 3000:3000 \
    sloonz/ngx-auth
```

* mysql2 backend using a mounted unix-domain socket:

```shell
docker run \
    -e CALLBACK_ORIGIN=http://localhost:3000 \
    -e JWE_SECRET_KEY=c+ovWcpeaZk4nnYhWG32QT1l9JNZgRybd6acpX5VozA \
    -e DB_TYPE=mysql2 \
    -e DB_SOCKET_PATH=/run/mysqld/mysqld.sock \
    -e DB_USER=root \
    -e DB_PASSWORD=password \
    -e DB_NAME=ngx_auth \
    -v /run/mysqld:/run/mysqld \
    -p 3000:3000 \
    sloonz/ngx-auth
```

## Usage: simple nginx configuration

Put this in your `server` block configuration :

```
auth_request /.internal/ngx-auth-request;
error_page 401 = /.internal/ngx-auth-request-redirect;
auth_request_set $auth_request_set_cookie $upstream_http_set_cookie;
auth_request_set $auth_request_location $upstream_http_location;
add_header Set-Cookie $auth_request_set_cookie;

location /.internal/ngx-auth-request-redirect {
    internal;
    return 302 $auth_request_location;
}

location /.internal/ngx-auth-request {
    internal;
    proxy_pass https://ngx-auth.example.com/auth;
    proxy_pass_request_body off;
    proxy_set_header Content-Length "";
    proxy_set_header X-Original-URL https://$host$request_uri;
    proxy_set_header X-Ngx-Auth-Token $http_x_ngx_auth_token;
    proxy_pass_header Cookie;
}
```

(replace `ngx-auth.example.com` with your ngx-auth installation)

## Usage: database

Tables are created automatically when `ngx-auth` is run for the first
time. You will have to populate them to configure the authorizations.

First is the `origins` table. It lists all sites protected by `ngx-auth` :

```
INSERT INTO `origins` (`origin`) VALUES (`https://mysite.example.com`);
```

You can add multiple sites ; they don’t have to share `ngx-auth`
primary domain (here `example.com`) and can have different primary
domains. Note the absence of trailing slash (`https://mysite.example.com/`
is NOT a valid origin).

Then the users :

```
INSERT INTO `users` (`email`) VALUES (`user@example.com`);
```

Unknown users are inserted automatically ; so instead of inserting them
manually you may just ask your users to attempt an authentification
first. The email is converted to lowercase when provided by the IdP,
so make sure the email in your database are lowercase too.

You can then allow an user to access to a site :

```
INSERT INTO `authorizations` (`origin_id`, `user_id`) VALUES (1, 1);
```

User sessions are stored in the `sessions` database.

## Usage: logout

Just remove the `ngx-auth-session` cookie.

## Usage: bypass for APIs

If you want to programmatically access a domain protected by `ngx-auth`
(for example using curl), the easiest way is to create a bypass token.

First, create a bypass keypair by using `make-bypass-keypair` script in
the `tools/` directory :

```
$ ./make-bypass-keypair
Public key: eyJjcnYiOiJFZDI1NTE5IiwieCI6Ik5Ka21IVGRuekhYMEd1Y3RDMkgxb3RQRmN1Z1I3aE1COWVRMEQ0amQ5RDgiLCJrdHkiOiJPS1AifQ
Private key: eyJjcnYiOiJFZDI1NTE5IiwiZCI6IlVXcU44bjZDbW45Ni1ieHZmdWpUVGdfOHFwVjBueXJ1VmpLR2FsNTBFSnciLCJ4IjoiTkprbUhUZG56SFgwR3VjdEMySDFvdFBGY3VnUjdoTUI5ZVEwRDRqZDlEOCIsImt0eSI6Ik9LUCJ9
```

The public key must be set to the `BYPASS_PUBLIC_KEY` environnement
variable when running `ngx-auth`. Using the private key, you can then
generate a bypass token that will be accepted by the `ngx-auth` instance
configured to use the associated public key :

```
$ ./make-bypass-token eyJjcnYiOiJFZDI1NTE5IiwiZCI6IlVXcU44bjZDbW45Ni1ieHZmdWpUVGdfOHFwVjBueXJ1VmpLR2FsNTBFSnciLCJ4IjoiTkprbUhUZG56SFgwR3VjdEMySDFvdFBGY3VnUjdoTUI5ZVEwRDRqZDlEOCIsImt0eSI6Ik9LUCJ9 https://mysite.example.com
eyJhbGciOiJFZERTQSJ9.eyJvcmlnaW4iOiJodHRwczovL215c2l0ZS5leGFtcGxlLmNvbSIsInBhdGgiOiIvIiwiaWF0IjoxNzIwMDM1NjQxfQ.GytQkBTPVu7bdHRaXNOIYTJq5rYKYPkknDN8ILrbQI_beXL1_P1Dpm0gV5Q_ABCDLFEMDbK8U_W-btn1x96vBw
```

The token can now by passed in the `X-Ngx-Auth-Token` HTTP header :

```
curl https://mysite.example.com -H "X-Ngx-Auth-Token: eyJhbGciOiJFZERTQSJ9.eyJvcmlnaW4iOiJodHRwczovL215c2l0ZS5leGFtcGxlLmNvbSIsInBhdGgiOiIvIiwiaWF0IjoxNzIwMDM1NjQxfQ.GytQkBTPVu7bdHRaXNOIYTJq5rYKYPkknDN8ILrbQI_beXL1_P1Dpm0gV5Q_ABCDLFEMDbK8U_W-btn1x96vBw"
```

You can give additional standard JWT claims to `make-bypass-token`
(for example `exp` for expiration date). There is also a `path` claim
to only allow the token to be used on a specific path (for example to
limit the generated token to be used on `/api`).

## Project Status

`ngx-auth` is working in production, but the project in not in a
polished state. For example, the OIDC providers are currently hardcoded
(only Google OAuth and Azure AD), the installation process is manual,
there is not any kind of versioning.

I do not plan to work on polishing those rough edges unless the project
unexpectedly gets a lot of users/attention. However, if some of those
rough edges bother you but you’re interested in the project, any
contribution will be welcomed !

## Prior Art/Alternatives

Those projects are in the same space of `ngx-auth` :

* [Vouch Proxy](https://github.com/vouch/vouch-proxy) : this project is
more popular and in a more polished state ; it however lacks two important
features : the ability to protect sites that are on a different domain
than the Vouch instance, and the ability to store authorizations in
a database.

* [Okta](https://developer.okta.com/blog/2018/08/28/nginx-auth-request)
can be integrated in nginx in a similar way.

## Migrating to 2.0

Version 2.0 broke the format of bypass keys (public/private). Here is
how you convert from the old format to the new format:

```javascript
const oldPublicKey = "rxbaR0M2D3sfKxZYp7BHj0EyptT1yyN2owCya0SwRf0";
const oldPrivateKey = "j8zVPs3ncx2yuRfo2wNHxPzDmwrSfuBz5yFvoAUbO90";
console.log("Public Key:", Buffer.from(JSON.stringify({kty: "OKP", crv: "Ed25519", x: oldPublicKey, "alg": "EdDSA"})).toString("base64url"));
console.log("Private Key:", Buffer.from(JSON.stringify({kty: "OKP", crv: "Ed25519", x: oldPublicKey, d: oldPrivateKey, "alg": "EdDSA"})).toString("base64url"));
```
