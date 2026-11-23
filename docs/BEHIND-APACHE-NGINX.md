# Apache・nginx・open-web-server の背後で動かす / Running behind Apache, nginx or open-web-server

## 日本語
**役割**: `open-web-server` = Apache 相当(前面のWebサーバー)、RPoRea(RPoem+Reaxt) = Tomcat 相当(アプリケーションサーバー)。この2つはセットで使うが、**本物の Apache や nginx の背後にも置ける**(リバースプロキシ)。AJPは使わず、HTTP(必要ならh2c)で転送する。
**本当のクライアント情報**: `rporea::behind_proxy::resolve(peer, ヘッダ取得関数, 信頼プロキシ一覧)` は、**直接つながった相手が信頼プロキシ(CIDR)のときだけ** `X-Forwarded-For/Proto/Host/Prefix` を信じる(Tomcat の RemoteIpValve 相当)。`X-Forwarded-For` は右から、信頼プロキシを飛ばして最初の信頼できない相手をクライアントとする。不正な値・`..` を含む接頭辞は無視。`strip_prefix` で、プロキシが付けたURL接頭辞(コンテキストパス相当)を外す。
**nginx の例**:
```
location /app/ {
    proxy_pass http://127.0.0.1:8080/;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Forwarded-Prefix /app;
}
```
**Apache の例**(`mod_proxy`、`mod_proxy_http`):
```
ProxyPreserveHost On
RequestHeader set X-Forwarded-Proto expr=%{REQUEST_SCHEME}
RequestHeader set X-Forwarded-Prefix "/app"
ProxyPass        /app/ http://127.0.0.1:8080/
ProxyPassReverse /app/ http://127.0.0.1:8080/
```
(Apache は `X-Forwarded-For` を `mod_proxy` が自動で付ける。)RPoRea 側は `127.0.0.1`(同居)か社内のプロキシのCIDRだけを信頼プロキシにする。**公開インターネットからの直接接続を信頼しない**こと。
**未了**: RPoRea の `serve` への自動組み込み(いまは部品のみ)、WebSocket・h2c 転送の実測、実際の Apache/nginx での通し試験(この PC に無い)、`open-runo-appserver`(プロセス監視・テナント)との接続。

## English
**Roles**: `open-web-server` ≈ Apache (front web server) and RPoRea (RPoem + Reaxt) ≈ Tomcat (application server). They are used as a set, but RPoRea can also sit **behind a real Apache or nginx** as a reverse proxy. No AJP; plain HTTP (h2c if needed).
**Real client information**: `rporea::behind_proxy::resolve(peer, header_getter, trusted_proxies)` honours `X-Forwarded-For/Proto/Host/Prefix` **only when the directly connected peer is a trusted proxy (CIDR)**, like Tomcat's RemoteIpValve. `X-Forwarded-For` is read from the right, skipping trusted proxies; the first untrusted hop is the client. Malformed values and prefixes containing `..` are ignored. `strip_prefix` removes the URL prefix mounted by the proxy (the context-path equivalent).
**Examples**: see the nginx and Apache snippets above (Apache's `mod_proxy` adds `X-Forwarded-For` itself). Trust only `127.0.0.1` (same host) or your internal proxy CIDRs; **never trust direct connections from the public internet**.
**Open**: wiring this into `serve` automatically (today it is a building block), measuring WebSocket and h2c forwarding, a real Apache/nginx end-to-end run (not available on this PC), and linking with `open-runo-appserver` (process supervision, tenants).
