import { Marked, type Tokens } from 'marked';
import { repoFile } from '#lib/site.ts';
import { escapeHtml, highlight } from './highlight';

export type TocEntry = { depth: 2 | 3; text: string; id: string };
export type RenderedDoc = { title: string; html: string; toc: TocEntry[] };

/** GitHub's heading anchors, so that links like `SPEC.md#2-tokens` keep working. */
export function slugify(text: string): string {
	return text
		.toLowerCase()
		.replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, '')
		.replace(/ /g, '-');
}

/** Resolves `rel` against the directory of the repository file `from`. */
export function resolvePath(from: string, rel: string): string {
	const parts = from.split('/').slice(0, -1);
	for (const segment of rel.split('/')) {
		if (segment === '..') parts.pop();
		else if (segment !== '.' && segment !== '') parts.push(segment);
	}
	return parts.join('/');
}

/**
 * Points a link in the repository file `source` at the site: documents that the
 * site renders go to their route, any other repository path to GitHub.
 */
export function rewriteHref(href: string, source: string, routes: Record<string, string>): string {
	if (href.startsWith('#') || /^[a-z][a-z0-9+.-]*:|^\/\//i.test(href)) return href;
	const at = href.indexOf('#');
	const path = at < 0 ? href : href.slice(0, at);
	const hash = at < 0 ? '' : href.slice(at);
	const target = resolvePath(source, path);
	return (routes[target] ?? repoFile(target)) + hash;
}

function plainText(html: string): string {
	return html
		.replace(/<[^>]+>/g, '')
		.replace(/&lt;/g, '<')
		.replace(/&gt;/g, '>')
		.replace(/&quot;/g, '"')
		.replace(/&#39;/g, "'")
		.replace(/&amp;/g, '&');
}

const attr = (value: string) => escapeHtml(value).replace(/"/g, '&quot;');

/**
 * Renders a Markdown file of the repository. The first `#` heading becomes the
 * title; `##` and `###` headings make the table of contents.
 */
export function renderMarkdown(
	markdown: string,
	source: string,
	routes: Record<string, string>
): RenderedDoc {
	const seen = new Map<string, number>();
	const toc: TocEntry[] = [];

	const marked = new Marked({
		gfm: true,
		renderer: {
			heading({ tokens, depth }) {
				const inner = this.parser.parseInline(tokens);
				const text = plainText(inner);
				let id = slugify(text);
				const count = seen.get(id) ?? 0;
				seen.set(id, count + 1);
				if (count) id = `${id}-${count}`;
				if (depth === 2 || depth === 3) toc.push({ depth, text, id });
				return `<h${depth} id="${id}"><a class="anchor" href="#${id}">${inner}</a></h${depth}>\n`;
			},
			code({ text, lang }) {
				const language = (lang ?? '').split(/\s/)[0];
				const label = language && language !== 'text' ? ` data-lang="${attr(language)}"` : '';
				return `<pre class="code"${label}><code>${highlight(text, language)}</code></pre>\n`;
			},
			link({ href, title, tokens }) {
				const inner = this.parser.parseInline(tokens);
				const url = rewriteHref(href, source, routes);
				const titled = title ? ` title="${attr(title)}"` : '';
				const external = /^https?:/.test(url) ? ' rel="external noopener"' : '';
				return `<a href="${attr(url)}"${titled}${external}>${inner}</a>`;
			}
		}
	});

	const tokens = marked.lexer(markdown);
	const h1 = tokens.findIndex((t) => t.type === 'heading' && t.depth === 1);
	const title = h1 < 0 ? '' : (tokens.splice(h1, 1)[0] as Tokens.Heading).text;
	const html = marked
		.parser(tokens)
		.replace(/<table>/g, '<div class="table-wrap"><table>')
		.replace(/<\/table>/g, '</table></div>');

	return { title, html, toc };
}
