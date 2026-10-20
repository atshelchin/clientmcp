import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import http from 'highlight.js/lib/languages/http';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import rust from 'highlight.js/lib/languages/rust';
import typescript from 'highlight.js/lib/languages/typescript';
import yaml from 'highlight.js/lib/languages/yaml';

hljs.registerLanguage('bash', bash);
hljs.registerLanguage('http', http);
hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('json', json);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('yaml', yaml);

const aliases: Record<string, string> = {
	sh: 'bash',
	shell: 'bash',
	zsh: 'bash',
	js: 'javascript',
	ts: 'typescript',
	jsonc: 'json',
	yml: 'yaml',
	rs: 'rust'
};

export function escapeHtml(text: string): string {
	return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Highlighted HTML for `code`; unknown languages and `text` are escaped only. */
export function highlight(code: string, lang = ''): string {
	const name = aliases[lang] ?? lang;
	if (!hljs.getLanguage(name)) return escapeHtml(code);
	return hljs.highlight(code, { language: name, ignoreIllegals: true }).value;
}
