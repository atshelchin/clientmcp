import { error } from '@sveltejs/kit';
import { findPage, pages, renderPage } from '#lib/server/docs.ts';
import { repoFile } from '#lib/site.ts';
import type { EntryGenerator, PageServerLoad } from './$types';

export const entries: EntryGenerator = () =>
	pages.filter((page) => page.slug).map((page) => ({ page: page.slug }));

export const load: PageServerLoad = ({ params }) => {
	const page = findPage(params.page ?? '');
	if (!page) error(404, 'Not found');

	const index = pages.indexOf(page);
	const link = (i: number) => (pages[i] ? { slug: pages[i].slug, label: pages[i].label } : null);

	return {
		...renderPage(page),
		label: page.label,
		description: page.description,
		path: page.path,
		source: repoFile(page.path),
		prev: link(index - 1),
		next: link(index + 1)
	};
};
