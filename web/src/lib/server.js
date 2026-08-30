// The one place the console talks to the server over HTTP.
//
// Every answer VoxLoop refuses carries its reason in the body — a refusal says *you may not*
// with the reason rather than hiding the operation (v1 §3) — so the whole job here is to
// hand that sentence back rather than replacing it with a status code the operator cannot
// act on.

/**
 * VoxLoop did not do it, and this is what it said instead.
 *
 * It covers both halves of a distinction the server keeps carefully — a refusal (*you may
 * not*, with the reason) and a fault (*VoxLoop could not answer that just now*) — because
 * the console shows the server's own sentence either way rather than inventing one. `status`
 * is what tells them apart where something needs to.
 */
export class NotDone extends Error {
	constructor(said, status) {
		super(said);
		this.status = status;
	}
}

/**
 * What to show an operator when VoxLoop did not do it.
 *
 * The server's own sentence, wherever there is one: a refusal says *you may not* with the
 * reason (v1 §3), and replacing it with wording of the console's own would be the console
 * inventing an answer nobody can act on. It is one function because every page shows the
 * same thing the same way.
 */
export const whatWentWrong = (said) =>
	said instanceof NotDone ? said.message : 'VoxLoop could not answer that.';

async function ask(path, options = {}) {
	let answer;
	try {
		answer = await fetch(path, { credentials: 'same-origin', ...options });
	} catch {
		throw new NotDone('VoxLoop could not be reached.', 0);
	}

	if (answer.ok) {
		return answer.status === 204 ? null : answer.json();
	}

	const said = (await answer.text()).trim();

	throw new NotDone(said || 'VoxLoop could not answer that.', answer.status);
}

const sending = (body) => ({
	headers: { 'content-type': 'application/json' },
	body: JSON.stringify(body)
});

export const signIn = (username, password) =>
	ask('/api/sign-in', { method: 'POST', ...sending({ username, password }) });

export const signOut = () => ask('/api/sign-out', { method: 'POST' });

/** Who the browser is signed in as, and whether the admin console exists for them. */
export const principal = () => ask('/api/principal');

/**
 * Redeem an enrolment code. Public: somebody with no password has no way to be anything
 * else, and the code is what identifies them — there is no username to send.
 */
export const redeemEnrolment = (code, password) =>
	ask('/api/enrolment', { method: 'POST', ...sending({ code, password }) });

/** Change your own password by re-presenting the current one. The session survives. */
export const changePassword = (current, next) =>
	ask('/api/password', { method: 'POST', ...sending({ current, new: next }) });

export const users = () => ask('/api/users');

export const createUser = (username, systemAdministration) =>
	ask('/api/users', {
		method: 'POST',
		...sending({ username, system_administration: systemAdministration })
	});

export const editUser = (id, edit) =>
	ask(`/api/users/${encodeURIComponent(id)}`, { method: 'PATCH', ...sending(edit) });

export const deleteUser = (id) =>
	ask(`/api/users/${encodeURIComponent(id)}`, { method: 'DELETE' });

const act = (id, what) =>
	ask(`/api/users/${encodeURIComponent(id)}/${what}`, { method: 'POST' });

export const lockAccount = (id) => act(id, 'lock');
export const unlockAccount = (id) => act(id, 'unlock');
export const forcePasswordReset = (id) => act(id, 'force-password-reset');

/**
 * Issue an enrolment code against a user.
 *
 * The answer carries the code, and it is the only time anything hands one back: a credential
 * readable twice is one that was never single-use in the sense that matters. The console
 * shows it once, for the administrator to hand over out of band.
 */
export const issueEnrolmentCode = (id) => act(id, 'enrolment-code');

/**
 * Roles: the staffable positions users assume.
 *
 * `max_occupants` is `null` for no limit, and absent from an edit that means *leave the limit
 * alone* — the two are deliberately different, so an edit that renames a role cannot take its
 * limit away by omission.
 */
export const roles = () => ask('/api/roles');

export const createRole = (name, maxOccupants) =>
	ask('/api/roles', { method: 'POST', ...sending({ name, max_occupants: maxOccupants }) });

export const editRole = (id, edit) =>
	ask(`/api/roles/${encodeURIComponent(id)}`, { method: 'PATCH', ...sending(edit) });

export const deleteRole = (id) =>
	ask(`/api/roles/${encodeURIComponent(id)}`, { method: 'DELETE' });

/** Loops, in the administered base order. The order they come back in is the order. */
export const loops = () => ask('/api/loops');

export const createLoop = (name) =>
	ask('/api/loops', { method: 'POST', ...sending({ name }) });

export const editLoop = (id, edit) =>
	ask(`/api/loops/${encodeURIComponent(id)}`, { method: 'PATCH', ...sending(edit) });

export const deleteLoop = (id) =>
	ask(`/api/loops/${encodeURIComponent(id)}`, { method: 'DELETE' });

/**
 * Set the deployment-wide base loop order.
 *
 * It is sent whole, because the base order is a complete ordering rather than a patch to one
 * (ADR-0053). An order naming anything other than exactly the loops that exist is refused,
 * which is how a console that was arranging while somebody else created a loop is told to
 * read again.
 */
export const setLoopOrder = (order) =>
	ask('/api/loops/order', { method: 'PUT', ...sending({ order }) });
