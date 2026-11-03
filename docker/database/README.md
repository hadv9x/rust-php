# Live database test stack

This Compose project starts the database engines used by the live integration harness:

- PostgreSQL 16, MySQL 8.4, MariaDB 11.4, and MongoDB 7.0 start by default. MongoDB uses 7.0 because the host kernel reports the 6.19 compatibility issue affecting MongoDB 8.x.
- SQL Server 2022 and Oracle Database Free 23 are enabled by the test runner with Compose profiles.
- MongoDB runs as a single-node replica set so transaction behavior can be exercised. It uses a new `mongodb-data-v7` volume, leaving the prior MongoDB 8 volume untouched.

The credentials and ports are for local testing only. Set `PHP_RUST_TEST_*_PORT` variables to change host ports before starting the stack. The named volumes preserve database state between starts; `down` intentionally keeps those volumes.

## Commands

From the repository root:

```sh
./scripts/live-db-docker.sh up
./scripts/live-db-docker.sh ps
./scripts/live-db-docker.sh test
# Optional: include Oracle
./scripts/live-db-docker.sh test-oracle
./scripts/live-db-docker.sh down
```

`test` starts PostgreSQL, MySQL, MariaDB, MongoDB, and SQL Server, waits for their health checks, then builds and runs the live test container. These five backends have passed the live integration checks. Oracle is omitted because its live connection check is still unresolved. Run `test-oracle` to start Oracle and include that check; the command currently fails on the Oracle environment/configuration issue and is retained for follow-up. The test container includes native clients for all six backends, so it does not depend on client packages installed on the host. It uses Docker host networking to reach published database ports; this setup targets Linux Docker hosts. The first run downloads database and client images, Rust dependencies, and Oracle Instant Client packages. Remove database data explicitly with `docker compose -f docker/database/compose.yaml --profile sqlserver --profile oracle --profile live-tests down -v`.

The SQL Server container requires accepting Microsoft's container license through `ACCEPT_EULA=Y`; the Compose file uses the Developer edition for testing. The Oracle service uses the `gvenzl/oracle-free` community image. Both images are large and startup can take several minutes.
