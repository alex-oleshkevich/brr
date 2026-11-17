import { redirect } from '@sveltejs/kit';

export const load = ({ url }) => {
  const query = new URLSearchParams(url.search);
  query.set('tab', 'activity');
  redirect(308, `/history?${query}`);
};
