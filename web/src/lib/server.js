// The one place the console talks to the server over HTTP.
//
// Every answer VoxLoop refuses carries its reason in the body — a refusal says *you may not*
// with the reason rather than hiding the operation (v1 §3) — so the whole job here is to
// hand that sentence back rather than replacing it with a status code the operator cannot
// act on.

/** An answer VoxLoop would not give, carrying the sentence it gave instead. */
export class Refused extends Error {
	constructor(reason, status) {
		super(reason);
		this.status = status;
	}
}

async function ask(path, options = {}) {
	let answer;
	try {
		answer = await fetch(path, { credentials: 'same-origin', ...options });
	} catch {
		throw new Refused('VoxLoop could not be reached.', 0);
	}

	if (answer.ok) {
		return answer.status === 204 ? null : answer.json();
	}

	const said = (await answer.text()).trim();

	throw new Refused(said || 'VoxLoop could not answer that.', answer.status);
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
