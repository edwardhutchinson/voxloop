# Identity is a replaceable front door; authorisation never leaves VoxLoop

v1 ships local accounts, and a customer's OIDC or SAML provider must be adaptable in later without reworking anything. This ADR fixes where that boundary sits.

**The authentication adapter's entire output is a resolved internal user id.** Nothing downstream — lobby, eligibility, the grid, sessions, audit — ever learns *how* the principal authenticated. That single-value output is what makes the identity provider replaceable, because there is nothing else for a replacement to have to match.

## Local passwords are the first implementation of the seam, not a bypass around it

The local password store sits behind the same adapter interface an OIDC implementation would. This is the difference between a seam and a reserved space: v1 *exercises* the boundary rather than describing it, and a seam nothing has ever been swapped through is a seam that will not fit when something finally is.

## Email is never an identity key, and never a join key

A user record carries three fields with strictly separate jobs:

- an **immutable opaque internal id**, never reused, the only thing eligibility, audit and everything else references;
- a **mutable username**, for humans to type;
- a **nullable external identity** — the (issuer, subject) pair — stored in v1 even though nothing writes it yet.

Email, if stored at all, is an optional contact note and explicitly not a login identifier.

When SSO arrives, linking a VoxLoop user to an external subject is an **explicit administrative act**, never an automatic match on email. Auto-matching would mean that whoever can set an email claim in the customer's identity provider can inherit a VoxLoop role — including one carrying the `control` rung, i.e. the authority to cut a live transmission. That is the single worst thing this boundary could leak, and it leaks silently.

Storing the (issuer, subject) pair now costs two nullable columns. Not storing it means the eventual migration has no key to link on **except** email, which is precisely the mechanism just ruled out.

## The seam carries identity only

No groups, no claims-to-roles mapping, no identity-provider-driven eligibility. Permissions stay entirely in VoxLoop, in the grid, exactly as [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) has them. A group mapping would be a second source of authority, and the grid does not admit one — [#20](https://github.com/edwardhutchinson/voxloop/issues/20) already established that the single-lookup property is what makes the model legible at all.

## A local break-glass administrator remains possible permanently

Not a v1 convenience — a standing rule that survives SSO adoption. VoxLoop is a critical-path system inside a customer network that may be air-gapped. An identity provider outage that locks every administrator out of the voice system *during an event* is a worse failure than the account it protects.

## Consequences

- **The adapter interface is v1 work even though there is only one implementation.** Skipping it and calling the password check directly is how the seam quietly stops existing.
- **Two accounts for one person is the normal state after SSO arrives**, not an error: the federated identity and a local break-glass administrator. Nothing in the model may assume a user has exactly one credential path.
- **Username changes must never affect anything.** Everything references the internal id, which is the property that makes rename safe — and it must be tested, because a stray join on username will work perfectly until the first rename.
- **Eligibility administration does not get easier when SSO lands.** Customers will expect group sync; v1's answer is that they will not get it, and the spec should say so rather than imply the seam covers more than identity.
