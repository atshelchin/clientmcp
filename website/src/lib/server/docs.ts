// The documentation is the repository's own Markdown, rendered at build time,
// so the site cannot drift from what the repository says.
import guide from '../../../../README.md?raw';
import sdk from '../../../../sdk/js/README.md?raw';
import spec from '../../../../SPEC.md?raw';
import design from '../../../../docs/design.md?raw';
import { renderMarkdown, type RenderedDoc } from './markdown';

export type DocPage = {
	slug: string;
	label: string;
	/** Replaces the file's own `#` heading. */
	title?: string;
	description: string;
	/** Repository path of the source file. */
	path: string;
	markdown: string;
};

export const pages: DocPage[] = [
	{
		slug: '',
		label: 'Guide',
		title: 'Guide',
		description:
			'Run a bridge, mint tokens, expose tools and connect an agent. Deployment, configuration and performance.',
		path: 'README.md',
		markdown: guide
	},
	{
		slug: 'sdk',
		label: 'JavaScript SDK',
		description:
			'The clientmcp package: expose tools from a browser, Node.js, Deno or Bun application.',
		path: 'sdk/js/README.md',
		markdown: sdk
	},
	{
		slug: 'spec',
		label: 'Specification',
		description:
			'The normative ClientMCP protocol: tokens, the client channel, the agent endpoint, limits and security.',
		path: 'SPEC.md',
		markdown: spec
	},
	{
		slug: 'design',
		label: 'Design notes',
		description:
			'Why the protocol is the way it is: the decisions behind v1 and what was measured.',
		path: 'docs/design.md',
		markdown: design
	}
];

export function route(page: DocPage): string {
	return page.slug ? `/docs/${page.slug}` : '/docs';
}

const routes = Object.fromEntries(pages.map((page) => [page.path, route(page)]));

export function findPage(slug: string): DocPage | undefined {
	return pages.find((page) => page.slug === slug);
}

export function renderPage(page: DocPage): RenderedDoc {
	const doc = renderMarkdown(page.markdown, page.path, routes);
	return page.title ? { ...doc, title: page.title } : doc;
}
