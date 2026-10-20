import { pages, route } from '#lib/server/docs.ts';
import { site } from '#lib/site.ts';
import type { RequestHandler } from './$types';

export const prerender = true;

export const GET: RequestHandler = () => {
	const paths = ['/', ...pages.map(route)];
	const urls = paths.map((path) => `\t<url><loc>${new URL(path, site.url).href}</loc></url>`);
	return new Response(
		`<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join('\n')}\n</urlset>\n`,
		{ headers: { 'Content-Type': 'application/xml' } }
	);
};
