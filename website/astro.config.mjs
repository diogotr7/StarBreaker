// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	site: 'https://starbreaker.app',
	integrations: [
		starlight({
			title: 'StarBreaker',
			description: 'Star Citizen data extraction and 3D export toolkit',
			social: [
				{
					icon: 'github',
					label: 'GitHub',
					href: 'https://github.com/diogotr7/StarBreaker',
				},
			],
			editLink: {
				baseUrl: 'https://github.com/diogotr7/StarBreaker/edit/main/website/',
			},
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ label: 'Install', slug: 'getting-started/install' },
						{ label: 'Quick Start', slug: 'getting-started/quickstart' },
					],
				},
				{
					label: 'Wiki',
					autogenerate: { directory: 'wiki' },
				},
				{
					label: 'Reference',
					autogenerate: { directory: 'reference' },
				},
			],
			customCss: ['./src/styles/custom.css'],
		}),
	],
});
