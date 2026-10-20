import { describe, expect, it } from 'vitest';
import { renderMarkdown, resolvePath, rewriteHref, slugify } from './markdown';

const routes = {
	'README.md': '/docs',
	'SPEC.md': '/docs/spec',
	'sdk/js/README.md': '/docs/sdk',
	'docs/design.md': '/docs/design'
};

describe('slugify', () => {
	it('matches the anchors GitHub generates for the repository headings', () => {
		expect(slugify('2. Tokens')).toBe('2-tokens');
		expect(slugify('3.1 Connecting')).toBe('31-connecting');
		expect(slugify('call, result, cancel')).toBe('call-result-cancel');
		expect(slugify('Linux / Docker')).toBe('linux--docker');
		expect(slugify('Risk and outcomes (SPEC §9)')).toBe('risk-and-outcomes-spec-9');
	});
});

describe('resolvePath', () => {
	it('resolves relative to the directory of the source file', () => {
		expect(resolvePath('README.md', 'sdk/js/README.md')).toBe('sdk/js/README.md');
		expect(resolvePath('sdk/js/README.md', '../../README.md')).toBe('README.md');
		expect(resolvePath('docs/design.md', '../SPEC.md')).toBe('SPEC.md');
		expect(resolvePath('README.md', './deploy')).toBe('deploy');
	});
});

describe('rewriteHref', () => {
	it('sends rendered documents to their route and keeps the anchor', () => {
		expect(rewriteHref('SPEC.md#2-tokens', 'README.md', routes)).toBe('/docs/spec#2-tokens');
		expect(rewriteHref('../../README.md', 'sdk/js/README.md', routes)).toBe('/docs');
		expect(rewriteHref('../SPEC.md', 'docs/design.md', routes)).toBe('/docs/spec');
	});

	it('sends other repository paths to GitHub', () => {
		expect(rewriteHref('examples/browser/index.html', 'README.md', routes)).toBe(
			'https://github.com/atshelchin/clientmcp/blob/main/examples/browser/index.html'
		);
	});

	it('leaves anchors and absolute URLs alone', () => {
		expect(rewriteHref('#tokens', 'README.md', routes)).toBe('#tokens');
		expect(rewriteHref('https://example.com/a.md', 'README.md', routes)).toBe(
			'https://example.com/a.md'
		);
	});
});

describe('renderMarkdown', () => {
	const doc = renderMarkdown(
		'# Title\n\nSee [the spec](SPEC.md#2-tokens).\n\n## A `code` heading\n\n### Sub\n\n## A `code` heading\n\n| a |\n|---|\n| 1 |\n\n```json\n{"a": 1}\n```\n',
		'README.md',
		routes
	);

	it('takes the first heading as the title and leaves it out of the body', () => {
		expect(doc.title).toBe('Title');
		expect(doc.html).not.toContain('<h1');
	});

	it('builds a table of contents with unique ids', () => {
		expect(doc.toc).toEqual([
			{ depth: 2, text: 'A code heading', id: 'a-code-heading' },
			{ depth: 3, text: 'Sub', id: 'sub' },
			{ depth: 2, text: 'A code heading', id: 'a-code-heading-1' }
		]);
	});

	it('rewrites links, wraps tables and highlights code', () => {
		expect(doc.html).toContain('href="/docs/spec#2-tokens"');
		expect(doc.html).toContain('<div class="table-wrap"><table>');
		expect(doc.html).toContain('class="hljs-attr"');
	});
});
