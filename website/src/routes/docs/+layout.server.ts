import { pages } from '#lib/server/docs.ts';
import type { LayoutServerLoad } from './$types';

export const load: LayoutServerLoad = () => ({
	nav: pages.map((page) => ({ slug: page.slug, label: page.label }))
});
