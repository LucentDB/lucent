const STORAGE_KEY = 'lucent-theme';

function createTheme() {
  let current = $state('light');

  function init() {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (
      stored === 'dark' ||
      (!stored && window.matchMedia('(prefers-color-scheme: dark)').matches)
    ) {
      current = 'dark';
    } else {
      current = 'light';
    }
    apply();
  }

  function apply() {
    document.documentElement.classList.toggle('dark', current === 'dark');
  }

  function toggle() {
    current = current === 'light' ? 'dark' : 'light';
    localStorage.setItem(STORAGE_KEY, current);
    apply();
  }

  return {
    get current() {
      return current;
    },
    init,
    toggle,
  };
}

let instance;

export function getTheme() {
  if (!instance) instance = createTheme();
  return instance;
}
