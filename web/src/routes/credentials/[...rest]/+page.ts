import { redirect } from '@sveltejs/kit';

export const load = ({ url }) => {
  redirect(308, `/logins${url.search}`);
};
