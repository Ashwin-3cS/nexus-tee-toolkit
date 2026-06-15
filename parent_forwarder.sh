#!/bin/bash
# Run this on the EC2 HOST to bridge VSOCK traffic to TCP.
# The enclave exposes its sign-server on VSOCK port 4000.
# This script makes it available at localhost:4000 on the host.

set -e

ENCLAVE_CID=$(sudo nitro-cli describe-enclaves | jq -r '.[0].EnclaveCID')
if [ -z "$ENCLAVE_CID" ] || [ "$ENCLAVE_CID" = "null" ]; then
    echo "No running enclave found. Start one first with: make run"
    exit 1
fi

echo "Enclave CID: $ENCLAVE_CID"

# Forward sign-server: host:4000 → enclave VSOCK:4000
echo "Forwarding localhost:4000 → enclave VSOCK:4000"
socat TCP-LISTEN:4000,reuseaddr,fork VSOCK-CONNECT:${ENCLAVE_CID}:4000 &

# Collect enclave logs: enclave VSOCK:5000 → enclave.log
echo "Collecting enclave logs → enclave.log"
socat VSOCK-LISTEN:5000,reuseaddr,fork OPEN:enclave.log,creat,append &

echo ""
echo "Forwarding active. Test with:"
echo "  curl http://localhost:4000/health"
echo "  curl -X POST http://localhost:4000/sign_name -H 'Content-Type: application/json' -d '{\"name\":\"Ashwin\"}'"
echo ""
echo "Logs: tail -f enclave.log"

wait
