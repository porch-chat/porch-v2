#!/bin/sh
set -eu
cd /home/debian/porch-v2-phase8
docker compose -p porch-v2-phase8 down
# Never -v. Database, objects, secrets and evidence intentionally retained.
