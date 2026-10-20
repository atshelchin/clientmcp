import { expect, test } from '@playwright/test';

test('home page explains the product and links to the docs', async ({ page }) => {
	await page.goto('/');
	await expect(page.locator('h1')).toContainText('UI for humans');
	await page.getByRole('link', { name: 'Read the spec', exact: true }).click();
	await expect(page).toHaveURL(/\/docs\/spec$/);
	await expect(page.locator('h1')).toHaveText('ClientMCP Protocol');
});

test('the demo shows an agent change in the app', async ({ page }) => {
	await page.goto('/');
	const app = page.getByRole('region', { name: 'Your application' });
	await expect(app.getByRole('checkbox', { name: 'Oat milk' })).toBeVisible({ timeout: 10_000 });
	await expect(app.getByText('agent', { exact: true })).toBeVisible();
});

test('links between rendered documents stay on the site', async ({ page }) => {
	await page.goto('/docs');
	await page.locator('.prose a[href="/docs/spec#2-tokens"]').first().click();
	await expect(page).toHaveURL(/\/docs\/spec#2-tokens$/);
	// GitHub-style ids can start with a digit, which a CSS #id selector cannot.
	await expect(page.locator('[id="2-tokens"]')).toBeInViewport();
});

test('unknown pages are 404s', async ({ page }) => {
	const response = await page.goto('/docs/nope');
	expect(response?.status()).toBe(404);
});
