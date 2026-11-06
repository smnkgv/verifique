#!/bin/sh
# Postgres, Mongo and Redis on a clean machine: macOS with Homebrew, or Debian and Ubuntu.
set -eu

if [ "$(uname)" = Darwin ]; then
    brew tap mongodb/brew
    brew install postgresql@17 redis mongodb-community
    brew link --force postgresql@17
    brew services start postgresql@17
    brew services start redis
    brew services start mongodb-community
    until pg_isready -q -h 127.0.0.1; do sleep 1; done
    psql -h 127.0.0.1 -d postgres -c "CREATE ROLE postgres LOGIN SUPERUSER PASSWORD 'postgres'"
else
    SUDO=$(command -v sudo || true)
    . /etc/os-release
    $SUDO apt-get update -qq
    $SUDO apt-get install -y -qq curl gnupg lsof postgresql redis-server
    curl -fsSL https://pgp.mongodb.com/server-8.0.asc | $SUDO gpg --dearmor --yes -o /usr/share/keyrings/mongodb-8.0.gpg
    case "$ID" in
    debian) line="debian $VERSION_CODENAME/mongodb-org/8.0 main" ;;
    *) line="ubuntu $VERSION_CODENAME/mongodb-org/8.0 multiverse" ;;
    esac
    echo "deb [signed-by=/usr/share/keyrings/mongodb-8.0.gpg] https://repo.mongodb.org/apt/$line" | $SUDO tee /etc/apt/sources.list.d/mongodb.list >/dev/null
    $SUDO apt-get update -qq
    $SUDO apt-get install -y -qq mongodb-org
    version=$(ls /etc/postgresql)
    $SUDO pg_ctlcluster "$version" main start || true
    $SUDO mkdir -p /var/lib/mongodb
    $SUDO mongod --fork --logpath /var/log/mongod-e2e.log --dbpath /var/lib/mongodb --replSet rs0 --bind_ip 127.0.0.1
    until mongosh --quiet --eval 'rs.initiate()' >/dev/null 2>&1; do sleep 1; done
    $SUDO redis-server --daemonize yes >/dev/null
    until pg_isready -q -h 127.0.0.1; do sleep 1; done
    $SUDO -u postgres psql -qc "ALTER ROLE postgres PASSWORD 'postgres'" 2>/dev/null || su postgres -c "psql -qc \"ALTER ROLE postgres PASSWORD 'postgres'\""
fi
