import { render } from 'solid-js/web';
import { App } from './App';

const root = document.getElementById('root');

if (!(root instanceof HTMLElement)) {
  throw new Error('Renamewright root element is missing');
}

render(() => <App />, root);
