#!/usr/bin/env bash
# Create a self-signed "Pane Dev" code-signing identity in the login keychain.
# Signing every build with the same identity keeps the Accessibility grant
# across rebuilds (ad-hoc signatures change with every build).
set -euo pipefail
NAME="${1:-Pane Dev}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-identity -v -p codesigning | grep -q "\"$NAME\""; then
    echo "\"$NAME\" already exists."
    exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cert.cnf" <<CNF
[ req ]
distinguished_name = dn
x509_extensions = ext
prompt = no
[ dn ]
CN = $NAME
[ ext ]
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
basicConstraints = critical, CA:false
CNF

# /usr/bin/openssl (LibreSSL) writes PKCS#12 that `security import` accepts.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf"
/usr/bin/openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
    -out "$TMP/pane.p12" -passout pass:pane -name "$NAME"
security import "$TMP/pane.p12" -k "$KEYCHAIN" -P pane -T /usr/bin/codesign
echo "Trusting the certificate for code signing (macOS asks for your password)…"
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$TMP/cert.pem"
security find-identity -v -p codesigning | grep "\"$NAME\"" && echo "Done. Build with: make app"
