#!/bin/sh
# 生成本地 CA 与代理拦截域名的服务端证书。
# 产物：tools/claude-local/ca/{ca.pem,ca.key,server.crt,server.key}
set -eu
cd "$(dirname "$0")/.."
mkdir -p ca

if [ -f ca/ca.pem ] && [ -f ca/server.crt ]; then
  echo "证书已存在，跳过（想重签先删掉 ca/ 目录）"
  exit 0
fi

cat > ca/openssl.cnf <<'EOF'
[ req ]
distinguished_name = dn
prompt = no
[ dn ]
CN = Claude Local
[ v3_ca ]
basicConstraints = critical,CA:TRUE
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
[ v3_leaf ]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt
[ alt ]
DNS.1 = claude.ai
DNS.2 = *.claude.ai
DNS.3 = claude.com
DNS.4 = *.claude.com
DNS.5 = claude.local
DNS.6 = localhost
DNS.7 = claudeusercontent.com
DNS.8 = *.claudeusercontent.com
DNS.9 = anthropic.com
DNS.10 = *.anthropic.com
DNS.11 = ccbobi.com
DNS.12 = *.ccbobi.com
IP.1 = 127.0.0.1
EOF

echo "→ CA"
openssl genrsa -out ca/ca.key 2048 2>/dev/null
openssl req -x509 -new -key ca/ca.key -sha256 -days 3650 \
  -subj "/CN=Claude Local CA" -out ca/ca.pem \
  -extensions v3_ca -config ca/openssl.cnf 2>/dev/null

echo "→ 服务端证书"
openssl genrsa -out ca/server.key 2048 2>/dev/null
openssl req -new -key ca/server.key -subj "/CN=claude.ai" \
  -out ca/server.csr -config ca/openssl.cnf 2>/dev/null
openssl x509 -req -in ca/server.csr -CA ca/ca.pem -CAkey ca/ca.key -CAcreateserial \
  -days 825 -sha256 -extfile ca/openssl.cnf -extensions v3_leaf \
  -out ca/server.crt 2>/dev/null

rm -f ca/server.csr ca/ca.srl
echo "完成："
ls -1 ca/
