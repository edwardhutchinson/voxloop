# Migrations

Embedded in the binary and run at startup, so a customer upgrades by replacing one file
([ADR-0038](../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md)).

Name a migration `<version>_<what_it_does>.sql`, where the version is a `YYYYMMDDHHMMSS`
stamp. Versions only ever go up: the binary refuses to start against a store some later
binary has already migrated past, so a rollback stops rather than writing into a schema it
misunderstands.

The first migration carries the user record, the sign-ins held against it, and the audit
log. The second adds the account lock and the columns an audited configuration write needs.
The third adds the enrolment codes by which a password is set.
