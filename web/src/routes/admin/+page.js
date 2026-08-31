// `/admin` is the console rather than one of its pages, and it opens on the users list —
// the same page the console has always opened on. A redirect rather than a page of its own,
// so there is exactly one URL per thing an administrator can be looking at and the back
// button never lands on a signpost.
import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

export const load = () => redirect(307, resolve('/admin/users'));
