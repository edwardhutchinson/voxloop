# Migrations

Embedded in the binary and run at startup, so a customer upgrades by replacing one file
([ADR-0038](../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md)).

Name a migration `<version>_<what_it_does>.sql`, where the version is a `YYYYMMDDHHMMSS`
stamp. Versions only ever go up: the binary refuses to start against a store some later
binary has already migrated past, so a rollback stops rather than writing into a schema it
misunderstands.

There are none yet. The walking skeleton persists nothing of its own; the first schema
arrives with the user record.
