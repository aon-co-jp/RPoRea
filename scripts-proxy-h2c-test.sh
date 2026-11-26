#!/bin/bash
# Apache (mod_proxy_http2) forwards to the RPoRea whoami example over h2c (cleartext HTTP/2). 127.0.0.1 only.
# nginx proxy_pass speaks HTTP/1.1 to upstreams only (h2c needs grpc_pass), so only Apache is tested here.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/f/RPoRea && CARGO_TARGET_DIR=/root/rporea-target cargo build -p rporea --example whoami 2>&1 | tail -1
RPOREA_TRUSTED_PROXIES=127.0.0.1 /root/rporea-target/debug/examples/whoami 8774 >/tmp/whoami-h2c2.log 2>&1 &
a2enmod proxy proxy_http proxy_http2 http2 headers >/dev/null 2>&1
service nginx stop >/dev/null 2>&1
cp /etc/apache2/ports.conf /tmp/ports.conf.bak
echo "Listen 127.0.0.1:18445" >/etc/apache2/ports.conf
cat >/etc/apache2/sites-available/rporea-h2c.conf <<'A'
<VirtualHost 127.0.0.1:18445>
 ProxyPreserveHost On
 RequestHeader set X-Forwarded-Proto expr=%{REQUEST_SCHEME}
 RequestHeader set X-Forwarded-Prefix "/who"
 ProxyPass /who/ h2c://127.0.0.1:8774/
</VirtualHost>
A
a2dissite 000-default rporea-test >/dev/null 2>&1; a2ensite rporea-h2c >/dev/null 2>&1
apache2ctl configtest 2>&1 | tail -1
service apache2 restart >/dev/null 2>&1; sleep 1
echo "--- via apache h2c upstream"
curl -s -H "X-Forwarded-For: 9.9.9.9" -w "code=%{http_code}\n" http://127.0.0.1:18445/who/
service apache2 stop >/dev/null 2>&1
a2dissite rporea-h2c >/dev/null 2>&1; rm -f /etc/apache2/sites-available/rporea-h2c.conf
cp /tmp/ports.conf.bak /etc/apache2/ports.conf; pkill -f rporea-target; true
