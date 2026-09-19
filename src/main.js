import App from './App.svelte';
import { mount } from 'svelte';

import { initContextMenuSuppression } from './lib/utils/contextmenu.ts';

// Suppress the default browser/webview context menu (Reload, Inspect Element, etc.)
initContextMenuSuppression(window);

const app = mount(App, { target: document.getElementById('app') });

export default app;
