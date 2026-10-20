import { highlight } from '#lib/server/highlight.ts';
import {
	authority,
	docker,
	idea,
	messages,
	quickstart,
	workers,
	type Snippet
} from '#lib/content/snippets.ts';
import type { PageServerLoad } from './$types';

const render = ({ code, lang }: Snippet) => ({ raw: code, html: highlight(code, lang) });

export const load: PageServerLoad = () => ({
	idea: render(idea),
	authority: render(authority),
	docker: render(docker),
	workers: render(workers),
	quickstart: quickstart.map((step) => ({ ...step, code: render(step.snippet) })),
	messages: messages.map((message) => ({ ...message, html: highlight(message.example, 'json') }))
});
