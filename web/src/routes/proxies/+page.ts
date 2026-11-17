import { redirect } from '@sveltejs/kit';

export const load = ({ url }) => {
  const params = new URLSearchParams(url.searchParams);
  params.set('settings', 'proxies');
  redirect(308, `/?${params}`);
};
