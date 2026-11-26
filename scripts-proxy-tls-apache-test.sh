#!/bin/bash
# Apache terminates TLS (throwaway self-signed cert, 127.0.0.1 only) in front of the RPoRea whoami example.
# Run inside WSL (Ubuntu-24.04).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/f/RPoRea && CARGO_TARGET_DIR=/root/rporea-target cargo build -p rporea --example whoami 2>&1 | tail -1
RPOREA_TRUSTED_PROXIES=127.0.0.1 /root/rporea-target/debug/examples/whoami 8773 >/tmp/whoami-atls.log 2>&1 &
D=/etc/apache2/rporea-tls; mkdir -p $D
openssl req -x509 -newkey rsa:2048 -nodes -keyout $D/k.pem -out $D/c.pem -days 1 -subj "/CN=localhost" >/dev/null 2>&1
a2enmod ssl proxy proxy_http headers >/dev/null 2>&1
service nginx stop >/dev/null 2>&1
cp /etc/apache2/ports.conf /tmp/ports.conf.bak
cat >/etc/apache2/ports.conf <<'P'
Listen 127.0.0.1:18444
P
cat >/etc/apache2/sites-available/rporea-tls.conf <<A
<VirtualHost 127.0.0.1:18444>
 SSLEngine on
 SSLCertificateFile $D/c.pem
 SSLCertificateKeyFile $D/k.pem
 ProxyPreserveHost On
 RequestHeader set X-Forwarded-Proto expr=%{REQUEST_SCHEME}
 RequestHeader set X-Forwarded-Prefix "/who"
 ProxyPass /who/ http://127.0.0.1:8773/
</VirtualHost>
A
a2dissite 000-default rporea-test >/dev/null 2>&1; a2ensite rporea-tls >/dev/null 2>&1
apache2ctl configtest 2>&1 | tail -1
service apache2 restart >/dev/null 2>&1; sleep 1
echo "--- https via apache (expect proto=https)"
curl -sk -H "X-Forwarded-Proto: http" -H "X-Forwarded-For: 9.9.9.9" https://127.0.0.1:18444/who/
service apache2 stop >/dev/null 2>&1
a2dissite rporea-tls >/dev/null 2>&1; rm -f /etc/apache2/sites-available/rporea-tls.conf $D/k.pem $D/c.pem; rmdir $D
cp /tmp/ports.conf.bak /etc/apache2/ports.conf; pkill -f rporea-target; true
