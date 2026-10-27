import type { Handle } from '@sveltejs/kit/hooks';

// Every dynamic page is private: never cached, never framed, and it sends a
// referrer only within the site (a page may carry a token in its body; none
// is ever in a URL). Not `no-referrer`: with it, browsers send a form's
// Origin as `null`, and SvelteKit refuses the post as cross-site.
export const handle: Handle = async ({ event, resolve }) => {
	const response = await resolve(event);
	if (
		!event.isDataRequest &&
		event.route.id &&
		!event.route.id.startsWith('/docs') &&
		event.route.id !== '/'
	) {
		response.headers.set('cache-control', 'no-store');
		response.headers.set('referrer-policy', 'same-origin');
		response.headers.set('x-frame-options', 'DENY');
		response.headers.set('content-security-policy', "frame-ancestors 'none'");
	}
	return response;
};
