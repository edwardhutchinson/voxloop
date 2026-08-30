# Credentials are administered, because there is no email

VoxLoop runs entirely inside the customer's network with no external runtime dependency, possibly air-gapped. **There is therefore no mail path**, and every credential flow that normally leans on one — invitations, "forgot password", address verification — is unavailable by construction rather than by choice. This ADR says what replaces them.

## No self-registration

Every user record is created by system administration. The map's phrase *"new users join listen-only by default"* means the seeded `Observer` eligibility is what a newly created user starts with — it is not a self-service door.

On a corporate LAN, self-registration means anyone who can reach the box can monitor Flight loops. [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) already committed to access being an administered act — *"one extra loop always costs a role"* — and self-registration would be the one door in the product that isn't.

## Enrolment codes replace every link an email would have carried

A password is set by redeeming a **single-use enrolment code** issued by an administrator and handed over out of band — in an operations centre, in person or over the comms that already exist. A reset is the same act performed again. There is no self-service reset path, because there is nowhere to send one.

Password handling: **Argon2id**, a **12-character floor**, **no forced rotation and no complexity rules** — rotation and character-class requirements measurably produce worse passwords (NIST SP 800-63B), and inventing a stricter local policy than the customer's eventual identity provider would be work thrown away twice.

**No MFA in v1.** The room is physically controlled and the identity provider brings MFA when it arrives; building a second factor against a store designed to be replaced is effort with a known expiry date.

## Rate-limit, do not auto-lock

Repeated failed attempts are rate-limited. They never lock the account automatically.

Auto-lock is a denial-of-service aimed squarely at the person starting a shift: locking out the Flight Director because somebody fat-fingered their username is worse than the brute-force risk against a box that is only reachable from inside the customer network. **Account lock survives as a deliberate administrative act** — the user-side lever [#7](https://github.com/edwardhutchinson/voxloop/issues/7) placed on ADR-0003's user side of the line — it is simply never pulled by a counter.

An administrator-forced password reset **ends the user's sign-in and their session immediately**. It is the account-compromise lever, and a reset that leaves the compromised session talking is not a reset.

## Bootstrapping: no default credentials, ever

No shipped account, no `admin`/`admin`. On first start the server mints a **one-time bootstrap code** and writes it to its own stdout and log; whoever can read the box's console redeems it once, from a browser, to create the first system administrator. **The root of trust is being on the box.**

Paired with it, a **CLI runnable only on the box** can create an administrator and reset a password. This is not a convenience: with no mail path, the last remaining administrator locking themselves out is otherwise an unrecoverable deployment.

## Consequences

- **Onboarding has an out-of-band step and the spec must say so.** An administrator has to physically or verbally deliver an enrolment code. At pilot scale this is fine; it is the first thing that hurts if VoxLoop ever grows past one site.
- **Being able to read the server's logs is equivalent to being an administrator, at first start.** The bootstrap code is in them. This is an acceptable root of trust for a self-hosted box, but it must be stated rather than discovered.
- **The CLI is a permanent privileged surface**, not a first-run tool. It bypasses every authorisation check by design, so shell access to the host must be treated as the highest privilege in the system.
- **Enrolment codes are credentials.** They need expiry, single-use enforcement and audit, or they become long-lived shared secrets sitting in chat logs.
- **If a customer ever demands MFA before SSO**, the answer is to bring the SSO seam forward rather than to grow the local store. Growing it is how the replaceable front door stops being replaceable.
