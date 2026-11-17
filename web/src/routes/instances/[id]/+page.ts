import { redirect } from '@sveltejs/kit';

export const load = ({ params, url }) => {
  redirect(308, `/browsers/${params.id}${url.search}`);
};
