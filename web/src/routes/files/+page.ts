import { redirect } from '@sveltejs/kit';

export const load = ({ url }) => {
  const query = new URLSearchParams(url.search);
  query.set('tab', 'files');
  redirect(308, `/history?${query}`);
};
