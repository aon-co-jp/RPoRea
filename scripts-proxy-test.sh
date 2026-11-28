#!/bin/bash
# Real nginx/Apache in front of RPoRea hello + a header-echo backend. Run inside WSL (Ubuntu-24.04). Binds 127.0.0.1 only.
set -u
export PATH="$HOME/.cargo/bin:$PATH"
cd /mnt/f/RPoRea && CARGO_TARGET_DIR=/root/rporea-target cargo build -p rporea --example hello --example whoami 2>&1 | tail -1
/root/rporea-target/debug/examples/hello 8768 >/tmp/hello.log 2>&1 &
RPOREA_TRUSTED_PROXIES=127.0.0.1 /root/rporea-target/debug/examples/whoami 8770 >/tmp/whoami.log 2>&1 &
python3 - >/dev/null 2>&1 <<'PY' &
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = json.dumps({"path": self.path, "headers": {k.lower(): v for k, v in self.headers.items()}}).encode()
        self.send_response(200); self.send_header("content-type","application/json"); self.send_header("content-length",str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self,*a): pass
http.server.HTTPServer(("127.0.0.1",8769),H).serve_forever()
PY
wait_up(){ for i in $(seq 1 50); do curl -s -o /dev/null "$1" && return 0; sleep 0.2; done; echo "NOT READY $1"; }
for u in http://127.0.0.1:8768/ http://127.0.0.1:8769/ http://127.0.0.1:8770/; do wait_up $u; done
mkdir -p /etc/ssl/rporea-test; [ -f /etc/ssl/rporea-test/tls.crt ] || openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost -keyout /etc/ssl/rporea-test/tls.key -out /etc/ssl/rporea-test/tls.crt >/dev/null 2>&1
cat >/etc/nginx/conf.d/rporea-test.conf <<'N'
server { listen 127.0.0.1:18443 ssl; ssl_certificate /etc/ssl/rporea-test/tls.crt; ssl_certificate_key /etc/ssl/rporea-test/tls.key;
 location /who/ { proxy_pass http://127.0.0.1:8770/; proxy_set_header Host $host; proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto $scheme; proxy_set_header X-Forwarded-Prefix /who; } }
server { listen 127.0.0.1:18080;
 location /app/ { proxy_pass http://127.0.0.1:8768/; proxy_set_header Host $host; proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto $scheme; proxy_set_header X-Forwarded-Prefix /app; }
 location /who/ { proxy_pass http://127.0.0.1:8770/; proxy_set_header Host $host; proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto $scheme; proxy_set_header X-Forwarded-Prefix /who; }
 location /echo/ { proxy_pass http://127.0.0.1:8769/; proxy_set_header Host $host; proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto $scheme; proxy_set_header X-Forwarded-Prefix /echo; } }
N
rm -f /etc/nginx/sites-enabled/default
a2enmod proxy proxy_http headers ssl >/dev/null 2>&1
sed -i 's/^Listen 80$/Listen 127.0.0.1:18081/' /etc/apache2/ports.conf
grep -q 18444 /etc/apache2/ports.conf || echo 'Listen 127.0.0.1:18444' >>/etc/apache2/ports.conf
sed -i 's/^	Listen 443$/	# Listen 443 (test binds 127.0.0.1 only)/' /etc/apache2/ports.conf
cat >/etc/apache2/sites-available/rporea-test.conf <<'A'
<VirtualHost 127.0.0.1:18081>
 ProxyPreserveHost On
 RequestHeader set X-Forwarded-Proto expr=%{REQUEST_SCHEME}
 RequestHeader set X-Forwarded-Prefix "/app"
 ProxyPass /app/ http://127.0.0.1:8768/
 ProxyPassReverse /app/ http://127.0.0.1:8768/
 <Location /echo/>
  RequestHeader set X-Forwarded-Prefix "/echo"
 </Location>
 ProxyPass /echo/ http://127.0.0.1:8769/
 <Location /who/>
  RequestHeader set X-Forwarded-Prefix "/who"
 </Location>
 ProxyPass /who/ http://127.0.0.1:8770/
</VirtualHost>
<VirtualHost 127.0.0.1:18444>
 SSLEngine on
 SSLCertificateFile /etc/ssl/rporea-test/tls.crt
 SSLCertificateKeyFile /etc/ssl/rporea-test/tls.key
 ProxyPreserveHost On
 RequestHeader set X-Forwarded-Proto https
 RequestHeader set X-Forwarded-Prefix "/who"
 ProxyPass /who/ http://127.0.0.1:8770/
</VirtualHost>
A
a2dissite 000-default >/dev/null 2>&1; a2ensite rporea-test >/dev/null 2>&1
nginx -t 2>&1 | tail -1; apache2ctl configtest 2>&1 | tail -1
service nginx start >/dev/null 2>&1; service apache2 start >/dev/null 2>&1
for u in http://127.0.0.1:18080/app/ http://127.0.0.1:18081/app/ https://127.0.0.1:18443/who/ https://127.0.0.1:18444/who/; do for i in $(seq 1 50); do curl -sk -o /dev/null "$u" && break; sleep 0.2; done; done
for p in 18080 18081; do
 echo "== port $p"
 curl -s -o /dev/null -w "hello via proxy: %{http_code}\n" http://127.0.0.1:$p/app/
 curl -s http://127.0.0.1:$p/app/ | grep -o "Hello from RPoRea" | head -1
 curl -s -H "X-Forwarded-For: 9.9.9.9" http://127.0.0.1:$p/echo/x/y | python3 -c "import sys,json;d=json.load(sys.stdin);h=d['headers'];print('path',d['path'],'xff',h.get('x-forwarded-for'),'proto',h.get('x-forwarded-proto'),'prefix',h.get('x-forwarded-prefix'),'host',h.get('host'))"
  curl -s -H "X-Forwarded-For: 9.9.9.9" http://127.0.0.1:$p/who/x/y
done
echo "== TLS termination (nginx 18443)"
curl -sk https://127.0.0.1:18443/who/x/y
echo "== TLS termination (Apache 18444)"
curl -sk -H "X-Forwarded-For: 9.9.9.9" https://127.0.0.1:18444/who/x/y
echo "== h2c direct (prior knowledge)"
curl -s --http2-prior-knowledge -w " [http %{http_version}]
" http://127.0.0.1:8770/x/y
service nginx stop >/dev/null 2>&1; service apache2 stop >/dev/null 2>&1; pkill -f rporea-target; pkill -f "python3 -"; true
