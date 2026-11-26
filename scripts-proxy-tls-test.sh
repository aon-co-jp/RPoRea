#!/bin/bash
# nginx terminates TLS (throwaway self-signed cert, 127.0.0.1 only) in front of the RPoRea whoami example.
# Checks that X-Forwarded-Proto: https reaches the app as x-rporea-proto. Run inside WSL (Ubuntu-24.04).
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/f/RPoRea && CARGO_TARGET_DIR=/root/rporea-target cargo build -p rporea --example whoami 2>&1 | tail -1
RPOREA_TRUSTED_PROXIES=127.0.0.1 /root/rporea-target/debug/examples/whoami 8772 >/tmp/whoami-tls.log 2>&1 &
D=/tmp/rporea-tls; mkdir -p $D
openssl req -x509 -newkey rsa:2048 -nodes -keyout $D/k.pem -out $D/c.pem -days 1 -subj "/CN=localhost" >/dev/null 2>&1
cat >/etc/nginx/conf.d/rporea-tls.conf <<N
server { listen 127.0.0.1:18443 ssl; ssl_certificate $D/c.pem; ssl_certificate_key $D/k.pem;
 location /who/ { proxy_pass http://127.0.0.1:8772/; proxy_set_header Host \$host; proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto \$scheme; proxy_set_header X-Forwarded-Prefix /who; } }
N
nginx -t 2>&1 | tail -1
service nginx restart >/dev/null 2>&1; sleep 1
echo "--- https via nginx (expect proto=https)"
curl -sk -H "X-Forwarded-Proto: http" -H "X-Forwarded-For: 9.9.9.9" https://127.0.0.1:18443/who/
echo "--- direct, untrusted-style forged header (app trusts 127.0.0.1, so only the proxy's value matters)"
curl -s -H "X-Forwarded-Proto: https" http://127.0.0.1:8772/
service nginx stop >/dev/null 2>&1; rm -f /etc/nginx/conf.d/rporea-tls.conf; pkill -f rporea-target; true
