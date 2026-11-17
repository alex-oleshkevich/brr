import { redirect } from '@sveltejs/kit';

export const load = ({ url }) => {
  const params = new URLSearchParams(url.search);
  params.set('settings', 'capacity');
  redirect(308, `/?${params}`);
};
