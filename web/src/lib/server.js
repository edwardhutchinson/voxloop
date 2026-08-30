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
