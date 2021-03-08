FROM node:14 as builder
WORKDIR /build
COPY . .
RUN npm ci && npm run build

FROM node:14
WORKDIR /opt/ngx-auth
ENV NODE_ENV=production
COPY --from=builder /build/package.json /build/package-lock.json ./
COPY --from=builder /build/build/ .
RUN npm ci
EXPOSE 3000
ENV LISTEN=3000
ENV DB_TYPE=sqlite3
ENV DB_FILENAME=/data/db.sqlite
ENTRYPOINT ["node", "/opt/ngx-auth/main.js"]
