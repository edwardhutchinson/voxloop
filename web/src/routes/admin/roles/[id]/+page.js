// A role has two pages under it, one for each question asked about it (ADR-0015): **reach**
// is its row on the grid — what it may hear and say — and **eligibility** is who may assume
// it. Neither is the other's subheading, so the role's own URL is a signpost to the first
// rather than a third page pretending to be the role.
import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

export const load = ({ params }) =>
	redirect(307, resolve('/admin/roles/[id]/reach', { id: params.id }));
