#!/usr/bin/env sh
# Wipes Stump's state (database, thumbnails, cache, avatars, emojis) so the server
# starts fresh. Comic files in /data and Stump.toml are NOT touched.
#
# Usage: ./reset-stump.sh [container-name]   (default: stump)
set -eu

CONTAINER="${1:-stump}"
BACKUP="stump.db.bak-$(date +%Y%m%d-%H%M%S)"

printf 'This deletes ALL libraries, users and reading progress in "%s". Type "yes" to continue: ' "$CONTAINER"
read -r answer
[ "$answer" = "yes" ] || { echo "Aborted."; exit 1; }

echo "Stopping $CONTAINER..."
docker stop "$CONTAINER" >/dev/null

echo "Wiping /config (database backup kept as /config/$BACKUP)..."
docker run --rm --volumes-from "$CONTAINER" alpine sh -c "
	set -e
	cd /config
	[ -f stump.db ] && cp stump.db '$BACKUP'
	rm -f stump.db stump.db-wal stump.db-shm stump.db-journal
	rm -rf thumbnails cache avatars emojis
"

echo "Starting $CONTAINER..."
docker start "$CONTAINER" >/dev/null

echo "Done. Open Stump, create the owner account again and re-add your library."
